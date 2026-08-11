#!/usr/bin/env python3
"""Trace the synthetic corpus and print the §26.7 complexity census.

The numbers `docs/IMPLEMENTATION_STATUS.md` quotes come from here, so a reader
can reproduce them with one command rather than taking them on trust (§0.2).

Usage:
    python3 tools/corpus_report.py <fixture-directory> [<path-to-pte>]
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys


def warning_count(report: dict, code: str) -> int:
    """The leading integer of a warning's message, or zero if absent."""
    for warning in report.get("warnings", []):
        if warning.get("code") == code:
            match = re.match(r"(\d+)", warning.get("message", ""))
            return int(match.group(1)) if match else 0
    return 0


def main() -> int:
    if not 2 <= len(sys.argv) <= 3:
        print(__doc__, file=sys.stderr)
        return 2
    root = sys.argv[1]
    pte = sys.argv[2] if len(sys.argv) == 3 else "target/release/pte"

    manifest_path = os.path.join(root, "manifest.json")
    if not os.path.exists(manifest_path):
        print(f"no manifest at {manifest_path}; run make engine-fixtures", file=sys.stderr)
        return 1
    with open(manifest_path, encoding="utf-8") as handle:
        manifest = json.load(handle)

    header = (
        f"{'fixture':<34}{'faces':>6}{'edges':>6}{'lines':>6}"
        f"{'cubics':>7}{'arcs':>6}{'prims':>6}{'bytes':>8}{'minimal':>8}{'fallback':>9}"
    )
    print(header)
    print("-" * len(header))

    failures = 0
    for entry in manifest["fixtures"]:
        path = os.path.join(root, entry["path"])
        # A fixture declares the profile it is for; the first is authoritative.
        profile = entry["intendedProfiles"][0]
        result = subprocess.run(
            [pte, "inspect", "--profile", profile, path],
            capture_output=True,
        )
        if result.returncode != 0:
            failures += 1
            print(f"{entry['id']:<34}  FAILED ({result.returncode}) {result.stderr.decode()[:60]}")
            continue

        report = json.loads(result.stdout)
        representation = report["representation"]
        print(
            f"{entry['id']:<34}"
            f"{report['topology']['faces']:>6}"
            f"{report['topology']['sharedEdges']:>6}"
            f"{representation['lines']:>6}"
            f"{representation['cubics']:>7}"
            f"{representation['arcs']:>6}"
            f"{representation['primitives']:>6}"
            f"{representation['svgBytes']:>8}"
            f"{warning_count(report, 'geometry.chains_already_minimal'):>8}"
            f"{representation['polylineFallbacks']:>9}"
        )

    print()
    print(
        "prims    = §11.7 semantic primitives; `arcs` beside one is its opaque\n"
        "           neighbour retracing the same analytic circle\n"
        "minimal  = chains whose polyline was already the simplest faithful form\n"
        "fallback = chains that exhausted the fitting budget (PTE-GEO-005); a\n"
        "           non-zero column here is a search failure, `minimal` is not."
    )
    if failures:
        print(f"\n{failures} fixture(s) failed to trace", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
