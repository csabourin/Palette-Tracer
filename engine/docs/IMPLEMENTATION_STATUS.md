# Palette Tracer Engine — implementation status

`engine/SPEC.md` is authoritative. This document records evidence; it does not
redefine requirements.

**Assessed at:** 2026-08-10, on the initial engine commit series.
**Measured with:** `cargo test --workspace` → **292 passed, 0 failed**;
`cargo clippy --workspace --all-targets -- -D warnings` → clean;
`cargo check --workspace --target wasm32-unknown-unknown` → clean.
Toolchain `rustc 1.94.1` on `x86_64-unknown-linux-gnu`.

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
one shared boundary graph. On the repository's own `examples/sample.png`
(320×240), converted to PAM with `tools/png_to_ppm.py`:

```
pte: 5 faces, 10 shared edges, 29115 bytes,
     digest pte-semantic-v1-blake3:83a2eacc9329e79473674cac3d3ff02d44f32c83986914d0e4db1255ecb71950
```

with an automatic five-colour palette, one hole recovered, and zero exposed
seam pixels.

**Boundaries are on the pixel grid.** That is the single most important thing
to know about this build. Curve fitting (§11) is not implemented, so a boundary
that should be one line or one Bézier is emitted as one line segment per pixel
step: the sample above contains 4022 line segments where a fitted result would
contain a small fraction of that. The output is *correct* — seam-free, valid,
deterministic, topologically sound — and it is *not compact*. §31.5's complexity
gates are not met and are not claimed.

---

## Phase status against §38

| Phase | Status | Evidence | Remaining |
|---|---|---|---|
| Phase 0 — evidence, licensing, contracts | Partly conforming | Workspace, licences, ADR-0001/2/3, typed config/IR/report/digest with round-trip tests, CLI and WASM adapter skeletons | No reference corpus with manifests; no baseline benchmarks against VTracer, Potrace, AutoTrace or ImageTracerJS (PTE-BASE-001); memory and runtime budgets not calibrated (PTE-PERF-003) |
| Phase 1 — colour and deterministic image foundation | Conforming | `palette-tracer-color`, 64 tests; OKLab reference vectors, hue wrap, alpha-zero invariance, exact-winner oracle, `u8`/`u16` transition | Plane lifetimes not measured with an allocator (PTE-PERF-002) |
| Phase 2 — segmentation and shared topology | Conforming | `palette-tracer-segment` 26 tests, `palette-tracer-topology` 35 tests; exhaustive 2×2/3×3 pattern oracle; reflection and rotation congruence | Tiling and tile reconciliation absent (PTE-SEG-018/019) |
| Phase 3 — subpixel boundaries and curve fitting | **Not implemented** | — | All of §10 and §11 |
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
| PTE-GOAL-004 | **not implemented** | — | §10 is absent; boundaries are grid-aligned |
| PTE-GOAL-005 | **not implemented** | — | §13 is absent |
| PTE-GOAL-006 | partly | 4 of 11 profiles | `unimplemented_profiles_are_refused_not_silently_downgraded` |
| PTE-GOAL-007 | partly | Library, CLI, host-free WASM adapter | No `wasm-bindgen` binding; no C ABI |
| PTE-GOAL-008 | conforming | `core::report` | `the_report_has_the_appendix_a_top_level_keys`, `the_report_names_what_is_not_implemented` |
| PTE-ARCH-001..003 | conforming | `core` depends on no sibling; no host API anywhere | `cargo check --target wasm32-unknown-unknown` |
| PTE-ARCH-006/007 | conforming | `VectorDocument`; the seven-category error taxonomy | `exit_codes_match_the_spec_table` |
| PTE-ARCH-011 | conforming | `kurbo` is declared but not yet used; no third-party geometry type is public | `THIRD_PARTY_NOTICES.md` |
| PTE-API-001/002/003 | conforming | `core::image` | 8 tests including padded rows and adversarial strides |
| PTE-API-005/006 | conforming | `TraceOutput`; `check_cancel` on a bounded stride | `cancellation_takes_effect_inside_a_long_loop` |
| PTE-API-007/008/009 | conforming | `Finite`, `deny_unknown_fields`, `Resolver` | `an_unknown_key_is_refused`, `provenance_distinguishes_user_from_profile` |
| PTE-API-010/011 | conforming | Identifiers from raster order; `f64` throughout | `labels_are_numbered_by_raster_position_not_allocation_order` |
| PTE-API-012 | conforming | Stable codes on every warning and error | `codes_are_category_prefixed_and_stable` |
| PTE-API-013/014/015 | conforming | `pte` stream discipline and flag precedence | Manual; no committed CLI test suite — see *Gaps* |
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
| PTE-GEO-001..025 | **not implemented** | `pixel-art` blocky output falls out of the grid-aligned boundary | All of §11 |
| PTE-STROKE-010/011/012 | conforming | Interfaces emitted from shared edges | `no_coloring_book_interface_is_emitted_twice` |
| PTE-STROKE-001..009 | **not implemented** | — | No centrelines |
| PTE-GRAD-* , PTE-FAB-* | **not implemented** | Refused by name at configuration time | `a_stroke_or_gradient_request_is_refused_with_its_requirement` |
| PTE-SVG-001..005 | conforming | `svg::writer` | `output_is_a_standalone_svg_with_a_finite_viewport`, `a_hole_is_empty_under_nonzero_winding` |
| PTE-SVG-007/008/009 | conforming | Searched precision; one chain, two orientations | `precision_is_searched_not_fixed`, `shared_coordinates_are_byte_identical_after_reversal` |
| PTE-SVG-012/013 | conforming | Caller's bytes; alpha once | `a_palette_colour_survives_verbatim`, `alpha_is_applied_exactly_once` |
| PTE-SEC-001 | conforming | `escape_xml` | `a_hostile_label_is_escaped` |
| PTE-SEC-005/006/007 | conforming | `checked`, incremental limits | `an_adversarial_region_count_hits_the_limit_cleanly` |
| PTE-SEC-009 | conforming | `unsafe_code = "deny"` workspace-wide | The workspace lint table |
| PTE-DET-001..004 | partly | Quantised keys, stable heaps, no unordered iteration in an output path | `determinism.rs`, 8 tests. Cross-target parity is **not** established — see *Gaps* |
| PTE-LIC-001..005 | conforming | ADR-0003, `deny.toml`, `THIRD_PARTY_NOTICES.md` | `cargo deny check` is not wired into a command yet — see *Gaps* |
| PTE-NO-042 | conforming | Unimplemented settings refused by name with the governing requirement | `unimplemented_modifiers_are_refused` |

