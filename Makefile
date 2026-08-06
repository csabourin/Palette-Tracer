PYTHON := .venv/bin/python

.PHONY: test test-unit verify-env install-dev

test:
	$(PYTHON) -m pytest

test-unit:
	$(PYTHON) -m pytest tests/unit/ -v --tb=short

verify-env:
	$(PYTHON) -c "import sys; print('Python:', sys.executable)"
	$(PYTHON) -c "import pytest; print('pytest:', pytest.__version__)"
	$(PYTHON) -c "import inkex; print('inkex:', inkex.__file__)"

install-dev:
	$(PYTHON) -m pip install -e ".[backends,test]"
