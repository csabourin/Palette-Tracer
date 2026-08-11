# Palette Tracer Engine — implementation status

`engine/SPEC.md` is authoritative. This document records evidence; it does not
redefine requirements.

**Assessed at:** 2026-08-11, after complete-circle primitive recognition.
**Measured with:** `cargo test --workspace` → **417 tests and 2 doctests passed**;
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
     digest pte-semantic-v1-blake3:af0978c0927d5dfd8adb3671d44ce1d12386fa5c5d3718190df0322bce3b6537
```

with an automatic five-colour palette, one hole recovered, zero exposed seam
pixels, and 184 lines plus 68 cubics.

**A passing complete circle can remain a circle.** §11.7's first vertical
slice recognises one fully supported closed circular boundary before generic
fitting and carries `Primitive::Circle` through the typed document, semantic
digest and `<circle>` SVG. An opaque neighbouring face traverses the same
analytic boundary as two exact arcs, so primitive editability does not trade
away shared topology. Recognition is automatic in `logo` and available through
`recognize-primitives`; every candidate still has to pass the resolved
displacement bound. Other primitive families and the generic §11.4 arc
candidate remain absent.

**Boundary evidence is now subpixel where the input carries it.** §10.3–§10.5
are implemented: the compositing transfer is estimated from the evidence,
coverage is inverted to a signed offset in closed form, and §10.5's objective
moves each boundary sample along its own normal inside a trust region. The
encoded-sRGB transfer hypothesis is a deliberate real-input compatibility
extension to §10.2 and means PTE-AA-001 is only partly conforming. Boundaries
meeting in three- or four-colour junctions are solved once as shared vertices,
with cyclic-order, crossing and displacement guards. Individual boundaries are
reported as `coverage_reconstructed`,
`crisp_grid`, `low_confidence_fallback` or `pixel_art_policy` according to what
actually happened to each one, and §31.2's synthetic geometry gates are
**measured and met** — see the table below.

Where the input carries no antialiasing there is nothing to invert, and §10
says so rather than inventing a position. `curves/shallow-staircase` is
generated hard-edged and is untouched by §10, which is why it is still 8
segments.

---

## Phase status against §38

| Phase | Status | Evidence | Remaining |
|---|---|---|---|
| Phase 0 — evidence, licensing, contracts | Partly conforming | Workspace, licences, ADR-0001/2/3, typed config/IR/report/digest with round-trip tests, CLI and WASM adapter skeletons; a 13-fixture synthetic corpus with manifests (`tools/make_fixtures.py`); `cargo deny` wired to `make engine-deny` | No **real-world** corpus; no baseline benchmarks against VTracer, Potrace, AutoTrace or ImageTracerJS (PTE-BASE-001); memory and runtime budgets not calibrated (PTE-PERF-003); no blind SVG scorer (§39.1) |
| Phase 1 — colour and deterministic image foundation | Conforming | `palette-tracer-color`, 70 tests; OKLab reference vectors, hue wrap, alpha-zero invariance, exact-winner oracle, `u8`/`u16` transition | Plane lifetimes not measured with an allocator (PTE-PERF-002) |
| Phase 2 — segmentation and shared topology | Conforming | `palette-tracer-segment` 28 tests, `palette-tracer-topology` 35 tests; exhaustive 2×2/3×3 pattern oracle; reflection and rotation congruence | Tiling and tile reconciliation absent (PTE-SEG-018/019) |
| Phase 3 — subpixel boundaries and curve fitting | Partly conforming | §10.3 closed-form coverage inversion, §10.4 normals/confidence and shared multi-colour junctions, §10.5 bounded optimisation: `palette-tracer-aa`, 30 tests, `docs/notes/subpixel-antialias.md`, `docs/notes/junction-optimization.md`; §31.2's gates measured in `crates/palette-tracer/tests/subpixel_gates.rs`, junction topology in `crates/palette-tracer/tests/junction_topology.rs`. §11 lines and cubics with bidirectional validation, multi-scale corners and DP segmentation, plus complete-circle recognition: `palette-tracer-geometry`, 53 tests, `docs/notes/curve-fitting.md`, `docs/notes/primitive-recognition.md` | PTE-AA-001's encoded-transfer compatibility extension; §11.4 arcs; non-circular §11.7 primitives |
| Phase 4 — logo, illustration, fabrication | Partly conforming | Complete circles in `logo`, with a typed primitive and shared-neighbour lowering | Remaining §11.7 families, §12, §16 |
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
| PTE-GOAL-001/002/003 | conforming | The whole pipeline | `conformance_gates.rs`, 12 tests |
| PTE-GOAL-004 | conforming | §10 reconstruction feeding §11 fitting | `crates/palette-tracer/tests/subpixel_gates.rs`, 5 tests: all six of §31.2's gates measured and met on the analytic circles, including `reconstruction_beats_what_the_grid_alone_can_express` |
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
| PTE-API-013/014/015 | conforming | `pte` stream discipline and source-based flag precedence, independent of token order | `crates/palette-tracer-cli/tests/cli.rs`, 24 tests against the real binary, including `a_profile_flag_before_the_config_file_still_wins` and the existing stream-collision gate |
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
| PTE-SEG-015/016 | conforming | Fringe cleanup runs to a deterministic fixed point before topology, retaining audit records that name the compositing transfer. §8.7's shape veto is not consulted by this gate; the mixture residual protects hairlines | `an_antialias_fringe_is_absorbed_with_an_audit_record` requires a real reassignment and proves the next sweep is empty; `an_antialiased_hairline_is_not_absorbed_because_it_is_not_a_mixture` |
| PTE-SEG-017 | **partly** | Elongation and width only; `segmentation.protectThinFeatures` now gates the protection penalty instead of being digest-only | `protection_distinguishes_a_hairline_from_a_speck`, `disabling_thin_feature_protection_changes_the_merge_decision`. Geodesic length, repeated-pattern evidence, endpoint and junction evidence, and profile role are **not** implemented |
| PTE-SEG-018/019 | **not implemented** | — | No tiling |
| PTE-TOPO-001..007 | conforming | `topology::extract` | `twin_traversals_are_exact_reverses_of_one_chain`, `every_face_cycle_closes_exactly_once` |
| PTE-TOPO-008/009/010 | **partly** | §9.4 energy with data, continuity, mixture and connectivity terms | `reflection_reflects_the_decisions`. PTE-TOPO-009's asymptotic-decider data rule is **not** implemented; the data term is colour similarity. Of Kopf–Lischinski's heuristics only sparse-pixel is implemented |
| PTE-TOPO-011/012/013 | conforming | One weighted 2×2 solve per eligible shared vertex; every incident *end* receives the same `Point`, including both ends of a loop edge; trust radius, cyclic order and non-incident crossing are hard gates, the last answered against a segment index rather than a full scan | `a_multicolour_junction_is_optimized_once_and_shared_exactly`, `a_loop_edge_at_a_junction_stays_closed`, `a_junction_move_that_reorders_incident_edges_is_rejected`, `a_junction_move_that_crosses_a_nonincident_edge_is_rejected`; `docs/notes/junction-optimization.md`. Junctions are swept in vertex order, not solved jointly |
| PTE-TOPO-014 | conforming | Precision search refuses a collapse | `precision_never_collapses_two_distinct_points` |
| PTE-TOPO-015 | conforming | | `shared_mosaics_have_no_exposed_seam_pixels`, with the first-party rasteriser |
| PTE-TOPO-016/017/018 | partly | Modes are distinct and recorded in the layer role | Only `shared-mosaic` is produced; `stacked` and `separate-operations` are refused |
| PTE-AA-001/002 | **partly** | `color::mixture`: linear-light is the default and all residuals are judged there, but a compatibility hypothesis estimates coverage in encoded sRGB when linear fitting visibly fails. This deliberate deviation handles common raster output but is not literal PTE-AA-001 conformance | `an_encoded_space_blend_is_recognised_and_its_coverage_recovered`, `the_linear_hypothesis_alone_is_biased_by_a_fifth_of_a_pixel`, `a_linear_light_blend_still_chooses_the_linear_hypothesis`, `inseparable_endpoints_have_no_estimate_in_either_space`; `docs/notes/subpixel-antialias.md` §2 |
| PTE-AA-003 | conforming | `aa::square`: exact piecewise analytic square/half-plane inversion, the first of §10.3's three options. One `sqrt`, no iteration, so the decision is bit-identical across targets (PTE-DET-004) | `the_inverse_recovers_the_offset_that_produced_the_coverage`, `the_inverse_is_monotone_in_every_octant`, `reversing_the_labels_mirrors_the_offset`, all over every octant |
| PTE-AA-004 | conforming | `aa::normal`: tangents from the extracted polyline over a five-sample window | `a_forty_five_degree_staircase_reads_as_a_diagonal_not_as_axis_steps`, with `a_one_sample_window_would_alternate_between_axis_directions` measuring what the requirement forbids |
| PTE-AA-005 | conforming | Confidence from mixture residual, colour separation and normal stability; offsets bounded by the formula rather than clamped | `every_offset_lies_within_the_pixel_support`, `stability_falls_where_the_boundary_turns_a_corner`, `no_sample_leaves_its_trust_region` |
| PTE-AA-006 | conforming | §10.4 barycentric weights gate and weight one shared junction solve; they never override topology constraints, and the intersection gate is scale-free so weak nearly parallel evidence cannot buy a position | `three_colour_barycentric_weights_are_recovered`, `a_multicolour_junction_is_optimized_once_and_shared_exactly`, `a_four_colour_junction_is_reconstructed_as_one_shared_vertex`, `nearly_parallel_junction_evidence_is_refused` |
| PTE-AA-007/008 | conforming | Hard trust region of one pixel; the per-chain solve pins endpoints; the joint pass updates one vertex and all incident shared chains exactly once | `no_sample_leaves_its_trust_region`, `the_per_chain_solve_does_not_move_endpoints`, `a_multicolour_junction_is_optimized_once_and_shared_exactly`, `fitting_changes_only_the_chains` |
| PTE-AA-009 | conforming | The report censuses each edge's actual evidence class, and `geometry.evidence_is_the_pixel_grid` is emitted from that census with its count rather than unconditionally. `pixel-art`, which never runs §10, says so in its own words | `make engine-corpus`; `boundary_source_census`; `the_report_names_what_is_not_implemented`, `pixel_art_does_not_claim_coverage_reconstruction_ran` |
| PTE-GEO-001..009 | conforming | `palette-tracer-geometry`: preparation, multi-scale corners with hysteresis, line and cubic models, bidirectional error, DP segmentation | 53 geometry tests in total. `a_forty_five_degree_staircase_has_no_corners` (PTE-GEO-002), `a_ballooning_cubic_is_rejected_though_it_passes_through_the_samples` (§11.5), `the_chord_bound_is_never_exceeded_by_the_real_curve` (PTE-GEO-007), `exhausting_the_candidate_budget_falls_back_to_the_polyline` (PTE-GEO-005) |
| PTE-GEO-010 | **partly conforming** | Complete-circle recognizer: full-support, radial residual/max displacement, topology and material-complexity gates; work is linear apart from the residual sort and charged to the global budget | `a_complete_circle_is_recovered_in_source_coordinates`, `a_partial_arc_is_not_misrepresented_as_a_complete_circle`, `a_square_fails_the_radial_displacement_gate`, `primitive_recognition_charges_the_work_budget`, `the_real_extractor_emits_a_semantic_circle`; rectangles, ellipses, rounded rectangles, polygons and repeated radii remain absent |
| PTE-GEO-011 | conforming | `PrimitiveRecognition` remains typed across fitting/lowering; `Primitive::Circle` is hashed semantically and emitted as `<circle>`. An opaque neighbour gets the same circle as exact arcs | `the_real_extractor_emits_a_semantic_circle`, `the_opaque_neighbour_reuses_the_exact_circle_as_arcs`, `recognizing_a_reversed_circle_is_semantically_equivalent`; `docs/notes/primitive-recognition.md` |
| PTE-GEO-012/013/014 | conforming | One chain per shared edge, fitted once; endpoints are junctions by construction; the validator reruns after fitting | `fitting_changes_only_the_chains`, `fitting_never_moves_a_chain_endpoint`, `fitting_a_reversed_chain_gives_the_reversed_fit_exactly` |
| PTE-STROKE-010/011/012 | conforming | Interfaces emitted from shared edges | `no_coloring_book_interface_is_emitted_twice` |
| PTE-STROKE-001..009 | **not implemented** | — | No centrelines |
| PTE-GRAD-* , PTE-FAB-* | **not implemented** | Refused by name at configuration time | `a_stroke_or_gradient_request_is_refused_with_its_requirement` |
| PTE-SVG-001..005 | conforming | `svg::writer` | `output_is_a_standalone_svg_with_a_finite_viewport`, `a_hole_is_empty_under_nonzero_winding` |
| PTE-SVG-007/008/009 | conforming | Searched precision; automatic output reserves the subpixel reconstruction budget; one chain, two orientations | `precision_is_searched_not_fixed`, `automatic_precision_preserves_subpixel_reconstruction`, `shared_coordinates_are_byte_identical_after_reversal` |
| PTE-SVG-012/013 | conforming | Caller's bytes; alpha once | `a_palette_colour_survives_verbatim`, `alpha_is_applied_exactly_once` |
| PTE-SEC-001 | conforming | `escape_xml` | `a_hostile_label_is_escaped` |
| PTE-SEC-005/006/007 | conforming | `checked`, incremental limits; §10 charges the work budget per sample inverted and per junction probed, so no stage is outside the budget | `an_adversarial_region_count_hits_the_limit_cleanly` |
| PTE-SEC-009 | conforming | `unsafe_code = "deny"` workspace-wide | The workspace lint table |
| PTE-DET-001..004 | conforming | Quantised keys, stable heaps, no unordered iteration in an output path | `determinism.rs`, 8 tests, plus `make engine-parity`: the same engine compiled for `wasm32-wasip1` and run under Node's V8 produces byte-identical semantic digests on all 13 fixtures. Browser-engine parity beyond V8 is still unproven |
| PTE-LIC-001..005 | conforming | MIT-only engine: ADR-0004; clean-room and one-way dependency boundary: ADR-0003; `deny.toml`, `THIRD_PARTY_NOTICES.md` | `make engine-deny` → `advisories ok, bans ok, licenses ok, sources ok`. `cargo metadata` reports the workspace packages as MIT |
| PTE-NO-042 | conforming | Unimplemented settings are refused by name; implemented settings reach a decision path | `unimplemented_modifiers_are_refused`, `an_unimplemented_profile_is_refused_not_downgraded`, `disabling_thin_feature_protection_changes_the_merge_decision` |
| PTE-TEST-003/004 | partly | 13 synthetic fixtures regenerated from analytic descriptions, with §25.2 manifests: `tools/make_fixtures.py`, `make engine-fixtures` | Synthetic only. No real-world corpus, and no multiple-resolution or rotation sweep yet |

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
topology/subpixel-t-junction           3     6    13      0     523       0        0
curves/circle-subpixel-0               2     2     6      6     619       0        0
curves/circle-subpixel-1               2     2     6      6     625       0        0
curves/rounded-rectangle               2     2    12      8     616       0        0
curves/star-acute-corners              2     2    20     20    1104       0        0
curves/shallow-staircase               2     3     8      0     449       0        0
alpha/transparent-arbitrary-rgb        2     2    12      0     485       0        0
color/near-neutral-bands               3     6    12      0     518       0        0
pixel-art/diagonals                   34    96  1984      0   13298       0        0
```

