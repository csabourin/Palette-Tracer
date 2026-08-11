# Clean-room evaluation corpus

This corpus adds the licensed common input set required by `PTE-TEST-003` and
the fixed train/development/holdout split required by `PTE-NO-048`. It is
separate from the strict synthetic conformance corpus produced by
`tools/make_fixtures.py`.

The 18 inputs have two evidence classes:

- **13 analytic/adversarial images** generated exactly by
  `tools/make_evaluation_corpus.py`. Their pixel properties are testable and
  regeneration must be byte-identical.
- **5 generated naturalistic images** used only for blind evaluation. Their
  fixed files and prompts are committed, but they are not mathematical truth
  and carry no pass/fail vectorization golden.

All assets are first-party clean-room work created for Palette Tracer and
redistributed under the engine's MIT licence. The test cards that inspired the
coverage categories are not part of the repository. No trademarked logo,
existing character, or external photograph is included.

Run:

```bash
python3 engine/tools/validate_evaluation_corpus.py
python3 -m unittest engine/tools/test_evaluation_corpus.py
```

The first command checks manifest completeness, fixed digests, container
semantics, split balance, regeneration, and the declared measurable
properties. The second includes negative controls showing that single-pixel,
palette, and alpha/provenance failures are detected.

This PR deliberately does not calibrate tracing thresholds or establish
goldens. A later benchmark/scorer slice must run every tracer on the same
raster inputs without privileged access to analytic truth (`PTE-TEST-014`).
