# Palette Tracer Engine — implementation status

`engine/SPEC.md` is authoritative. This document records evidence; it does not
redefine requirements.

**Assessed at:** 2026-08-11, after §11 curve fitting.
**Measured with:** `cargo test --workspace` → **362 passed, 0 failed**;
`cargo clippy --workspace --all-targets -- -D warnings` → clean;
`cargo fmt --check` → clean;
`cargo check --workspace --target wasm32-unknown-unknown` → clean;
`cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`;
`make engine-parity` → **13 fixtures, native and wasm32 agree**.
Toolchain `rustc 1.97.1` on `x86_64-unknown-linux-gnu`.

## Status vocabulary

`engine/SPEC.md` §34.5 defines the labels this document uses. They are not the
root `docs/IMPLEMENTATION_STATUS.md` vocabulary, because a different
specification governs this subtree.

* **prototype** — demonstrates the idea; gates may be absent.
* **experimental** — integrated behind opt-in; metrics exist; compatibility not
  stable.
* **conforming** — passes the requirement's test matrix.
* **stable** — public API and configuration compatibility promised.
* **optimized** — conforming behaviour with measured improvement and no gate
  regression.
* **not implemented** — used here for requirements with no implementation. §0.2
  requires this to be said rather than left blank.

Nothing in this build is **stable**: no compatibility is promised yet.

---

## What this engine currently does

It converts a decoded raster to SVG through one exclusive raster partition and
one shared boundary graph, then fits that graph's chains to lines and cubic
Béziers. On the repository's own `examples/sample.png` (320×240), converted to
PAM with `tools/png_to_ppm.py`:

```
pte: 5 faces, 10 shared edges, 3353 bytes,
     digest pte-semantic-v1-blake3:87c7afc66d7bbd7b979eb8bc3a89fb42f283543160d9ef5d4381381b755418f4
```

with an automatic five-colour palette, one hole recovered, zero exposed seam
pixels, and 184 lines plus 68 cubics.

**Boundary evidence is still the pixel grid.** That is now the single most
important thing to know about this build. §11 fitting makes the geometry
*compact* — the sample above went from 4022 segments and 29115 bytes to 252
segments and 3353 bytes — but §10's coverage reconstruction is not implemented,
so the positions those curves are fitted *to* still come from pixel-cell
interfaces. Every boundary is reported as `crisp_grid` or `pixel_art_policy`,
and §31.2's subpixel gates remain unmeasured.

The corpus below is the honest picture of where that hurts.

---

## Phase status against §38

| Phase | Status | Evidence | Remaining |
|---|---|---|---|
| Phase 0 — evidence, licensing, contracts | Partly conforming | Workspace, licences, ADR-0001/2/3, typed config/IR/report/digest with round-trip tests, CLI and WASM adapter skeletons; a 12-fixture synthetic corpus with manifests (`tools/make_fixtures.py`); `cargo deny` wired to `make engine-deny` | No **real-world** corpus; no baseline benchmarks against VTracer, Potrace, AutoTrace or ImageTracerJS (PTE-BASE-001); memory and runtime budgets not calibrated (PTE-PERF-003); no blind SVG scorer (§39.1) |
| Phase 1 — colour and deterministic image foundation | Conforming | `palette-tracer-color`, 64 tests; OKLab reference vectors, hue wrap, alpha-zero invariance, exact-winner oracle, `u8`/`u16` transition | Plane lifetimes not measured with an allocator (PTE-PERF-002) |
| Phase 2 — segmentation and shared topology | Conforming | `palette-tracer-segment` 26 tests, `palette-tracer-topology` 35 tests; exhaustive 2×2/3×3 pattern oracle; reflection and rotation congruence | Tiling and tile reconciliation absent (PTE-SEG-018/019) |
| Phase 3 — subpixel boundaries and curve fitting | **Half** | §11 lines and cubics with bidirectional validation, multi-scale corners, DP segmentation: `palette-tracer-geometry`, 48 tests, `docs/notes/curve-fitting.md` | All of §10; §11.4 arcs; §11.7 primitives (PTE-GEO-010/011); §10.5 boundary optimisation, whose absence is what pins fitted split points to source samples |
| Phase 4 — logo, illustration, fabrication | Not implemented | — | §11.7, §12, §16 |
| Phase 5 — lines, lettering, coloring books | Partly | `coloring-book` emits each interface once (§13.6) | All of §13.1–§13.5 |
| Phase 6 — standard gradients | Not implemented | — | All of §14 |
| Phase 7 — hardening and 1.0 | Not started | — | Fuzzing, cross-renderer matrix, published budgets |