Five things to read out of it.

**The fitting search never failed, and now rarely has anything left to
simplify.** The `fallback` column — chains that exhausted the candidate budget
— is zero everywhere. The `minimal` column, which counts chains whose polyline
was already the simplest faithful form, has collapsed from 25/19/36/82 on the
four `curves/` fixtures to zero. Those counts were never fitting failures; they
were thin fringe slivers whose short jagged boundaries had no simpler faithful
form. There are no fringe slivers now.

**Antialiased input is where this build improved most.** The census before §10
is worth putting beside it, because it is the same command on the same
fixtures:

```
                      faces  lines cubics  bytes        faces  lines cubics  bytes
circle-subpixel-0         8    146     16   1917   ->       2      6      6    619
circle-subpixel-1         9    158     22   2167   ->       2      6      6    625
rounded-rectangle        14    184      4   2181   ->       2     12      8    616
star-acute-corners       47    556     12   6160   ->       2     20     20   1104
```

A five-pointed star is now two faces — the star and its background — described
by 20 lines and 20 cubics. It was 47 faces and 556 lines. Both circles are also
the expected two faces after fixed-point fringe cleanup. Three changes did this,
and the larger one was not §10: §8.6's fringe gate was rejecting genuine fringe
because its mixture model assumed the wrong compositing transfer, and §8.7's
shape veto was blocking the rest. `docs/notes/subpixel-antialias.md` §1 records
the diagnosis. §10 then took the surviving boundaries subpixel, which is what
took the *segment counts* down by an order of magnitude.

