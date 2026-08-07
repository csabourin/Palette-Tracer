PYTHON := .venv/bin/python

.PHONY: test test-unit test-conformance conformance phase0 verify-env install-dev web

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