---

## Requirement tracking

Only requirements this build touches are listed. A requirement absent from the
table is not implemented; §39's "recommended first implementation issues" is
the order the remaining ones should be taken in.

| Requirement | Status | Implementation | Validation |
|---|---|---|---|
| PTE-GOAL-001/002/003 | conforming | The whole pipeline | `conformance_gates.rs`, 11 tests |
| PTE-GOAL-004 | partly | §11 fitting produces lines and cubics | §10 is absent, so the geometry is compact but its *evidence* is grid-aligned. `a_circle_is_reproduced_to_well_under_a_tenth_of_a_pixel` bounds the fitter's own error at 0.0072 px on exact samples; that is not the same claim as §31.2 |
| PTE-GOAL-005 | **not implemented** | — | §13 is absent |
| PTE-GOAL-006 | partly | 4 of 11 profiles | `unimplemented_profiles_are_refused_not_silently_downgraded` |
| PTE-GOAL-007 | partly | Library, CLI, host-free WASM adapter | No `wasm-bindgen` binding; no C ABI |
| PTE-GOAL-008 | conforming | `core::report` | `the_report_has_the_appendix_a_top_level_keys`, `the_report_names_what_is_not_implemented` |
| PTE-ARCH-001..003 | conforming | `core` depends on no sibling; no host API anywhere | `cargo check --target wasm32-unknown-unknown` |
| PTE-ARCH-006/007 | conforming | `VectorDocument`; the seven-category error taxonomy | `exit_codes_match_the_spec_table` |
| PTE-ARCH-011 | conforming | No third-party geometry crate at all: `kurbo` was declared for the §11 fitter and has been removed, because a library's root-solve iteration count is not part of its API contract and PTE-DET-004 needs the same *decision* on every target | `THIRD_PARTY_NOTICES.md`, `docs/notes/curve-fitting.md` |
| PTE-API-001/002/003 | conforming | `core::image` | 8 tests including padded rows and adversarial strides |
| PTE-API-005/006 | conforming | `TraceOutput`; `check_cancel` on a bounded stride | `cancellation_takes_effect_inside_a_long_loop` |
| PTE-API-007/008/009 | conforming | `Finite`, `deny_unknown_fields`, `Resolver` | `an_unknown_key_is_refused`, `provenance_distinguishes_user_from_profile` |
| PTE-API-010/011 | conforming | Identifiers from raster order; `f64` throughout | `labels_are_numbered_by_raster_position_not_allocation_order` |
| PTE-API-012 | conforming | Stable codes on every warning and error | `codes_are_category_prefixed_and_stable` |
| PTE-API-013/014/015 | conforming | `pte` stream discipline and flag precedence | `crates/palette-tracer-cli/tests/cli.rs`, 22 tests against the real binary. Writing it found a defect: `--report -` with an SVG also on stdout concatenated two documents, and is now refused |
| PTE-API-017..022 | partly | `TraceSession` with Appendix E.1 keys, `dispose` | `precision_reuses_the_caches_and_reach_does_not`; no JS binding exists |
| PTE-COLOR-001..004 | conforming | `color::spaces` | Published OKLab reference vectors; `ΔE_OK` convention declared |
| PTE-COLOR-005 | conforming | Reported as `assumed-srgb` | `the_report_has_the_appendix_a_top_level_keys` |
| PTE-COLOR-006/007 | **not implemented** | — | No ICC adapter |
| PTE-COLOR-008/009/010 | conforming | `color::alpha` | `alpha_zero_colour_randomisation_changes_nothing` (end to end) |
| PTE-COLOR-011..014 | conforming | `color::palette` | `hue_is_powerless_near_neutral`, `ties_are_broken_by_priority_then_id_not_array_order` |
| PTE-COLOR-015..017 | conforming | Running winner; exact direct-mapped cache | `the_cached_path_agrees_with_the_scalar_oracle`, `working_memory_does_not_scale_with_palette_size` |
| PTE-COLOR-018..022 | conforming | `color::auto` | `generation_is_deterministic_and_order_independent`, `centroids_are_cartesian_and_survive_the_hue_wrap` |
| PTE-COLOR-023 | conforming | Background is a face; only its paint changes | `a_transparent_face_is_omitted_without_disturbing_the_rest` |
| PTE-SEG-001..007 | conforming | `segment::edges`, `segment::partition` | `every_pixel_is_owned_by_exactly_one_region`, `equal_code_point_steps_are_not_equal_costs` |
| PTE-SEG-008 | conforming | Named: Felzenszwalb–Huttenlocher MSF predicate, **not** Cousty's watershed cut | Module documentation and `AlgorithmVersions` |
| PTE-SEG-009..014 | conforming | `segment::rag` | `adjacency_is_symmetric`, `merging_is_deterministic`, `distinct_pinned_identities_never_merge` |
| PTE-SEG-015/016 | conforming | Fringe pass before topology, with audit records | `an_antialias_fringe_is_absorbed_with_an_audit_record` |
| PTE-SEG-017 | **partly** | Elongation and width only | `protection_distinguishes_a_hairline_from_a_speck`. Geodesic length, repeated-pattern evidence, endpoint and junction evidence, and profile role are **not** implemented |
| PTE-SEG-018/019 | **not implemented** | — | No tiling |
| PTE-TOPO-001..007 | conforming | `topology::extract` | `twin_traversals_are_exact_reverses_of_one_chain`, `every_face_cycle_closes_exactly_once` |
| PTE-TOPO-008/009/010 | **partly** | §9.4 energy with data, continuity, mixture and connectivity terms | `reflection_reflects_the_decisions`. PTE-TOPO-009's asymptotic-decider data rule is **not** implemented; the data term is colour similarity. Of Kopf–Lischinski's heuristics only sparse-pixel is implemented |
| PTE-TOPO-011/012/013 | **not implemented** | — | Junctions sit where extraction put them; nothing moves them |
| PTE-TOPO-014 | conforming | Precision search refuses a collapse | `precision_never_collapses_two_distinct_points` |
| PTE-TOPO-015 | conforming | | `shared_mosaics_have_no_exposed_seam_pixels`, with the first-party rasteriser |
| PTE-TOPO-016/017/018 | partly | Modes are distinct and recorded in the layer role | Only `shared-mosaic` is produced; `stacked` and `separate-operations` are refused |
| PTE-AA-001..009 | **not implemented** | The §10.2/§10.4 mixture estimator exists in `color::mixture` and is used by fringe detection | Coverage-to-position inversion (§10.3), normal estimation, boundary optimisation: **absent** |
| PTE-GEO-001..009 | conforming | `palette-tracer-geometry`: preparation, multi-scale corners with hysteresis, line and cubic models, bidirectional error, DP segmentation | 48 tests. `a_forty_five_degree_staircase_has_no_corners` (PTE-GEO-002), `a_ballooning_cubic_is_rejected_though_it_passes_through_the_samples` (§11.5), `the_chord_bound_is_never_exceeded_by_the_real_curve` (PTE-GEO-007), `exhausting_the_candidate_budget_falls_back_to_the_polyline` (PTE-GEO-005) |
| PTE-GEO-010/011 | **not implemented** | — | §11.7 primitive recognition; a circle is cubics, not a circle |
| PTE-GEO-012/013/014 | conforming | One chain per shared edge, fitted once; endpoints are junctions by construction; the validator reruns after fitting | `fitting_changes_only_the_chains`, `fitting_never_moves_a_chain_endpoint`, `fitting_a_reversed_chain_gives_the_reversed_fit_exactly` |
| PTE-GEO-024/025 | conforming | `pixel-art` sets a zero tolerance, which means "the grid *is* the geometry" and skips fitting | `a_zero_tolerance_leaves_every_chain_untouched` |
| PTE-STROKE-010/011/012 | conforming | Interfaces emitted from shared edges | `no_coloring_book_interface_is_emitted_twice` |
| PTE-STROKE-001..009 | **not implemented** | — | No centrelines |
| PTE-GRAD-* , PTE-FAB-* | **not implemented** | Refused by name at configuration time | `a_stroke_or_gradient_request_is_refused_with_its_requirement` |
| PTE-SVG-001..005 | conforming | `svg::writer` | `output_is_a_standalone_svg_with_a_finite_viewport`, `a_hole_is_empty_under_nonzero_winding` |
| PTE-SVG-007/008/009 | conforming | Searched precision; one chain, two orientations | `precision_is_searched_not_fixed`, `shared_coordinates_are_byte_identical_after_reversal` |
| PTE-SVG-012/013 | conforming | Caller's bytes; alpha once | `a_palette_colour_survives_verbatim`, `alpha_is_applied_exactly_once` |
| PTE-SEC-001 | conforming | `escape_xml` | `a_hostile_label_is_escaped` |
| PTE-SEC-005/006/007 | conforming | `checked`, incremental limits | `an_adversarial_region_count_hits_the_limit_cleanly` |
| PTE-SEC-009 | conforming | `unsafe_code = "deny"` workspace-wide | The workspace lint table |
| PTE-DET-001..004 | conforming | Quantised keys, stable heaps, no unordered iteration in an output path | `determinism.rs`, 8 tests, plus `make engine-parity`: the same engine compiled for `wasm32-wasip1` and run under Node's V8 produces byte-identical semantic digests on all 13 fixtures. Browser-engine parity beyond V8 is still unproven |
| PTE-LIC-001..005 | conforming | ADR-0003, `deny.toml`, `THIRD_PARTY_NOTICES.md` | `make engine-deny` → `advisories ok, bans ok, licenses ok, sources ok`. Running it for the first time found the policy failing: `arrayref` is BSD-2-Clause only, and is now admitted with a review note |
| PTE-NO-042 | conforming | Unimplemented settings refused by name with the governing requirement, now including `geometry.allowArcs` | `unimplemented_modifiers_are_refused`, `an_unimplemented_profile_is_refused_not_downgraded` |
| PTE-TEST-003/004 | partly | 12 synthetic fixtures regenerated from analytic descriptions, with §25.2 manifests: `tools/make_fixtures.py`, `make engine-fixtures` | Synthetic only. No real-world corpus, and no multiple-resolution or rotation sweep yet |