**§31.2's gates are measured and met.** On the analytic circles, at a curve
tolerance of `0.10 px`:

| Metric | Gate | circle-0 | circle-1 |
|---|---:|---:|---:|
| Median boundary normal error | `≤ 0.10 px` | 0.028 | 0.055 |
| p95 boundary normal error | `≤ 0.35 px` | 0.155 | 0.262 |
| Max error (no junctions to exclude) | `≤ 0.75 px` | 0.391 | 0.537 |
| Circle centre error | `≤ 0.20 px` | 0.001 | 0.032 |
| Circle relative radius error | `≤ 1%` or `0.20 px` | 0.092% | 0.106% |

The tolerance matters and is not a loosened gate. §31.2's boundary-normal gates
and `geometry.curveTolerancePx` are budgets for the same quantity: the fitter
is *permitted* to deviate from its samples by up to the tolerance, so a profile
configured above a gate cannot meet that gate however exact the reconstruction
beneath it is. `flat-illustration` defaults to `0.6 px`, six times §31.2's
median target, and at that setting circle-1 measures a median of 0.103 and a
p95 of 0.392 — breached by the fitter's licence, not by reconstruction error.
The centre and radius gates hold at the default tolerance, because fitting
error is signed and averages out over a closed curve. No profile default was
changed and no gate was loosened; `the_gates_are_a_budget_shared_with_the_
fitters_tolerance` records the interaction with numbers.

