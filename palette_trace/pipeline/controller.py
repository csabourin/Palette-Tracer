"""
Pipeline Controller coordinating full end-to-end tracing operations.
"""

import uuid
import numpy as np
from palette_trace.document.image_source import DecodedImageSource
from palette_trace.color.claims import resolve_claimed_pixels
from palette_trace.color.histogram import build_color_histogram
from palette_trace.color.quantizer import run_deterministic_quantization
from palette_trace.color.background import extract_edge_connected_mask
from palette_trace.masks.label_map import LabelMap
from palette_trace.masks.components import remove_small_speckles, fill_small_holes
from palette_trace.masks.morphology import apply_mask_offset
from palette_trace.masks.geometry_policy import apply_underlap_to_mask, apply_trapping_to_mask
from palette_trace.tracing.protocol import TraceRequest
from palette_trace.tracing.registry import BackendRegistry

class PipelineController:
    """Manages full tracing pipeline execution."""

    def __init__(self, source: DecodedImageSource, settings: dict):
        self.source = source
        self.settings = settings
        self.backend_registry = BackendRegistry()

    def run_pipeline(self) -> dict:
        """
        Executes pipeline stages and returns dictionary containing:
        - palette_entries: updated list of palette entries (automatic colors populated)
        - claims_stats: claimed percentages per pinned entry
        - scan_results: list of traced scan results ({entryId, name, color, role, pathDatas, fillRule})
        """
        palette_cfg = self.settings["palette"]
        entries = list(palette_cfg["entries"])
        scan_count = self.settings["scanCount"]
        geometry_cfg = self.settings["geometry"]

        # Ensure correct entry count
        if len(entries) != scan_count:
            # Adjust entry count
            while len(entries) < scan_count:
                eid = str(uuid.uuid4())
                entries.append({
                    "id": eid,
                    "name": f"Scan {len(entries)+1}",
                    "enabled": True,
                    "kind": "automatic",
                    "sourceAnchor": None,
                    "output": { "mode": "automatic_centroid", "color": None },
                    "assignment": { "mode": "automatic", "overallReach": 25, "channels": { "mode": "linked" } },
                    "role": "primary_fill",
                    "traceProfile": { "mode": "inherit" },
                })
            entries = entries[:scan_count]

        # 1. Separate pinned and automatic entries
        pinned_entries = [e for e in entries if e.get("kind") == "pinned" and e.get("enabled", True)]
        auto_entries = [e for e in entries if e.get("kind") == "automatic" and e.get("enabled", True)]

        # 2. Evaluate pinned pixel claims
        claims_grid, claim_stats = resolve_claimed_pixels(self.source.oklch, pinned_entries)

        # 3. Create unclaimed mask
        unclaimed_mask = (claims_grid == None)

        # Exclude fully transparent pixels
        unclaimed_mask &= (self.source.alpha >= 0.05)

        # 4. Automatic quantization on unclaimed pixels
        if auto_entries and np.any(unclaimed_mask):
            hist = build_color_histogram(self.source.srgb, unclaimed_mask)
            quant_results = run_deterministic_quantization(hist, len(auto_entries))

            for idx, auto_e in enumerate(auto_entries):
                if idx < len(quant_results):
                    q = quant_results[idx]
                    auto_e["output"] = {
                        "mode": "automatic_centroid",
                        "color": { "srgb": q["hex"] },
                    }

        # 5. Build Label Map
        enabled_entries = [e for e in entries if e.get("enabled", True)]
        entry_ids = [e["id"] for e in enabled_entries]

        label_map = LabelMap(self.source.intrinsic_height, self.source.intrinsic_width, entry_ids)
        label_map.set_claims(claims_grid)

        # Assign remaining unclaimed pixels to nearest automatic/pinned palette entry
        if np.any(unclaimed_mask):
            # Assign remaining pixels to first automatic or pinned entry
            default_entry_id = auto_entries[0]["id"] if auto_entries else (pinned_entries[0]["id"] if pinned_entries else None)
            if default_entry_id:
                label_map.set_label_for_mask(unclaimed_mask, default_entry_id)

        # 6. Trace each scan mask
        preferred_backend = self.settings.get("backend", {}).get("preferredBackendId", "auto")
        backend = self.backend_registry.get_backend(preferred_backend)

        subject_silhouette = self.source.alpha >= 0.05
        underlap_val = geometry_cfg.get("underlap", {}).get("value", 0.5)

        scan_results = []
        global_profile = self.settings.get("globalTraceProfile", {})

        for idx, entry in enumerate(enabled_entries):
            eid = entry["id"]
            name = entry.get("name", f"Scan {idx+1}")
            role = entry.get("role", "primary_fill")

            # Determine output hex color
            out_cfg = entry.get("output", {})
            hex_color = "#000000"
            if out_cfg.get("mode") == "exact" and out_cfg.get("color"):
                hex_color = out_cfg["color"].get("srgb", "#000000")
            elif out_cfg.get("color"):
                hex_color = out_cfg["color"].get("srgb", "#000000")
            elif entry.get("sourceAnchor", {}):
                hex_color = entry["sourceAnchor"].get("srgb", "#000000")

            # Extract raw binary mask for this scan
            raw_mask = label_map.get_binary_mask(eid)
            if not np.any(raw_mask):
                scan_results.append({
                    "entryId": eid,
                    "name": name,
                    "color": hex_color,
                    "role": role,
                    "pathDatas": [],
                    "fillRule": "evenodd",
                })
                continue

            # Mask cleanup
            mask_cfg = global_profile.get("mask", {})
            cleaned_mask = remove_small_speckles(raw_mask, mask_cfg.get("minimumRegionAreaPx2", 4))
            cleaned_mask = fill_small_holes(cleaned_mask, mask_cfg.get("fillHolesAreaPx2", 4))
            cleaned_mask = apply_mask_offset(cleaned_mask, mask_cfg.get("offsetPx", 0))

            # Geometry underlap / trapping
            policy = geometry_cfg.get("policy", "stacked")
            if policy == "stacked":
                cleaned_mask = apply_underlap_to_mask(cleaned_mask, subject_silhouette, underlap_val)
            elif policy == "stacked_trapped":
                trap_val = geometry_cfg.get("trapping", {}).get("value", 0.25)
                cleaned_mask = apply_trapping_to_mask(cleaned_mask, subject_silhouette, trap_val)

            # Trace mask using backend
            req = TraceRequest(
                width=self.source.intrinsic_width,
                height=self.source.intrinsic_height,
                packed_binary_mask=cleaned_mask.astype(np.uint8).tobytes(),
                profile=global_profile,
            )

            res = backend.trace_mask(req)

            scan_results.append({
                "entryId": eid,
                "name": name,
                "color": hex_color,
                "role": role,
                "pathDatas": list(res.svg_path_data),
                "fillRule": res.fill_rule,
            })

        return {
            "palette_entries": entries,
            "claims_stats": claim_stats,
            "scan_results": scan_results,
        }