---

## What the corpus measures

`make engine-corpus` regenerates the §25.2 synthetic corpus and traces it. The
census below is that command's output, and it is the most useful single view of
this build's strengths and weaknesses.

```
fixture                            faces edges lines cubics   bytes minimal fallback
topology/nested-rectangles             3     3    20      0     587       0        0
topology/donut                         3     3    86     30    1523       0        0
adversarial/checkerboard-4px           2   188  1024      0    7953       0        0
topology/one-pixel-bridge              3     3    36      0     683       1        0
curves/circle-subpixel-0               8    35   146     16    1917      25        0
curves/circle-subpixel-1               9    44   158     22    2167      19        0
curves/rounded-rectangle              14    49   184      4    2181      36        0
curves/star-acute-corners             47   160   556     12    6160      82        0
curves/shallow-staircase               2     3     8      0     449       0        0
alpha/transparent-arbitrary-rgb        2     2    12      0     485       0        0
color/near-neutral-bands               3     6    12      0     518       0        0
pixel-art/diagonals                    34    96  1984      0   13298       0        0
```

Three things to read out of it.

**The fitting search never failed.** The `fallback` column — chains that
exhausted the candidate budget — is zero everywhere. Every large number in the
`minimal` column is a chain whose polyline was *already* the simplest faithful
representation, which is the search succeeding, not failing.

