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
    ISLAND_HOLE_OR_LAYER,
    ISLAND_POLICIES,
    KEEP_PATHS,
    OMIT,
    REPLACE_WITH_RECTANGLE,
    background_rectangle_path,
    classify_background,
    classify_background_islands,
)
from palette_trace.color.assignment import NEAREST_AVAILABLE, distribute_unclaimed
from palette_trace.color.claims import resolve_claim_indices
from palette_trace.color.histogram import (
    build_color_histogram,
    snap_to_source_colors,
    unique_source_colors,
)
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
from palette_trace.presets.preview_scaling import (
    scale_measurement,
    scale_profile,
    scale_value,
)
from palette_trace.presets.profiles import resolve_entry_profile, unsupported_settings
from palette_trace.settings import create_palette_entry
from palette_trace.tracing.normalization import scale_svg_path_data
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

    def __init__(
        self,
        source: DecodedImageSource,
        settings: dict,
        preview_scale: float = 1.0,
    ):
        """
        `preview_scale` below 1.0 traces a reduced copy of the bitmap (§17.4).

        Everything the run produces is still expressed in source pixels: the
        pixel-denominated settings are converted on the way in, and the geometry
        is converted back on the way out. A caller asking for a preview is
        choosing how much work to do, not what coordinate system to get back.

        The default of 1.0 is the delivered result, and it is not merely
        "scale by one" — `working_view` hands back this very object and both
        conversions are skipped, so the full-resolution path is untouched.
        """
        self.requested_source = source
        self.preview_scale = float(preview_scale)
        # The bitmap actually traced. Every stage below reads `self.source`, so
        # they neither know nor need to know that a preview is running.
        self.source = source.working_view(self.preview_scale)
        # What the reduction really came to. `working_view` rounds to whole
        # pixels and refuses to enlarge, so the requested factor is not
        # necessarily the one the thresholds must be converted by. Taken from
        # the area, which makes the area conversion exact and the linear one the
        # geometric mean of the two axes — they differ by a fraction of a pixel
        # in a couple of thousand, and a single isotropic factor is what a
        # radius, an offset and a path all want.
        source_pixels = source.intrinsic_width * source.intrinsic_height
        self.effective_scale = (
            (self.source.intrinsic_width * self.source.intrinsic_height
             / float(source_pixels)) ** 0.5
            if source_pixels else 1.0
        )
        self.settings = settings
        self.backend_registry = BackendRegistry()

    @property
    def is_preview(self) -> bool:
        """True when the traced bitmap is not the source bitmap."""
        return self.source is not self.requested_source

    @property
    def _to_source_units(self) -> float:
        """The factor that takes traced geometry back to source pixels."""
        return 1.0 if not self.is_preview else 1.0 / self.effective_scale

    def _preview_pixels(self, measurement: dict) -> float:
        """A `{value, unit}` geometry measurement in the pixels being traced."""
        return _to_source_pixels(scale_measurement(measurement, self.effective_scale))

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
        claim_winners, claim_stats = resolve_claim_indices(self.source.oklch, pinned_entries)

        opaque = self.source.alpha >= alpha_threshold
        unclaimed_mask = (claim_winners < 0) & opaque

        # -- Stage 6: automatic quantization -------------------------------- #
        if auto_entries and np.any(unclaimed_mask):
            histogram = build_color_histogram(self.source.srgb, unclaimed_mask)
            quantized = run_deterministic_quantization(histogram, len(auto_entries))

            # §34.5: a swatch has to be a colour the picture contains. A K-Means
            # centroid is an average, so it is routinely a colour that appears
            # nowhere — snapping each one back onto the nearest real pixel
            # colour keeps the clustering and drops the invented value.
            rgb255, counts, oklab = unique_source_colors(self.source.srgb, unclaimed_mask)
            snapped = snap_to_source_colors(
                [q["oklab"] for q in quantized], rgb255, counts, oklab
            )

            for index, entry in enumerate(auto_entries):
                if index < len(quantized):
                    hex_value = snapped[index] if index < len(snapped) else quantized[index]["hex"]
                    entry["output"] = {
                        "mode": "automatic_centroid",
                        "color": {"srgb": hex_value},
                    }

        # -- Stage 7: label map and background classification ---------------- #
        layer_order = self._layer_order(entries)
        entry_ids = [e["id"] for e in enabled_entries]

        label_map = LabelMap(
            self.source.intrinsic_height, self.source.intrinsic_width, entry_ids
        )
        label_map.set_claims_from_indices(
            claim_winners, [e["id"] for e in pinned_entries]
        )

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
        islands = None
        foreground_entry = None

        if background_entry_id and background_entry_id in entry_ids:
            matching = label_map.get_binary_mask(background_entry_id)
            background_mask = classify_background(
                matching,
                self.source.alpha,
                self.settings["palette"].get("backgroundMatching", ALL_MATCHING),
                alpha_threshold,
            )
            # Pixels that matched but are not background under the selected mode
            # — the white of an eye when the backdrop is white — are foreground.
            # They leave the backdrop first, so nothing counts them twice.
            island_mask = matching & ~background_mask
            if np.any(island_mask):
                label_map.data[island_mask] = 0
                islands = classify_background_islands(
                    island_mask, label_map.data, self._island_policy()
                )
                if np.any(islands.layer_mask):
                    foreground_entry = self._foreground_entry(
                        next(e for e in enabled_entries if e["id"] == background_entry_id)
                    )
                if islands.summary():
                    warnings.append({
                        "code": "background_in_foreground",
                        "entryId": background_entry_id,
                        "message": islands.summary(),
                    })

        # -- Stage 8: label-aware cleanup ------------------------------------ #
        global_profile = self.settings.get("globalTraceProfile", {})
        subject_silhouette = opaque
        cleaned_masks = {}
        resolved_profiles = {}

        # The foreground layer is a consequence of the picture, not a palette
        # entry the user manages, so it is traced like any other scan but never
        # written back into the settings document.
        traced_entries = list(enabled_entries)
        if foreground_entry is not None:
            traced_entries.append(foreground_entry)
            layer_order = layer_order + [foreground_entry["id"]]

        for entry in traced_entries:
            entry_id = entry["id"]
            # §17.4: every threshold below is denominated in pixels, and the
            # mask being cleaned is a preview's pixels when a preview is
            # running. Converting here rather than at each call site is what
            # stops a 16-pixel-area threshold being applied as 16 preview
            # pixels — which would delete detail the delivered file keeps.
            profile = scale_profile(
                resolve_entry_profile(entry, global_profile), self.effective_scale
            )
            resolved_profiles[entry_id] = profile
            mask_cfg = profile.get("mask", {})

            raw_mask = label_map.get_binary_mask(entry_id)
            if entry_id == background_entry_id and background_mask is not None:
                raw_mask = background_mask
            elif foreground_entry is not None and entry_id == foreground_entry["id"]:
                raw_mask = islands.layer_mask

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

        # §17.4 again: underlap and trapping are linear distances applied to the
        # mask. They carry their unit in a field rather than in their name, so
        # `scale_profile` cannot see them and they are converted here — a trap
        # left at its source-pixel width would be twice as wide at half scale.
        if policy == STACKED:
            underlap_px = self._preview_pixels(geometry_cfg.get("underlap"))
            for entry_id, mask in cleaned_masks.items():
                cleaned_masks[entry_id] = apply_underlap_to_mask(
                    mask, subject_silhouette, underlap_px, preserve_silhouette
                )
        elif policy == STACKED_TRAPPED:
            trapping_px = self._preview_pixels(geometry_cfg.get("trapping"))
            for entry_id, mask in cleaned_masks.items():
                cleaned_masks[entry_id] = apply_trapping_to_mask(
                    mask, subject_silhouette, trapping_px, preserve_silhouette
                )
        elif policy in EXCLUSIVE_POLICIES:
            cleaned_masks = enforce_exclusive_ownership(cleaned_masks, layer_order)
            if policy == SEPARATE_OPERATIONS:
                minimum_area = global_profile.get("mask", {}).get("minimumRegionAreaPx2", 4)
                warnings.extend(validate_operation_masks(
                    cleaned_masks,
                    int(scale_value(minimum_area, self.effective_scale, 2)),
                ))

        # A hole is subtracted last, after geometry: an underlap or a trap grows
        # every shape outwards, and would close the very hole the picture asked
        # for if it were punched any earlier.
        if islands is not None:
            for scan_label, hole_mask in islands.holes.items():
                owner_id = label_map.label_to_id.get(scan_label)
                if owner_id in cleaned_masks:
                    cleaned_masks[owner_id] = cleaned_masks[owner_id] & ~hole_mask

        # -- Stage 10: vector tracing ---------------------------------------- #
        backend = self.backend_registry.get_backend(
            self.settings.get("backend", {}).get("preferredBackendId", "auto")
        )
        capabilities = backend.capabilities()
        scan_results = []
        total_pixels = float(self.source.intrinsic_width * self.source.intrinsic_height) or 1.0
        border_share = self._border_share(label_map)

        for index, entry in enumerate(traced_entries):
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
                # True for a scan the pipeline invented rather than one the user
                # can edit, so the interface can show it without offering
                # controls that would have nothing to write to.
                "derived": bool(entry.get("derived")),
                # Share of the picture this scan ends up owning, after cleanup
                # and geometry. `claims_stats` covers pinned entries only, so
                # without this the interface has nothing truthful to report for
                # an automatic colour. Reported only; it affects no geometry.
                "coveragePercent": float(np.count_nonzero(mask)) * 100.0 / total_pixels,
                # Share of the *frame* it owns. What sits around the edge of a
                # picture is almost always its backdrop, so this is what lets
                # the interface offer a one-tap backdrop instead of a menu.
                "borderSharePercent": border_share.get(entry_id, 0.0),
                "pathDatas": [],
                "fillRule": "evenodd",
                "warnings": [],
            }

            if is_background and background_output == REPLACE_WITH_RECTANGLE:
                # §16.2: the rectangle uses the complete intrinsic source bounds,
                # not the traced extent of the background. It is written in
                # source pixels directly rather than in the preview's and scaled
                # back, because §16.2 says *the* bounds and a round trip through
                # a reduced copy would land a fraction of a pixel away from them.
                scan["pathDatas"] = [background_rectangle_path(
                    self.requested_source.intrinsic_width,
                    self.requested_source.intrinsic_height,
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

            # Back into source pixels. The backend traced whatever bitmap this
            # run was given, so a preview's geometry is in preview coordinates —
            # and every consumer downstream, from the preview panel to the SVG
            # writer to the Inkscape commit, is written against the source's.
            # Converting once here is what lets them stay that way.
            scan["pathDatas"] = [
                scale_svg_path_data(path, self._to_source_units)
                for path in result.svg_path_data
            ]
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

    def _island_policy(self) -> str:
        """What to do with backdrop-coloured patches in the foreground (§16.1)."""
        policy = self.settings["palette"].get(
            "backgroundForegroundPolicy", ISLAND_HOLE_OR_LAYER
        )
        return policy if policy in ISLAND_POLICIES else ISLAND_HOLE_OR_LAYER

    @staticmethod
    def _foreground_entry(background_entry: dict) -> dict:
        """
        The extra layer for backdrop-coloured shapes a hole cannot express.

        It borrows the backdrop's colour and cleanup profile — it is the same
        paint, just in front — but it is marked derived so nothing tries to
        store it back into the palette.
        """
        entry = dict(background_entry)
        entry["id"] = f"{background_entry['id']}::foreground"
        entry["name"] = f"{background_entry.get('name', 'Backdrop')} in front"
        entry["role"] = "primary_fill"
        entry["derived"] = True
        return entry

    @staticmethod
    def _border_share(label_map: LabelMap) -> dict:
        """Percentage of the frame's outermost ring each entry owns."""
        data = label_map.data
        if data.size == 0:
            return {}

        border = np.concatenate([
            data[0, :].ravel(), data[-1, :].ravel(),
            data[:, 0].ravel(), data[:, -1].ravel(),
        ])
        counts = np.bincount(border, minlength=len(label_map.entry_ids) + 1)
        total = float(border.size) or 1.0
        return {
            entry_id: float(counts[label]) * 100.0 / total
            for entry_id, label in label_map.id_to_label.items()
            if label < counts.size
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
