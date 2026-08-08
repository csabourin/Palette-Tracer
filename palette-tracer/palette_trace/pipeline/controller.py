"""
Pipeline controller (SPEC §24).

Coordinates the stages that turn a decoded raster into per-scan vector geometry.
Stage numbering below refers to §24.

The controller is deliberately backend-neutral (§35): it resolves a backend
through `BackendRegistry` and never names one.
"""

import uuid

import numpy as np

from palette_trace.color.background import (
    ALL_MATCHING,
    KEEP_PATHS,
    OMIT,
    REPLACE_WITH_RECTANGLE,
    background_rectangle_path,
    classify_background,
)
from palette_trace.color.assignment import NEAREST_AVAILABLE, distribute_unclaimed
from palette_trace.color.claims import resolve_claimed_pixels
from palette_trace.color.histogram import build_color_histogram
from palette_trace.color.quantizer import run_deterministic_quantization
from palette_trace.image_source import DecodedImageSource
from palette_trace.masks.components import fill_small_holes, remove_small_speckles
from palette_trace.masks.geometry_policy import (
    EXCLUSIVE_POLICIES,
    SEPARATE_OPERATIONS,
    STACKED,
    STACKED_TRAPPED,
    apply_trapping_to_mask,
    apply_underlap_to_mask,
    enforce_exclusive_ownership,
    validate_operation_masks,
)
from palette_trace.masks.label_map import LabelMap
from palette_trace.masks.morphology import apply_mask_offset
from palette_trace.masks.thin_features import preserve_thin_features
from palette_trace.presets.profiles import resolve_entry_profile, unsupported_settings
from palette_trace.settings import create_palette_entry
from palette_trace.tracing.protocol import TraceRequest
from palette_trace.tracing.registry import BackendRegistry

#: Millimetres per source pixel is document-dependent; until the document DPI is
#: plumbed through, trapping expressed in millimetres is converted at 96 dpi,
#: which is the SVG user-unit default.
_MM_PER_INCH = 25.4
_DEFAULT_DPI = 96.0


def _to_source_pixels(measurement: dict) -> float:
    """Converts a {value, unit} measurement to source pixels."""
    value = float((measurement or {}).get("value", 0.0) or 0.0)
    unit = (measurement or {}).get("unit", "source_px")
    if unit == "mm":
        return value / _MM_PER_INCH * _DEFAULT_DPI
    return value