**Antialiased input is where this build is weakest, and §10 is why.** A circle
should be two faces. It is eight or nine, because the antialiased fringe becomes
its own thin regions rather than being inverted to a subpixel boundary
position. The star is 47 faces for one star. Those extra faces are also the
source of most of the `minimal` count: a two-pixel fringe sliver has a short
jagged boundary with no simpler faithful form. This is the single clearest
argument for building §10 next, and it is a measurement rather than an opinion.

**Un-antialiased geometry fits well.** The nested rectangles are 20 lines and no
cubics; the staircase is 8 segments for one straight edge; the donut is 30
cubics for two circles. `pixel-art/diagonals` is deliberately unfitted (§15).

The `curves/shallow-staircase` figure of 8 is the limitation recorded as item 1
in `docs/notes/curve-fitting.md`: split points are source samples, so a
staircase cannot become one line at a tolerance below its own deviation from its
chord.

---

## Gaps, stated plainly

These are the things a reader would otherwise have to discover.

1. **No subpixel reconstruction.** The largest remaining gap, and the corpus
   above quantifies it. Every boundary is reported as `crisp_grid` or
   `pixel_art_policy`; `coverage_reconstructed` is always zero. §31.2's
   subpixel gates have **not been measured**, because the evidence they measure
   does not exist yet. The mixture estimator (§10.2, §10.4) is built and used by
   fringe detection; what is missing is §10.3's coverage-to-position inversion,
   normal estimation, and §10.5's boundary optimisation.
2. **No §10.5 boundary optimisation, so fitted split points sit on source
   samples.** A span runs sample to sample, and on a 45° staircase the samples
   off the ideal diagonal are `1/√2 ≈ 0.707` px from the chord joining the ones
   on it. A tolerance below that necessarily splits a boundary that is "really"
   one line. `flat-illustration` defaults to 0.6 px.
