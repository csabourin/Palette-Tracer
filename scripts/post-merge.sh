#!/usr/bin/env bash
# Post-merge setup — runs automatically after every task merge.
# Must be idempotent, non-interactive, and fast.
set -euo pipefail

# Re-install the editable package so any new Python files or dependencies
# added by the merged task are immediately importable.
python3 -m pip install --quiet --disable-pip-version-check -e .