**Un-antialiased geometry is unchanged, correctly.** The nested rectangles are
20 lines and no cubics; the donut is 30 cubics for two circles;
`pixel-art/diagonals` is deliberately unfitted (§15) and never reaches §10.
`curves/shallow-staircase` is still 8 segments for one straight edge, and that
is the honest answer: it is generated hard-edged, so there is no coverage to
invert. Item 1 in `docs/notes/curve-fitting.md` is therefore narrowed rather
than removed — §10 fixes the antialiased staircase, which is the common case,
and cannot fix the un-antialiased one, which carries no information.

---

**Multi-colour junction evidence is now exercised.**
`topology/subpixel-t-junction` is an analytic linear-light fixture with three
faces meeting at `(7.35, 7.62)`. It retains exactly those three faces and one
degree-three interior junction; all three interior boundaries are classified
as coverage reconstructed. The unit gate measures the recovered shared point
within `0.08 px` and exact equality across all incident chain endpoints.

---

## Gaps, stated plainly

These are the things a reader would otherwise have to discover.

1. **The compositing transfer is estimated per pixel, not pooled.** Two
   neighbouring samples on one boundary can in principle choose different
   hypotheses. The margin rule makes it rare and §10.5's smoothness absorbs a
   single disagreeing sample, but pooling the decision over an edge or an image
   would be strictly stronger evidence than deciding it afresh each time.