---

## Gaps, stated plainly

These are the things a reader would otherwise have to discover.

1. **No curve fitting.** Output is grid-aligned polylines. §31.5's complexity
   gates ("a rectangle SHOULD be one semantic rectangle or at most four line
   segments") are **not met**. This is the largest single gap and the next
   thing to build (§39.9).
2. **No subpixel reconstruction.** Every boundary is reported as `crisp_grid`
   or `pixel_art_policy`; `coverage_reconstructed` is always zero. The §31.2
   subpixel gates have **not been measured**, because there is nothing to
   measure yet.
3. **No independent renderer.** The seam gate uses a first-party rasteriser
   over the typed IR. §18.7's compatibility matrix (`resvg`, Chromium, Firefox,
   Inkscape, a fabrication importer) is **not satisfied**, and no claim of
   cross-renderer correctness is made.
4. **No native-versus-WASM parity evidence.** `cargo check --target
   wasm32-unknown-unknown` proves the engine is host-free. It does **not**
   prove the digests match, because the conformance manifest has not been run
   in a browser or a JavaScript runtime. PTE-NO-049 is therefore not
   discharged.
5. **No calibration.** Every threshold in this build — the merge threshold, the
   MSF scale parameter, the edge weights, the ambiguity weights, the default
   reaches — is an engineering choice, not a measured optimum. §31's numbers
   are "initial engineering gates" and Phase 0 was to calibrate them against a
   reference corpus. There is no reference corpus.
6. **No fuzzing, no benchmarks, no allocator instrumentation.** §30, §29 and
   PTE-PERF-002/005 are untouched. The adversarial *cases* appear in tests
   (checkerboards, noise, resource limits); the adversarial *tooling* does not.
7. **No `cargo deny` in a command.** `deny.toml` is written and reviewed;
   nothing runs it. PTE-LIC-005 says licence checks are release-blocking, and
   there is no release.
8. **No committed CLI test suite.** `pte` was exercised by hand on the
   repository's sample image. `netpbm.rs` has unit tests; the command surface
   does not.
9. **Thin-feature protection is partial.** Two of §8.7's seven signals.
10. **The Python application does not use this engine.** Nothing in
    `palette_trace/` calls it, and the tracing backend registry is unchanged.
    Wiring the two together is a separate piece of work with its own licence
    question (the engine is permissive, the host is GPL).

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