3. **No primitives and no arcs.** §11.7 is not implemented, so a circle is a
   handful of cubics rather than a circle, and PTE-GEO-011's "recognized
   primitives MUST remain represented semantically in the IR" has nothing to
   represent. `geometry.allowArcs` is refused by name.
4. **No independent renderer.** The seam gate uses a first-party rasteriser over
   the typed IR. §18.7's compatibility matrix (`resvg`, Chromium, Firefox,
   Inkscape, a fabrication importer) is **not satisfied**, and no claim of
   cross-renderer correctness is made.
5. **Native-versus-WASM parity is established for V8, not for browsers.**
   `make engine-parity` compiles `pte` for `wasm32-wasip1`, runs it under Node's
   WASI, and compares semantic digests against the native build across all 13
   fixtures. They match. What that establishes is that the engine's arithmetic
   and decisions are target-independent. What it does not establish: the
   behaviour of a `wasm-bindgen` binding, which does not exist, or of a browser
   engine other than V8. PTE-NO-049 is therefore *partly* discharged.
6. **No calibration.** Every threshold in this build — the merge threshold, the
   MSF scale parameter, the edge weights, the ambiguity weights, the default
   reaches, and now the corner scales, the 8° extrapolation limit and the
   turning limit — is an engineering choice, not a measured optimum. §31's
   numbers are "initial engineering gates". The synthetic corpus now exists;
   calibrating against it does not.
7. **No baseline comparison.** PTE-BASE-001 asks for VTracer, Potrace, AutoTrace
   and ImageTracerJS benchmarks and §39.1 for a blind SVG scorer that can grade
   an arbitrary external SVG. Neither exists, so "252 segments" has nothing to
   be better or worse *than*.
8. **No fuzzing, no benchmarks, no allocator instrumentation.** §30, §29 and
   PTE-PERF-002/005 are untouched. The adversarial *cases* appear in tests
   (checkerboards, noise, resource limits, a 2000-sample random walk); the
   adversarial *tooling* does not.
9. **Thin-feature protection is partial.** Two of §8.7's seven signals:
   elongation and width. Geodesic length, repeated-pattern evidence, endpoint
   and junction evidence, and profile role are not implemented.
10. **No tiling.** PTE-SEG-018/019 and §19.5 are absent, so peak memory scales
    with the whole image.
11. **The Python application does not use this engine.** Nothing in
    `palette_trace/` calls it, and the tracing backend registry is unchanged.
    Wiring the two together is a separate piece of work with its own licence
    question (the engine is permissive, the host is GPL), untouched by design.

## Deliberate deviations from the specification

Each is recorded where it happens, and repeated here so the list is in one
place.

| Deviation | Where | Why |
|---|---|---|
| Orchestration lives in `palette-tracer`, not `palette-tracer-core` | ADR-0002 | §5.1's assignment is circular |
| `ImageView` has private fields | `core::image` | PTE-API-001 requires validation before any pixel read; public fields would let a caller skip it |
| A `u32` label tier beyond §7.5's `u8`/`u16` | `core::labels` | §24.3's adversarial images exceed 65536 regions |
| 8-connected partition | `segment::edges` | §8.2 permits it, and under 4-connectivity §9.4 has nothing to decide |
| MSF with the Felzenszwalb–Huttenlocher predicate rather than a watershed cut | `segment::partition` | PTE-SEG-008 permits a named alternative; it is named here, in the module, and in `AlgorithmVersions` |
| Serialisation policy excluded from the semantic digest | `core::digest` | §20.1 defines the digest over quantised typed IR; Appendix E.1 classifies precision as invalidating serialisation only |
| No third-party curve library, though §37.4 suggests `kurbo` | `geometry::curves` | A nearest-point query on a cubic is a quintic root solve; a library's iteration count and convergence test are not part of its API contract, and PTE-DET-004 needs the same *decision* on every target rather than a similar number |
| Fitting cost is a lexicographic tuple, not §11.1's weighted sum | `geometry::segmentation` | §11.6 also gives a strict tie-break order. Weights would make that order emerge from three arbitrary constants, and PTE-DET-003 forbids a bare float from deciding anything |
| Corner evidence is the *minimum* turning over scales, not the mean | `geometry::chain` | PTE-GEO-002 forbids declaring a corner from stair-step noise. Requiring agreement at every scale is what makes a staircase score zero |