2. **A hard-edged staircase is still a staircase.** §10 narrowed item 1 of
   `docs/notes/curve-fitting.md` but did not remove it. Where antialias
   evidence exists, split points are no longer pinned to source samples; where
   it does not — `curves/shallow-staircase`, generated without antialiasing —
   the samples off the ideal diagonal are still `1/√2 ≈ 0.707` px from the
   chord joining the ones on it, so a tolerance below that still splits a
   boundary that is "really" one line.
3. **Primitive recognition is circle-only; generic arcs are absent.** A
   sufficiently supported complete circle can remain semantic through SVG
   lowering. Rectangles, ellipses, rounded rectangles, polygons, repeated
   radii and §11.4's partial-arc candidate remain unimplemented;
   `geometry.allowArcs` is still refused by name.
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
   reaches, the corner scales, the 8° extrapolation limit, the turning
   limit, and now §10's smoothness weight of 0.35, its eight Gauss-Seidel
   sweeps, its one-pixel trust radius, its 0.03 coverage band, the 4e-3
   quantisation floor and the 0.5 transfer margin — is an engineering choice,
   not a measured optimum. §31's
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
9. **Thin-feature protection is partial.** Its configuration switch now
   controls the merge penalty, but the score still uses only two of §8.7's
   seven signals: elongation and width. Geodesic length, repeated-pattern
   evidence, endpoint and junction evidence, and profile role are not
   implemented.
10. **No tiling.** PTE-SEG-018/019 and §19.5 are absent, so peak memory scales
    with the whole image.
11. **The Python application does not use this engine.** Nothing in
    `palette_trace/` calls it, and the tracing backend registry is unchanged.
    The licence choice is now settled: the engine is MIT and a future adapter
    belongs on the GPL side. Choosing and validating the integration mechanism
    remains separate engineering work; ADR-0004 defines the boundary.

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
| §10.2's mixture is fitted under two compositing hypotheses, not one | `color::mixture` | §10.2 states a linear-light model, which is only right if the *source* composited in linear light. Most rasterisers do not. PTE-AA-001 is honoured — every value compared and every residual reported is linear-light premultiplied — but the model the residual judges is chosen from the evidence. `docs/notes/subpixel-antialias.md` §2 |
| §10.5's `E_topology` and `E_pins` are hard constraints, not weighted penalties | `aa::reconstruct` | An infinite penalty *is* a constraint, and a constraint cannot be traded away by a large enough smoothness term. The per-chain solve pins endpoints; PTE-TOPO-011's separate joint solve may move only the one shared vertex while PTE-TOPO-013's legality gates hold |
| §8.7's protection score is not consulted by §8.6's fringe gate | `segment::rag` | §8.6 lists four conditions and a shape veto is not among them; §8.6 treats thinness as evidence *for* fringe. The proxy also over-scores the diagonal chains antialiasing produces. Colour, not shape, is what distinguishes a hairline |
