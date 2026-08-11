PYTHON := .venv/bin/python

.PHONY: test test-unit test-conformance conformance phase0 verify-env install-dev web \
	engine-test engine-lint engine-wasm engine-deny engine-trace engine-evaluation-corpus

test:
	$(PYTHON) -m pytest

test-unit:
	$(PYTHON) -m pytest tests/unit/ -v --tb=short

test-conformance:
	$(PYTHON) -m pytest tests/conformance/ -v --tb=short

# Human-readable conformance report across every discovered backend.
conformance:
	$(PYTHON) -m palette_trace.tracing.conformance.runner

# Phase 0 gate (SPEC §36). Executes the suites rather than checking for files.
phase0:
	$(PYTHON) scripts/check_phase0.py

# Standalone web application (SPEC §9.4). Pass an image: make web IMAGE=path/to.png
web:
	$(PYTHON) -m palette_trace.standalone $(IMAGE)

verify-env:
	$(PYTHON) -c "import sys; print('Python:', sys.executable)"
	$(PYTHON) -c "import pytest; print('pytest:', pytest.__version__)"
	$(PYTHON) -c "import inkex; print('inkex:', inkex.__file__)"
	$(PYTHON) -c "from palette_trace.tracing.registry import BackendRegistry as R; print('backends:', [b['id'] for b in R().list_available_backends()])"

install-dev:
	$(PYTHON) -m pip install -e ".[backends,test]"

# --- Palette Tracer Engine (engine/, Rust, MIT) ----------------------------
# engine/SPEC.md is authoritative for these; engine/AGENTS.md has the rules.

engine-test:
	cd engine && cargo test --workspace

engine-lint:
	cd engine && cargo fmt --check && \
		cargo clippy --workspace --all-targets -- -D warnings

# PTE-ARCH-003: every baseline algorithm compiles for wasm32 with no OS stubs.
engine-wasm:
	cd engine && cargo check --workspace --target wasm32-unknown-unknown

# PTE-LIC-005: licence and advisory checks are release-blocking, so they need a
# command that runs them. Requires `cargo install cargo-deny --locked`.
engine-deny:
	cd engine && cargo deny check

# §25.2 synthetic reference corpus. Regenerated from analytic descriptions
# rather than committed as rasters (PTE-TEST-004).
ENGINE_FIXTURES ?= /tmp/pte-fixtures
engine-fixtures:
	python3 engine/tools/make_fixtures.py $(ENGINE_FIXTURES)

# PTE-NO-049: the same engine, compiled for wasm32 and run by a JavaScript
# runtime, must produce the same semantic digest as the native build.
# Requires Node 18+ and `rustup target add wasm32-wasip1`.
engine-parity: engine-fixtures
	cd engine && cargo build --release -p palette-tracer-cli
	cd engine && cargo build --release -p palette-tracer-cli --target wasm32-wasip1
	node --experimental-wasi-unstable-preview1 \
		engine/tools/wasm-parity.mjs $$(find $(ENGINE_FIXTURES) -name '*.pam' | sort)

# Trace the whole corpus and print the §26.7 complexity census.
engine-corpus: engine-fixtures
	cd engine && cargo build --release -p palette-tracer-cli
	@python3 engine/tools/corpus_report.py $(ENGINE_FIXTURES) \
		engine/target/release/pte

# PTE-TEST-003/PTE-NO-048: licensed clean-room evaluation inputs with fixed
# train/development/holdout splits. This does not alter the strict synthetic
# conformance corpus or calibrate any threshold.
engine-evaluation-corpus:
	python3 engine/tools/validate_evaluation_corpus.py
	python3 -m unittest engine/tools/test_evaluation_corpus.py

# End-to-end on the repository's own sample. `pte` reads Netpbm, not PNG:
# the core takes decoded pixels (PTE-ARCH-001) and no codec adapter is built.
engine-trace:
	python3 engine/tools/png_to_ppm.py examples/sample.png /tmp/pte-sample.pam
	cd engine && cargo run -q -p palette-tracer-cli -- trace \
		--profile flat-illustration --report /tmp/pte-sample-report.json \
		/tmp/pte-sample.pam /tmp/pte-sample.svg
