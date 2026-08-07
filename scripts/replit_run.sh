#!/usr/bin/env bash
# Replit "Run" entry point.
#
# Palette Trace is a CLI tool over a local image path (`palette-trace-web
# image.png`), not a request-driven web service — there is no server to boot
# without an image to trace. This traces the bundled sample image so the Run
# button produces a working preview in Replit's webview, using the same
# `palette_trace.standalone` entry point a real user would run locally.
#
# `--host`/`--port` are not passed explicitly: standalone.py detects $PORT
# (set in .replit) and binds 0.0.0.0 automatically in that case, while still
# defaulting to the strict 127.0.0.1-only bind required by SPEC §9.1 when
# $PORT is absent, i.e. every non-Replit invocation.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 -m pip --version >/dev/null 2>&1 || python3 -m ensurepip --upgrade --quiet
python3 -m pip install --quiet --disable-pip-version-check -e .

exec python3 -m palette_trace.standalone examples/sample.png \
  --no-browser --force --output /tmp/palette-trace-replit-demo.svg