class PipelineController:
    """Runs the full tracing pipeline for one image and one settings document."""

    def __init__(self, source: DecodedImageSource, settings: dict):
        self.source = source
        self.settings = settings
        self.backend_registry = BackendRegistry()

    # -- Stage 4: palette initialization ------------------------------------ #

    def _normalized_entries(self) -> list:
        """Returns palette entries padded or trimmed to the configured scan count."""
        entries = [dict(e) for e in self.settings["palette"]["entries"]]
        scan_count = int(self.settings["scanCount"])

        while len(entries) < scan_count:
            entries.append(create_palette_entry(len(entries)))
        return entries[:scan_count]

    def _layer_order(self, entries: list) -> list:
        """Bottom-to-top entry ids, defaulting to palette order (§10.4)."""
        known = {e["id"] for e in entries}
        ordered = [eid for eid in self.settings["palette"].get("layerOrder", []) if eid in known]
        ordered += [e["id"] for e in entries if e["id"] not in ordered]
        return ordered

    # -- main ---------------------------------------------------------------- #

    def run_pipeline(self) -> dict:
        """
        Executes the pipeline and returns:

        * ``palette_entries`` — entries with automatic output colours resolved
        * ``claims_stats`` — percentage of the image claimed per pinned entry
        * ``scan_results`` — one dict per scan, ready for SVG assembly
        * ``warnings`` — destination and backend warnings (§18, §20.4, §30)
        * ``backend`` — id and version actually used, for provenance (§10.3)
        """
        entries = self._normalized_entries()
        geometry_cfg = self.settings.get("geometry", {})
        policy = geometry_cfg.get("policy", STACKED)
        alpha_threshold = float(self.settings.get("alpha", {}).get("ignoreBelow", 0.05))
        warnings = []

        enabled_entries = [e for e in entries if e.get("enabled", True)]
        pinned_entries = [e for e in enabled_entries if e.get("kind") == "pinned"]
        auto_entries = [e for e in enabled_entries if e.get("kind") != "pinned"]

        # -- Stage 5: pinned-colour claim ----------------------------------- #
        claims_grid, claim_stats = resolve_claimed_pixels(self.source.oklch, pinned_entries)

        opaque = self.source.alpha >= alpha_threshold
        unclaimed_mask = (claims_grid == None) & opaque      # noqa: E711 — object array

        # -- Stage 6: automatic quantization -------------------------------- #
        if auto_entries and np.any(unclaimed_mask):
            histogram = build_color_histogram(self.source.srgb, unclaimed_mask)
            quantized = run_deterministic_quantization(histogram, len(auto_entries))
            for index, entry in enumerate(auto_entries):
                if index < len(quantized):
                    entry["output"] = {
                        "mode": "automatic_centroid",
                        "color": {"srgb": quantized[index]["hex"]},
                    }

        # -- Stage 7: label map and background classification ---------------- #
        layer_order = self._layer_order(entries)
        entry_ids = [e["id"] for e in enabled_entries]

        label_map = LabelMap(
            self.source.intrinsic_height, self.source.intrinsic_width, entry_ids
        )
        label_map.set_claims(claims_grid)

        # §14.4: unclaimed pixels go to the automatic cluster they are nearest
        # to. Assigning them all to one entry would compute an automatic palette
        # and then throw it away.
        distributed = distribute_unclaimed(
            self.source.oklch,
            unclaimed_mask,
            auto_entries,
            pinned_entries,
            self.settings.get("colorProcessing", {}).get(
                "unmatchedPixelPolicy", NEAREST_AVAILABLE),
        )
        for entry_id, mask in distributed.items():
            label_map.set_label_for_mask(mask, entry_id)

        background_entry_id = self.settings["palette"].get("backgroundEntryId")
        background_output = self.settings.get("output", {}).get("backgroundOutput", KEEP_PATHS)
        background_mask = None

        if background_entry_id and background_entry_id in entry_ids:
            matching = label_map.get_binary_mask(background_entry_id)
            background_mask = classify_background(
                matching,
                self.source.alpha,
                self.settings["palette"].get("backgroundMatching", ALL_MATCHING),
                alpha_threshold,
            )
            # Pixels that matched but are not background under the selected mode
            # (an enclosed region of the same colour, say) revert to unclassified
            # rather than silently joining the backdrop.
            reverted = matching & ~background_mask
            if np.any(reverted):
                label_map.data[reverted] = 0

        # -- Stage 8: label-aware cleanup ------------------------------------ #
        global_profile = self.settings.get("globalTraceProfile", {})
        subject_silhouette = opaque
        cleaned_masks = {}
        resolved_profiles = {}

        for entry in enabled_entries:
            entry_id = entry["id"]
            profile = resolve_entry_profile(entry, global_profile)
            resolved_profiles[entry_id] = profile
            mask_cfg = profile.get("mask", {})

            raw_mask = label_map.get_binary_mask(entry_id)
            if entry_id == background_entry_id and background_mask is not None:
                raw_mask = background_mask

            cleaned = remove_small_speckles(raw_mask, mask_cfg.get("minimumRegionAreaPx2", 4))
            cleaned = fill_small_holes(cleaned, mask_cfg.get("fillHolesAreaPx2", 4))

            if mask_cfg.get("preserveThinFeatures", True):
                cleaned = preserve_thin_features(
                    raw_mask, cleaned, mask_cfg.get("minimumFeatureWidthPx", 1)
                )

            # §20.3: per-scan offsets are disabled under exclusive policies,
            # because an offset is exactly what breaks exclusive ownership.
            if policy not in EXCLUSIVE_POLICIES:
                cleaned = apply_mask_offset(cleaned, mask_cfg.get("offsetPx", 0))

            cleaned_masks[entry_id] = cleaned

        # -- Stage 9: destination geometry ----------------------------------- #
        preserve_silhouette = geometry_cfg.get("preserveOuterSilhouette", True)

        if policy == STACKED:
            underlap_px = _to_source_pixels(geometry_cfg.get("underlap"))
            for entry_id, mask in cleaned_masks.items():
                cleaned_masks[entry_id] = apply_underlap_to_mask(
                    mask, subject_silhouette, underlap_px, preserve_silhouette
                )
        elif policy == STACKED_TRAPPED:
            trapping_px = _to_source_pixels(geometry_cfg.get("trapping"))
            for entry_id, mask in cleaned_masks.items():
                cleaned_masks[entry_id] = apply_trapping_to_mask(
                    mask, subject_silhouette, trapping_px, preserve_silhouette
                )
        elif policy in EXCLUSIVE_POLICIES:
            cleaned_masks = enforce_exclusive_ownership(cleaned_masks, layer_order)
            if policy == SEPARATE_OPERATIONS:
                warnings.extend(validate_operation_masks(
                    cleaned_masks,
                    int(global_profile.get("mask", {}).get("minimumRegionAreaPx2", 4)),
                ))

        # -- Stage 10: vector tracing ---------------------------------------- #
        backend = self.backend_registry.get_backend(
            self.settings.get("backend", {}).get("preferredBackendId", "auto")
        )
        capabilities = backend.capabilities()
        scan_results = []
        total_pixels = float(self.source.intrinsic_width * self.source.intrinsic_height) or 1.0

        for index, entry in enumerate(enabled_entries):
            entry_id = entry["id"]
            profile = resolved_profiles[entry_id]
            mask = cleaned_masks[entry_id]
            is_background = entry_id == background_entry_id

            if is_background and background_output == OMIT:
                continue

            scan = {
                "entryId": entry_id,
                "name": entry.get("name", f"Scan {index + 1}"),
                "color": self._output_color(entry),
                "role": "background" if is_background else entry.get("role", "primary_fill"),
                "isBackground": is_background,
                # Share of the picture this scan ends up owning, after cleanup
                # and geometry. `claims_stats` covers pinned entries only, so
                # without this the interface has nothing truthful to report for
                # an automatic colour. Reported only; it affects no geometry.
                "coveragePercent": float(np.count_nonzero(mask)) * 100.0 / total_pixels,
                "pathDatas": [],
                "fillRule": "evenodd",
                "warnings": [],
            }

            if is_background and background_output == REPLACE_WITH_RECTANGLE:
                # §16.2: the rectangle uses the complete intrinsic source bounds,
                # not the traced extent of the background.
                scan["pathDatas"] = [background_rectangle_path(
                    self.source.intrinsic_width, self.source.intrinsic_height
                )]
                scan["fillRule"] = "nonzero"
                scan_results.append(scan)
                continue

            if not np.any(mask):
                scan_results.append(scan)
                continue

            # §18: a backend must map, emulate or *report* a setting — never
            # ignore it silently.
            unsupported = unsupported_settings(profile, capabilities.supported_canonical_settings)
            if unsupported:
                message = (
                    f"{capabilities.backend_id} does not support: {', '.join(unsupported)}"
                )
                scan["warnings"].append(message)
                warnings.append({
                    "code": "unsupported_trace_setting",
                    "entryId": entry_id,
                    "message": message,
                })

            result = backend.trace_mask(TraceRequest(
                width=self.source.intrinsic_width,
                height=self.source.intrinsic_height,
                packed_binary_mask=mask.astype(np.uint8).tobytes(),
                profile=profile,
            ))

            scan["pathDatas"] = list(result.svg_path_data)
            scan["fillRule"] = result.fill_rule
            scan["warnings"].extend(result.warnings)

            if profile.get("vector", {}).get("requireClosedPaths") and not all(
                path.rstrip().endswith(("Z", "z")) for path in result.svg_path_data
            ):
                message = "Destination requires closed paths but the backend produced an open path."
                scan["warnings"].append(message)
                warnings.append({
                    "code": "open_path",
                    "entryId": entry_id,
                    "message": message,
                })

            scan_results.append(scan)

        # Emit bottom to top so SVG document order matches layer order (§10.4).
        position = {eid: i for i, eid in enumerate(layer_order)}
        scan_results.sort(key=lambda s: position.get(s["entryId"], len(position)))

        return {
            "palette_entries": entries,
            "claims_stats": claim_stats,
            "scan_results": scan_results,
            "warnings": warnings,
            "backend": {
                "id": capabilities.backend_id,
                "version": capabilities.version,
            },
        }

    @staticmethod
    def _output_color(entry: dict) -> str:
        """
        Resolves an entry's output colour (§6.5).

        A picked colour is exact — it is the colour that comes out (§34.5), so
        the explicit output colour always wins over the source anchor.
        """
        output = entry.get("output") or {}
        color = output.get("color") or {}
        if color.get("srgb"):
            return color["srgb"]

        anchor = entry.get("sourceAnchor") or {}
        if anchor.get("srgb"):
            return anchor["srgb"]

        return "#000000"
