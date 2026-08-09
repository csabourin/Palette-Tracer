#!/usr/bin/env bash
# Replit entry point — the only way this project starts here.
#
# Both the `run` key and the "Palette Trace Web" workflow call this script, so
# there is one entry point on one port. There used to be two, on two ports,
# doing two different things, and nothing said which one the webview was
# looking at.
#
# No image is passed. Since §9.4.2 the interface can load a bitmap chosen in
# the browser, which is the only thing that makes sense on a hosted webview: it
# has no reach into the user's files, and tracing a bundled sample would show
# them a picture that is not theirs. Nothing is written to disk in this mode —
# results leave as downloads (§9.4.3, §9.4.4).
#
# `--host`/`--port` are deliberately not passed: standalone.py detects $PORT
# (set in .replit) and binds 0.0.0.0 in that case, while still defaulting to
# the strict 127.0.0.1-only bind required by SPEC §9.1 for every invocation
# that is not on a container platform.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 -m pip --version >/dev/null 2>&1 || python3 -m ensurepip --upgrade --quiet
python3 -m pip install --quiet --disable-pip-version-check -e .

exec python3 -m palette_trace.standalone --no-browser
