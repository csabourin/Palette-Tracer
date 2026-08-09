# Palette Trace for Inkscape

## Product Requirements and Technical Implementation Specification

**Status:** Implementation specification
**Audience:** Coding agents, extension developers, maintainers and contributors
**Initial target:** Inkscape 1.4 or later, with runtime capability detection
**Document date:** August 5, 2026
**Proposed project licence:** GPL-3.0-or-later
**Working project name:** Palette Trace

---

# 1. Normative language

The terms **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT** and **MAY** are normative.

* **MUST / REQUIRED:** Necessary for the MVP to be considered complete.
* **SHOULD:** Strong recommendation; deviations require a documented technical reason.
* **MAY:** Optional behaviour that does not affect conformance.
* **Future:** Explicitly outside the MVP but anticipated by the architecture.

The coding agent MUST not silently omit a requirement because it is difficult. Any requirement that cannot be completed MUST be documented as incomplete, including the reason, observable impact and proposed path forward.

---

# 2. Product vision

Palette Trace is an Inkscape extension for **art-directed multicolour bitmap tracing**.

Unlike ordinary posterization, Palette Trace allows users to:

1. Choose the total number of colour scans.
2. Pick and pin specific colours from the source image.
3. Require picked colours to become exact SVG output colours by default.
4. Control how broadly each pinned colour claims similar source colours.
5. Refine that claim using separate hue, saturation/chroma and lightness tolerances.
6. Allow remaining scans to be generated automatically.
7. Apply different mask-cleanup and path-tracing profiles to different colour scans.
8. Choose a destination preset such as illustration, branding, screen printing, vinyl cutting or laser work.
9. Preserve the settings on the source image so the trace can be reopened and adjusted later.
10. Use a replaceable tracing backend so the project is not permanently tied to one native binary, operating system or tracing engine.

The extension is not merely an alternative interface for Inkscape’s existing Trace Bitmap dialog. It introduces a distinct preprocessing and scan-management pipeline.

Inkscape’s own source describes tracing conceptually as preprocessing, tracing and post-processing, although current engines sometimes combine those stages. Palette Trace MUST preserve that separation internally.

---

# 3. Primary user promise

The primary promise is:

> The user controls the important colours. The extension intelligently derives the remaining palette and traces each colour according to its visual or production role.

When a user picks a colour, Palette Trace MUST assume:

> “This is the output colour I want.”

The next question is:

> “How broadly should this colour claim nearby source colours?”

The user-facing name for that concept is **Colour reach**.

---

# 4. Non-goals for the MVP

The MVP MUST NOT attempt to provide all of the following:

* Manual bitmap painting or mask brushing.
* Arbitrary lasso selection of source pixels.
* Fully topological shared-boundary vectorization.
* Continuous per-pixel alpha reconstruction.
* Gradient reconstruction.
* Mesh generation.
* Device-specific laser instructions.
* Device-specific vinyl cutter output.
* CMYK colour management.
* Automatic spot-colour registration marks.
* Automatic semantic recognition of objects.
* AI-based image segmentation.
* Tracing of arbitrary SVG filter output.
* Network or cloud processing.
* Batch processing of multiple images.
* Editing the source bitmap.
* Replacing Inkscape’s native Trace Bitmap feature.

The architecture MUST leave room for these capabilities without requiring the MVP to implement them.

---

# 5. Technical basis and constraints

An Inkscape effect extension receives an SVG document, modifies it and returns the modified SVG. The INX descriptor identifies the extension, interpreter, parameters, dependencies and menu location.

Palette Trace requires a dynamic interface that cannot reasonably be represented as a fixed set of INX widgets. Standard INX widgets support predefined controls, but Palette Trace requires a changing palette, expandable per-scan controls, image sampling, mask overlays and layer ordering.

A custom GUI extension blocks interaction with the main Inkscape interface until the extension process returns. The implementation therefore MUST NOT require the user to click on the main Inkscape canvas while the Palette Trace interface is open. It MUST provide its own image preview and colour picker.

Inkscape exposes a command-line `object-trace` action, but its current interface accepts one global seven-value tracing configuration and constructs a colour-quantization Potrace engine. It does not expose prepared binary masks, custom palettes or separate settings for individual scans. Therefore, this action MUST NOT be treated as the required primary backend.

Calling another Inkscape process through `inkex.command` is resource-intensive and should be avoided unless necessary. Any Inkscape CLI adapter MUST batch work and MUST NOT launch one complete Inkscape process per scan.

The extension MAY store arbitrary namespaced attributes on SVG elements. Palette Trace will use this to store image-specific settings directly on the selected `<image>` element.

Because the interface is a local web application rather than an INX form, almost none of the product depends on Inkscape being present. Inkscape supplies a document and a selected bitmap; everything the user interacts with is served by the extension process itself. Palette Trace therefore treats Inkscape as one host among two, and also runs as a standalone local web application over image files on disk. §9.4 defines the host contract that keeps both honest.

Pillow is part of the documented `inkex` dependency set, while NumPy is also declared by the project. Palette Trace SHOULD use Pillow for decoding and MAY use NumPy for acceleration, but correctness MUST NOT depend solely on NumPy being available.

VTracer is one possible third-party backend candidate because its project supports raster-to-SVG conversion, binary tracing, Python integration and a pluggable pipeline. It is a candidate, not a required dependency.

---

# 6. Defined terminology

## 6.1 Source image

The selected SVG `<image>` element whose raster content will be traced.

The source image MAY be:

* Embedded as a data URI.
* Linked to a local file.
* Transformed, scaled, rotated or skewed in the SVG document.

The MVP MUST reject:

* Remote HTTP or HTTPS image URLs.
* Multiple selected source images.
* A selection containing no valid image.
* Images whose linked file cannot be resolved.

## 6.2 Scan

One intended colour layer in the traced output.

A scan consists of:

* One palette entry.
* One classified source-pixel region.
* One cleaned binary mask.
* Zero or more generated SVG paths.
* One output colour or production role.
* One trace profile.

The requested scan count is a **maximum target count**. Empty or merged scans may cause the final output to contain fewer non-empty scan groups.

## 6.3 Palette entry

A configured colour slot corresponding to one scan.

A palette entry is either:

* **Automatic:** Its source anchor and output colour are calculated.
* **Pinned:** Its output colour is controlled by the user.

## 6.4 Source anchor colour

The source colour around which pixel matching or clustering occurs.

This is separate from the output colour.

Example:

```text
Source anchor: #59738A
Output colour: #0057B8
```

This supports restoring a faded source logo to an exact official colour.

## 6.5 Output colour

The colour assigned to the generated SVG paths.

For a newly picked colour, the output colour MUST default to the exact sampled colour.

## 6.6 Colour reach

A user-friendly measure of how broadly a pinned colour claims similar source pixels.

Colour reach MUST be expressed as an integer from 0 through 100.

The interface MUST explain it as:

> Controls how broadly this colour claims nearby source colours.

The interface MAY include informal wording such as “how much this colour eats nearby colours” in contextual help, but configuration and documentation MUST use **Colour reach** and **claim**.

## 6.7 Claim

The act of assigning a source pixel to a pinned palette entry before automatic palette generation.

## 6.8 Reserved matching

A pinned colour with reserved matching claims pixels that fall within its configured colour reach.

Reserved matching answers:

> “Which source pixels belong to this exact output colour?”

## 6.9 Fixed-centre matching

A pinned colour may instead act as a fixed cluster centre. It then participates in ordinary nearest-palette classification without preclaiming pixels.

This is an advanced mode.

## 6.10 Trace profile

A named or custom set of mask-cleanup and vectorization settings.

Examples:

* Smooth shapes
* Sharp details
* Thin line art
* Small accents
* Simplified background
* Fabrication clean

## 6.11 Destination preset

A built-in, immutable policy describing the expected use of the SVG output.

Initial destination presets:

* Illustration
* Logo / branding
* Screen printing
* Vinyl / paper cutting
* Laser
* Custom

A destination preset affects geometry, validation, trace defaults and output organization.

## 6.12 Saved user preset

A reusable user-created configuration stored outside the SVG image.

A saved user preset MAY include:

* Destination.
* Scan count.
* Palette structure.
* Exact palette colours.
* Colour reaches.
* Trace profiles.
* Geometry settings.

A saved user preset MUST NOT contain:

* Source-image identifiers.
* Source checksums.
* Generated-group identifiers.
* Temporary preview data.
* Cached masks.

## 6.13 Image settings

The complete configuration associated with one source image and stored directly on that image.

Deleting the image therefore deletes its settings.

## 6.14 Geometry policy

The method used to turn classified colour regions into output layers.

Initial policies:

* `stacked`
* `stacked_trapped`
* `exclusive_layers`
* `separate_operations`

A future policy MAY be:

* `shared_boundaries`

## 6.15 Backend

A component that converts one cleaned binary mask into SVG path geometry.

The backend MUST NOT decide:

* Palette colours.
* Colour reach.
* Pixel ownership.
* Destination.
* Layer order.
* Background semantics.
* Source-to-output colour mapping.

These belong to Palette Trace itself.

---

# 7. User decisions incorporated into this specification

The product MUST support all principal output destinations rather than optimize only for illustration.

Destination presets MUST determine whether overlaps, trapping or exclusive layers are appropriate.

When the user picks a colour:

* It MUST become a pinned palette entry.
* The output MUST default to that exact colour.
* Reserved matching MUST be enabled.
* Colour reach MUST default to a conservative **Narrow** value.
* The user MUST be shown how much of the image the colour currently claims.

Settings MUST be stored on the selected image.

When the image is selected again, the extension MUST restore its settings.

When the image is deleted, its image-specific settings MUST disappear with it.

Saved user presets MUST remain available independently and MAY be applied to a new image.

Portability is a primary architectural requirement. The core extension MUST not be coupled to one operating-system-specific native executable.

---

# 8. Supported workflow

## 8.1 New trace

1. User selects one image in Inkscape.
2. User opens **Extensions → Raster → Palette Trace**.
3. Extension validates the selection.
4. Extension reads image-specific settings if present.
5. Otherwise, it initializes settings from the Illustration destination preset.
6. Extension opens the Palette Trace interface.
7. User chooses a destination.
8. Extension generates an automatic palette.
9. User samples important colours from the picture, adding to the palette or removing from it.
10. Picked colours become pinned exact output colours.
11. User adjusts Colour reach and optional advanced channel tolerances.
12. User assigns roles and per-scan trace profiles.
13. User previews colour assignment.
14. User previews vector output.
15. User applies the trace.
16. Extension writes settings to the source image.
17. Extension creates or updates a generated trace group.

## 8.2 Reopen trace settings

1. User reselects the same image.
2. User opens Palette Trace.
3. Extension restores image settings.
4. If the source fingerprint matches, cached semantic configuration is reused.
5. Derived masks and paths MUST be recalculated rather than stored in the SVG.
6. User modifies and reapplies the trace.

## 8.3 Apply a saved preset to another image

1. User selects another image.
2. User opens Palette Trace.
3. User chooses **Load preset**.
4. Extension applies only the categories included in the preset.
5. Image-specific identifiers and generated-result references are newly created.
6. Automatic palette entries are recalculated from the new image.

---

# 9. Extension interface architecture

## 9.1 Required portable interface

The MVP SHOULD use a bundled local HTML, CSS and JavaScript application served by the extension process.

The application MUST:

* Run entirely on the local computer.
* Bind only to `127.0.0.1` or `::1`.
* Use an ephemeral operating-system-assigned port.
* Require an unguessable per-session token.
* Load no remote scripts, fonts, styles or images.
* Make no external network requests.
* Expose no arbitrary filesystem browsing endpoint.
* Shut down after Apply, Cancel or inactivity timeout.
* Reject requests without the session token.
* Reject unsupported HTTP methods.
* Restrict accepted payload size.
* Avoid including local filesystem paths in browser-visible data unless essential.
* Delete temporary files at session completion.

The custom extension process MUST remain alive while the local application is open.

Because the main Inkscape interface is blocked during a custom-GUI extension, all picking and previews MUST be available in this local interface.

## 9.2 Interface layout

The interface MUST be usable on a touchscreen phone. That is the binding constraint on this section: it is why the narrow layout is the base case rather than a reduction of the desktop one, why no interaction may depend on hover, a modifier key or a pointer, and why every interactive target is at least 44 CSS pixels on its shortest side.

The interface MUST use a stacked narrow-screen layout, and SHOULD widen into a two-panel layout — preview beside controls — where there is room for one.

### 9.2.1 Progressive disclosure

The interface MUST NOT present a control before it can do anything.

* With no source bitmap loaded, the interface MUST show only the means of loading one (§9.4.2). Destination, scan count, palette and preview controls MUST NOT be on screen.
* Controls that configure a subject that does not yet exist MUST be hidden until it does. The background matching and output modes (§16) are hidden until a background entry is chosen; a scan's colour-reach controls appear only for a pinned entry (§13); a trace profile's fields appear only when that profile is being set by hand (§18).
* The following MUST be reachable but MUST NOT be presented before the user asks for them: tracing backend selection, background handling, reset to destination defaults, per-scan colour-reach channels, and per-scan trace profiles.

Nothing in this subsection permits removing a control. Every setting named in this specification remains reachable without leaving the interface.

### 9.2.2 Naming

Controls MUST be named for the action they perform or the outcome they produce, in the user's vocabulary rather than this specification's. A control MUST NOT be labelled with a settings-schema key, and a destination MUST be offered as the thing being made rather than as a preset identifier.

Where a setting's effect can be stated as a result, the interface SHOULD state it — the share of the picture a colour covers, the number of flat colours the result will contain, what a destination does to the geometry.

### 9.2.3 Main controls

* Load a source bitmap (§9.4.2)
* Destination
* The palette (§9.2.5)
* Tracing backend
* Load preset
* Save preset
* Reset to destination defaults
* Commit the result

The commit control MUST name where the result will go: into the open document, into the configured output file, or as a download (§9.4.2).

The number of scans is deliberately absent from this list as a control of its own. §10.4 requires `palette.entries.length` to equal `scanCount`, so the palette already states the number; a separate number to type would be a second way to say the same thing, and the two would disagree the moment a colour was added or removed. The interface MUST NOT offer a scan-count control separate from the palette itself: the count follows the length of the list (§9.2.5).

Preview quality is deliberately absent from this list, and no longer because it would do nothing: §17.4 preview scaling is implemented, and a preview traces a reduced copy of the bitmap whenever the source exceeds the preview pixel budget. The reason it stays out is that the budget is derived from the source's own dimensions, so the factor is already the largest one that answers quickly — a control would offer the user a choice between "as good as it can be" and "worse", and the interface MUST NOT ask a question whose better answer it already knows.

This holds only while the preview differs from the result in geometry alone. It does not: an automatic palette previewed from a reduced copy resolves its swatches from resampled pixels, so a swatch MAY differ slightly from the one the delivered file carries (§34.5 is satisfied against the bitmap actually traced). Should that drift prove to matter in use, the honest fix is to resolve the automatic palette at full resolution rather than to add a control asking the user to work around it; a preview-quality control returns to this list only if some case is found that neither of those addresses.

### 9.2.4 Preview panel

Preview modes:

* **Source** — the decoded bitmap, unmodified.
* **Result** — every scan's traced geometry filled with its output colour. This is the quantized-colour, vector and production preview of earlier drafts of this section: they render the same geometry, and offering them as separate modes implied a difference the pipeline does not produce.
* **Coverage** — each scan's claimed region, tinted with its output colour and outlined, over the source. This carries the selected-colour claim, all-scan-mask, overlap and mask-boundary requirements below.

Required interactions:

* Zoom, including a two-finger pinch gesture where the pointer supports one
* Pan
* Fit to window
* Actual pixels
* Sample colour (§9.3)
* Toggle scan visibility
* Show claimed-pixel percentage
* Show overlaps or unresolved pixels
* Show mask boundaries

Every one of these MUST have a control or key that does not require a pointer gesture.

### 9.2.5 Palette panel

The palette is a list of colours. Each row MUST show, without the user opening anything:

* Swatch
* Name
* Automatic or Pinned status
* Output colour, as text
* Share of the source it covers
* Warning indicator

Opening a row MUST additionally expose:

* Source anchor
* Colour reach
* Role
* Trace profile
* Visibility
* Layer order
* Remove

Reordering MUST be possible through both:

* Drag and drop.
* Keyboard-accessible Move up / Move down controls.

Colour MUST NOT be the only means of identifying a scan. Every row therefore carries its output colour and its automatic/pinned status as text.

The whole palette MUST be reachable as one list in which a colour can be added and any colour removed, without opening a row. This is where the number of colours is set (§9.2.3): adding lengthens the list, removing shortens it, and `scanCount` follows in both directions.

Adding a colour to the palette MUST add one. It MUST NOT overwrite or consume an entry already in the list — neither a colour the user picked earlier nor an automatic entry, whose colour is as much a part of the result as any other. Removal is how the palette gets shorter, and it MUST be refused for the last remaining entry, which §10.4 requires to exist.

A colour MUST enter the palette by being sampled from the source bitmap (§9.3). The interface MUST NOT offer a way to add a colour the picture does not contain: an entry with no pixels to claim contributes no geometry, and offering it invites a palette that cannot produce the result it depicts. Changing an entry already in the palette to a stated exact value MUST remain possible, because a screen print or a cut file is sometimes specified by an ink or material reference rather than by what the picture happens to hold.

## 9.3 Picked-colour behaviour

Default picker sample:

* 5 × 5 source-pixel neighbourhood.
* Median or robust dominant sample.
* Not arithmetic mean.

An arithmetic mean is forbidden because averaging across an edge returns a colour that appears nowhere in the source — precisely the colour a user aiming at an edge does not want.

Sample size:

* Exact-pixel sampling MUST be available.
* A larger dominant-colour sample MAY be offered.
* Sample size MUST be selectable through an on-screen control. A modifier key MUST NOT be the only way to reach a sample size, because a touchscreen has none.

Aiming:

* The interface MUST provide a magnifier that shows the source pixels around the sample point at a magnification sufficient to distinguish individual pixels, with the pixel to be sampled marked.
* The magnifier MUST track a press-and-drag gesture and commit on release, so the sample point is not obscured by the finger making it.
* Aiming MUST also be possible without a pointer: a keyboard MUST be able to move the sample point by one source pixel at a time and commit it.

After sampling:

```text
Kind: Pinned
Assignment: Reserve within reach
Output: Exact selected colour
Colour reach: Narrow
```

The selected colour’s claim overlay MUST update immediately.

## 9.4 Application hosts

The local interface described in §9.1 is the whole of the user experience. Inkscape supplies a document, a selected bitmap and a place to write the result; it does not supply the interface. Palette Trace therefore MUST be structured so that Inkscape is one *host* rather than a precondition, and MUST support two hosts:

* the **Inkscape extension host**, which reads and writes an open SVG document; and
* the **standalone host**, which reads an image file from disk and writes an SVG file.

Both hosts MUST drive the same headless core and the same local interface. Identical inputs MUST produce identical geometry in either host (§34.30).

### 9.4.1 Host contract

A host is responsible for exactly four things:

1. Supplying one decoded source bitmap.
2. Loading and persisting image settings conforming to §11.
3. Committing the generated result.
4. Reporting errors without destroying user data.

A host MUST NOT reimplement palette logic, claim resolution, quantization, mask cleanup, geometry policy or tracing.

The following MUST hold for the shared modules:

* The headless core MUST NOT import `inkex`.
* Modules that import `inkex` MUST be confined to the Inkscape host.
* `inkex` MUST be an optional dependency, required only for the Inkscape host.

### 9.4.2 Standalone host requirements

The standalone host MUST:

* Accept one local image path, **or** no image path at all, in which case it serves the interface's image-loading screen and the user chooses a bitmap in the browser (§9.2.1).
* Serve the same local interface under the same §9.1 and §31 constraints.
* Open the interface in the user's browser.
* Write a standalone SVG document on Apply, when it has an output path to write to.
* Exit after Apply, Cancel or inactivity timeout.
* Function with no Inkscape installation present.

The standalone host MUST NOT:

* Accept a remote URL as a source.
* Serve any file outside its bundled interface assets and the selected source image.
* Write outside the user-specified output path.

#### Browser-supplied source bitmaps

Requiring a path on the command line makes the standalone host unusable from any device without a shell, and from any device whose filesystem is not the server's — which is every phone. The interface therefore MUST be able to supply the source bitmap itself.

The bitmap's bytes MUST arrive in a request body. The host MUST NOT gain an endpoint that lists, browses or reads a path chosen by the client: §9.1's prohibition on arbitrary filesystem browsing is not relaxed by this section, and the browser's own file chooser is what selects the file.

An uploaded bitmap:

* MUST be decoded in memory. It MUST NOT be written to disk, so there is no temporary file to delete at session completion (§9.1).
* MUST be rejected unless its declared type is a bitmap format the host can decode.
* MUST be rejected above a documented byte ceiling, enforced before decoding (§9.1 restricted payload size).
* MAY be scaled down before tracing when it is large enough that full-resolution tracing would appear to hang. When it is, the interface MUST say so and MUST state the dimensions actually traced. The scale factor MUST be derived from a fixed pixel budget, so the same upload always produces the same geometry (§34.30).
* MUST replace the session's settings with destination defaults rather than inheriting the previous bitmap's palette. Pinned colours, claims and layer order describe the bitmap they were made against.

A session whose bitmap arrived this way has no source path, and therefore:

* has no sidecar location (§9.4.3), so its settings are session-scoped. A saved preset (§26) is how a configuration outlives such a session.
* MUST NOT write to an output path derived from a different bitmap. Its result MUST leave as a download instead.

Delivering a result as a download MUST NOT end the session. A download is a checkpoint; the user may adjust and produce another.

The Inkscape host MUST NOT offer browser-supplied bitmaps. It is bound to a selected `<image>` whose settings live on that element (§10.2) and whose result is committed beside it; swapping the bitmap underneath it has no meaning.

### 9.4.3 Standalone persistence

The Inkscape host stores settings on the source `<image>` element (§10.2) so that deleting the image deletes its settings (§34.23). A standalone host has no document to store them in, and MUST instead write a sidecar file beside the source image:

```text
<source image path>.palettetrace.json
```

The sidecar MUST contain the same §11 image-settings object that `pt:settings` would contain.

The sidecar MUST be treated as advisory: a missing, unreadable or wrong-version sidecar MUST fall back to destination defaults rather than failing.

A bitmap supplied through the browser (§9.4.2) has no path and therefore no sidecar. Its settings are session-scoped; a saved preset (§26) is the mechanism for carrying a configuration beyond such a session.

Deleting the source image does not delete the sidecar. The standalone host MUST tolerate an orphaned sidecar and SHOULD ignore one whose recorded fingerprint cannot be reconciled with any source.

### 9.4.4 Standalone output

Generated SVG MUST:

* Contain the same group structure, labels and `pt:` attributes defined in §10.3 and §10.4.
* Declare the `pt` namespace on the root element.
* Use a `viewBox` matching the intrinsic source dimensions.
* Embed no raster data unless the user explicitly requests that the source be included.
* Be written atomically — written to a temporary file in the destination directory and then moved into place — so an interrupted run cannot leave a truncated SVG (§34.29).

The standalone host MUST NOT overwrite an existing output file without an explicit instruction.

### 9.4.5 Host parity

Where a requirement in this specification refers to the SVG document, the source `<image>` element or the generated group, the standalone host MUST satisfy the equivalent obligation against its own source file, sidecar and output file. Specifically:

| Requirement | Inkscape host | Standalone host |
| ----------- | ------------- | --------------- |
| §34.1 Open one selected bitmap | Selected `<image>` in the document | Image path given on the command line, or a bitmap chosen in the browser (§9.4.2) |
| §34.2 Restore stored settings | `pt:settings` on the image | Sidecar file; a browser-supplied bitmap has none, and starts from destination defaults |
| §34.22 Settings stored on the source | On the `<image>` element | Sidecar beside the image; not applicable to a browser-supplied bitmap, which has no path |
| §34.23 Deleting the image deletes its settings | Inherent — settings are an attribute | Not applicable; the sidecar is separate and orphans MUST be tolerated |
| §34.25 Replace the linked generated group | Replace the group in place | Rewrite the output SVG, or emit a fresh download |
| §34.29 Errors do not corrupt the document | Group swap after successful build | Atomic file replace |

---

# 10. Data persistence model

## 10.1 Custom XML namespace

Use:

```text
https://christiansabourin.com/ns/palette-trace/1
```

Suggested prefix:

```text
pt
```

The URI is an identifier, not a required network endpoint.

## 10.2 Attributes on the source image

The source `<image>` MUST contain:

```xml
pt:image-uuid="..."
pt:schema-version="1"
pt:settings="..."
```

`pt:settings` MUST contain minified JSON conforming to the schema in this specification.

Settings MUST be stored directly on the image so deleting the image also deletes its settings.

Unknown namespaced attributes can be read and written through `inkex`.

## 10.3 Generated group attributes

The generated root group MUST contain:

```xml
pt:generated="true"
pt:source-image-uuid="..."
pt:settings-hash="..."
pt:source-hash="..."
pt:backend-id="..."
pt:backend-version="..."
pt:schema-version="1"
```

It MUST NOT duplicate the full image settings.

## 10.4 Generated layer structure

Suggested structure:

```xml
<g
  id="palette-trace-result-..."
  inkscape:label="Palette Trace — Source image name"
  pt:generated="true"
  pt:source-image-uuid="...">

  <g
    id="palette-trace-scan-..."
    inkscape:label="01 Background — #F5E8D0"
    pt:palette-entry-id="..."
    pt:role="background">
    <path ... />
  </g>

  <g
    id="palette-trace-scan-..."
    inkscape:label="02 Blue fill — #245FA8"
    pt:palette-entry-id="..."
    pt:role="fill">
    <path ... />
  </g>
</g>
```

The outer result group SHOULD be inserted immediately above the source image in the same parent.

The source image MUST remain unchanged unless the user explicitly selects **Hide source image after tracing**.

Deleting the source image MUST NOT automatically delete generated vectors. The generated vectors then become ordinary, non-retraceable artwork.

## 10.5 Standalone sidecar

When running under the standalone host, the settings object defined in §11 is stored in the sidecar file described in §9.4.3 instead of in `pt:settings`.

The sidecar MUST be a single JSON object with the same fields, and MUST NOT introduce a parallel schema. `schemaVersion` and the validation rules in §12 apply unchanged.

The `pt:` attributes in §10.3 remain required on the generated group in both hosts, because they describe the generated result rather than the host.

---

# 11. Canonical image-settings schema

The following TypeScript-style definition is normative for field names, types and structure. The repository MUST additionally contain an equivalent JSON Schema document.

```ts
type UUID = string;
type HexColor = string; // "#RRGGBB" only in schema version 1

interface PaletteTraceImageSettingsV1 {
  schemaVersion: 1;
  extensionVersion: string;

  imageUuid: UUID;

  source: SourceDescriptor;
  destination: DestinationSettings;

  scanCount: number; // integer, 1..64 in MVP

  colorProcessing: ColorProcessingSettings;
  palette: PaletteSettings;
  alpha: AlphaSettings;

  globalTraceProfile: TraceProfile;
  geometry: GeometrySettings;
  backend: BackendPreference;

  output: OutputSettings;
  generation: GenerationState;
  uiState?: PersistedUiState;
}

interface SourceDescriptor {
  sourceMode: "intrinsic";
  fingerprint: SourceFingerprint;
  intrinsicWidth: number;
  intrinsicHeight: number;
  mimeType: string;

  traceClipEffects: false;
  traceSvgFilters: false;
}

interface SourceFingerprint {
  algorithm: "sha256";
  value: string;
  linkedFileModifiedTime?: string;
}

interface DestinationSettings {
  id:
    | "illustration"
    | "logo_branding"
    | "screen_printing"
    | "vinyl_paper"
    | "laser"
    | "custom";

  presetVersion: number;
}

interface ColorProcessingSettings {
  inputSpace: "srgb";
  comparisonSpace: "oklch";
  quantizer: "deterministic_constrained_kmeans";

  automaticPalette: AutomaticPaletteSettings;
  unmatchedPixelPolicy: "nearest_available" | "drop";

  reachMappingVersion: 1;
  lowChromaHuePolicyVersion: 1;
}

interface AutomaticPaletteSettings {
  histogramBitsPerChannel: 5 | 6 | 7 | 8;
  maxHistogramEntries: number;
  maxIterations: number;
  convergenceThreshold: number;
  minimumSeparation: number;
}

interface PaletteSettings {
  entries: PaletteEntry[];
  layerOrder: UUID[];
  backgroundEntryId: UUID | null;
}

interface PaletteEntry {
  id: UUID;
  name: string;
  enabled: boolean;

  kind: "automatic" | "pinned";

  sourceAnchor: ColorDefinition | null;
  output: OutputColorDefinition;

  assignment: AssignmentSettings;
  role: PaletteRole;

  traceProfile:
    | {
        mode: "inherit";
      }
    | {
        mode: "preset";
        profileId: TraceProfilePresetId;
      }
    | {
        mode: "override";
        // Rebases onto a named preset before merging, or onto the document's
        // global profile when omitted (§18).
        profileId?: TraceProfilePresetId;
        // A *partial* profile: only the mask/vector keys present here are
        // replaced over the base. See §18 for why this is a merge, not a
        // full replacement.
        values: Partial<TraceProfile>;
      };

  operation?: OperationRole;
}

interface ColorDefinition {
  srgb: HexColor;
}

interface OutputColorDefinition {
  mode: "exact" | "automatic_centroid";
  color: ColorDefinition | null;
}

interface AssignmentSettings {
  mode:
    | "reserve_within_reach"
    | "fixed_cluster_center"
    | "automatic";

  overallReach: number; // integer, 0..100

  channels: ReachChannelSettings;
}

interface ReachChannelSettings {
  mode: "linked" | "custom";

  hue: ChannelTolerance;
  chroma: ChannelTolerance;
  lightness: ChannelTolerance;
}

interface ChannelTolerance {
  enabled: boolean;
  tolerance: number;
  weight: number;
}

type PaletteRole =
  | "background"
  | "outline"
  | "primary_fill"
  | "secondary_fill"
  | "accent"
  | "highlight"
  | "shadow"
  | "operation"
  | "custom";

type OperationRole =
  | "none"
  | "cut"
  | "engrave"
  | "score"
  | "ignore";

type TraceProfilePresetId =
  | "default"
  | "smooth_shapes"
  | "sharp_details"
  | "thin_line_art"
  | "small_accents"
  | "simplified_background"
  | "fabrication_clean";

interface TraceProfile {
  mask: MaskCleanupSettings;
  vector: VectorTraceSettings;
}

interface MaskCleanupSettings {
  minimumRegionAreaPx2: number;
  fillHolesAreaPx2: number;
  closeGapsRadiusPx: number;
  smoothingRadiusPx: number;

  preserveThinFeatures: boolean;
  minimumFeatureWidthPx: number;

  offsetPx: number;
}

interface VectorTraceSettings {
  cornerSensitivity: number; // 0..1
  curveSmoothing: number; // 0..1
  optimization: number; // 0..1

  requireClosedPaths: boolean;
  minimumPathAreaPx2: number;

  backendOptions?: Record<string, unknown>;
}

interface GeometrySettings {
  policy:
    | "stacked"
    | "stacked_trapped"
    | "exclusive_layers"
    | "separate_operations";

  underlap: Measurement;
  trapping: Measurement;

  preserveOuterSilhouette: boolean;
  validateClosedPaths: boolean;
  detectDuplicateGeometry: boolean;
}

interface Measurement {
  value: number;
  unit: "source_px" | "document_unit" | "mm";
}

interface AlphaSettings {
  fullyTransparentPolicy: "ignore";
  partialAlphaPolicy: "composite";
  ignoreBelow: number; // 0..1

  matte:
    | { mode: "background_entry" }
    | { mode: "white" }
    | { mode: "black" }
    | { mode: "custom"; color: HexColor };

  outputOpacity: "opaque";
}

interface BackendPreference {
  preferredBackendId: string | "auto";
  requireExactBackend: boolean;
  options: Record<string, unknown>;
}

interface OutputSettings {
  updateExistingResult: boolean;
  hideSourceImageAfterApply: boolean;

  backgroundOutput:
    | "keep_paths"
    | "omit"
    | "replace_with_rectangle";

  groupScans: true;
  labelLayers: true;
}

interface GenerationState {
  lastGeneratedGroupId: string | null;
  lastSourceHash: string | null;
  lastSettingsHash: string | null;

  lastBackendId: string | null;
  lastBackendVersion: string | null;
}

interface PersistedUiState {
  selectedPaletteEntryId: UUID | null;

  previewMode:
    | "source"
    | "quantized"
    | "selected_claim"
    | "masks"
    | "vector"
    | "production";

  zoom: number;
  panX: number;
  panY: number;
}
```

---

# 12. Schema validation rules

The implementation MUST enforce all of the following:

1. `schemaVersion` MUST equal `1`.
2. `scanCount` MUST be between 1 and 64.
3. `palette.entries.length` MUST equal `scanCount`.
4. Palette entry IDs MUST be unique.
5. Every ID in `layerOrder` MUST identify exactly one palette entry.
6. Every enabled palette entry MUST appear exactly once in `layerOrder`.
7. The number of pinned entries MUST NOT exceed `scanCount`.
8. Automatic entries MUST use:

   * `assignment.mode = "automatic"`
   * `output.mode = "automatic_centroid"`
9. Pinned entries MUST have a non-null source anchor.
10. Pinned entries with exact output MUST have a non-null output colour.
11. A newly picked colour MUST use:

    * `kind = "pinned"`
    * `assignment.mode = "reserve_within_reach"`
    * `output.mode = "exact"`
12. Only one entry MAY have the role `background`.
13. `backgroundEntryId` MUST match that entry or be null.
14. Hue tolerance MUST be measured in degrees from 0 through 180.
15. Chroma tolerance MUST be in OKLCH chroma units from 0 through 0.4.
16. Lightness tolerance MUST be from 0 through 1.
17. Channel weights MUST be finite and non-negative.
18. All numeric values MUST be finite.
19. Backend-specific options MUST be namespaced by backend ID.
20. Unknown schema-version-1 fields SHOULD be preserved when reading and writing.
21. Invalid image settings MUST NOT crash the extension.
22. Invalid settings MUST be backed up in memory for diagnostic reporting, then replaced only after user confirmation.

---

# 13. Colour-reach mapping

## 13.1 Overall reach

`overallReach` is an integer from 0 through 100.

When `channels.mode = "linked"`, the three channel tolerances MUST be calculated from a versioned mapping table.

Initial mapping anchors:

| Reach | Label      |  Hue | Chroma | Lightness |
| ----: | ---------- | ---: | -----: | --------: |
|     0 | Exact      | 0.5° |  0.002 |     0.002 |
|    25 | Narrow     |  10° |  0.035 |     0.060 |
|    50 | Similar    |  25° |  0.080 |     0.160 |
|    75 | Broad      |  60° |  0.160 |     0.350 |
|   100 | Aggressive | 180° |  0.400 |     1.000 |

Values between anchors MUST use deterministic linear interpolation.

The mapping MUST exist in a versioned data file rather than hard-coded throughout the UI and engine.

Suggested file:

```text
palette_trace/data/reach_mapping_v1.json
```

## 13.2 Advanced channel controls

When `channels.mode = "custom"`, the user may control:

* Hue tolerance.
* Saturation/chroma tolerance.
* Lightness tolerance.
* Whether each channel participates.
* Channel weight.

The interface SHOULD label chroma as:

> Saturation / chroma

The implementation MUST use OKLCH chroma internally, not HSL saturation.

## 13.3 Hue wrapping

Hue distance MUST use the shortest circular difference:

```text
difference = abs(h1 - h2)
hueDistance = min(difference, 360 - difference)
```

## 13.4 Low-chroma hue handling

Hue becomes unreliable for nearly grey colours.

Required policy:

* If a pinned source anchor has chroma below `0.02`, hue matching MUST default to disabled.
* For a source pixel with chroma below `0.05`, the hue contribution to matching score MUST be reduced proportionally.
* Hue MUST NOT prevent an otherwise valid match for a nearly neutral pixel.
* The UI SHOULD display “Hue has little effect for this neutral colour” when appropriate.

Suggested hue-confidence function:

```text
hueConfidence = clamp(sourceChroma / 0.05, 0, 1)
```

Effective hue weight:

```text
effectiveHueWeight = configuredHueWeight × hueConfidence
```

---

# 14. Pixel-claim algorithm

For every source pixel not excluded by transparency or background policy:

## 14.1 Find reserved candidates

A pinned entry using `reserve_within_reach` is eligible when every enabled hard-tolerance condition passes.

For channel `i`:

```text
distance_i <= tolerance_i
```

Disabled channels do not participate.

## 14.2 Calculate normalized candidate score

For eligible candidates:

```text
score =
    sum(weight_i × (distance_i / tolerance_i)²)
    ------------------------------------------------
                sum(enabled weight_i)
```

For a zero tolerance, use a documented numeric epsilon and require an effectively exact match.

## 14.3 Resolve conflicts

When multiple pinned entries claim one pixel:

1. Lowest normalized score wins.
2. If scores are equal within epsilon, the entry earlier in explicit claim priority wins.
3. If no explicit claim priority exists, palette-entry UUID lexical order is the deterministic final tie-breaker.

Palette layer order MUST NOT determine colour ownership.

## 14.4 Unclaimed pixels

Unclaimed pixels proceed to automatic quantization.

If no automatic scans exist:

* `nearest_available` assigns each unclaimed pixel to the nearest enabled pinned palette entry.
* `drop` leaves the pixel unassigned and transparent in the output.

The default MUST be `nearest_available`.

## 14.5 Overlapping claims

One pixel MUST belong to no more than one scan in the MVP.

An “allow overlap” classification mode is future work.

---

# 15. Deterministic automatic palette generation

Automatic palette generation MUST be deterministic.

It MUST NOT depend on an unseeded random initializer.

## 15.1 Inputs

The quantizer receives:

* Unclaimed pixels.
* Their frequency or weight.
* Fixed-centre pinned entries, when present.
* Requested number of automatic entries.
* Minimum palette separation.
* Maximum iterations.
* Convergence threshold.

## 15.2 Histogram reduction

For performance, pixels SHOULD first be collapsed into a weighted colour histogram.

Default:

```text
6 bits per sRGB channel
Maximum histogram entries: 32,768
```

Transparent and ignored pixels MUST be excluded.

## 15.3 Colour space

Clustering MUST occur in OKLab or OKLCH-derived Cartesian coordinates.

Hue-angle values MUST NOT be averaged directly.

## 15.4 Deterministic initialization

Recommended process:

1. Begin with any fixed-centre pinned entries.
2. Select the most frequent remaining colour as the first automatic centre.
3. Select each next centre using weighted farthest-point initialization.
4. Break ties by packed sRGB numeric value.
5. Never move fixed centres.
6. Recalculate automatic centres until convergence or iteration limit.

## 15.5 Minimum separation

If two automatic centres are closer than the configured minimum separation:

1. Merge the lower-weight centre into the higher-weight centre.
2. Reseed the empty centre using the weighted farthest remaining colour.
3. If no sufficiently distinct colour remains, reduce the effective output scan count.
4. Display a warning instead of manufacturing a redundant colour.

## 15.6 Automatic output colours

Automatic output colours MUST be calculated from weighted cluster centroids and converted to in-gamut sRGB.

Automatic colour generation MUST consider pinned centres so it does not waste scans on colours nearly identical to pinned colours.

---

# 16. Background handling

One palette entry MAY be designated as background.

## 16.1 Matching modes

The MVP MUST support:

* All matching pixels.
* Edge-connected matching pixels.
* Transparent pixels.

For edge-connected background matching:

1. Build the matching mask.
2. Start flood-fill components from image boundaries.
3. Treat only connected qualifying pixels as background.
4. Preserve enclosed regions of the same colour.

## 16.2 Output modes

* `keep_paths`
* `omit`
* `replace_with_rectangle`

For rectangle replacement:

* Use the complete intrinsic source-image bounds.
* Apply the same source-to-document transform as other generated geometry.
* Use the background entry’s exact output colour.

## 16.3 Background and locking

Background is a role, not a separate palette type.

A background entry MAY be automatic or pinned.

---

# 17. Mask representation and cleanup

## 17.1 Label map

The primary classification result MUST be stored as one integer label map, not as one full RGBA image per scan.

Recommended representation:

* `uint8` for up to 255 labels.
* `uint16` if future scan limits exceed 255.

This reduces memory use and ensures each pixel has one owner.

Binary masks SHOULD be generated lazily per scan.

## 17.2 Cleanup order

For stacked or independent scan masks:

1. Remove small connected components.
2. Fill small holes.
3. Close small gaps.
4. Apply mask smoothing.
5. Restore protected thin features when enabled.
6. Apply destination geometry operations such as underlap or trapping.
7. Pass final mask to tracing backend.

## 17.3 Speckle removal

`minimumRegionAreaPx2` is measured in source-image pixels squared.

A connected component smaller than the threshold is removed.

For exclusive-layer geometry, removed pixels MUST be reassigned to the neighbouring label with the greatest shared boundary, unless the destination explicitly allows transparent holes.

## 17.4 Preview scaling

When preview dimensions differ from source dimensions:

* Linear dimensions MUST scale by the preview scale factor.
* Area dimensions MUST scale by the square of the preview scale factor.

A preview MUST NOT apply a full-resolution 16-pixel-area threshold as 16 preview pixels.

## 17.5 Morphology and offsets

`offsetPx` means:

* Positive: expand mask.
* Negative: contract mask.
* Zero: unchanged.

Per-scan offsets MUST be disabled or clearly warned in exclusive-layer mode because they can create overlap or gaps.

---

# 18. Trace profiles

A per-entry `traceProfile` has three modes: `inherit`, `preset` (naming a
built-in profile by `profileId`), and `override`. `override` merges an
explicit `values` object over a base profile — the document's global profile
by default, or a named preset when `profileId` is also given — replacing only
the mask/vector keys the user actually touched.

This is a *partial* merge, not a full-profile replacement, and the field is
named `override` rather than `custom` for that reason: a mode called `custom`
that required restating every one of the twelve `TraceProfile` fields would
be worse UI (the editor would need to force a value onto every field, not
just the ones a user actually wants to change) and worse for forward
compatibility (a future field added to `TraceProfile` would silently vanish
from every entry that had already gone fully custom, instead of inheriting
the new default the way an untouched field does today). `profileId` replaces
the earlier `presetId` name so the same field name is used in both the
`preset` and `override` branches, which name the same kind of value. The
granularity of what a user can override is every individual field the
`TraceProfile` type defines (`palette_trace/presets/profiles.py::merge_profiles`
merges key-by-key within `mask` and `vector`), not a section-level or
profile-level choice — the per-entry override editor exposes the same field
set as the global profile editor for exactly that reason.

Trace-profile definitions MUST live in versioned data files.

Suggested file:

```text
palette_trace/data/trace_profiles_v1.json
```

Initial defaults:

| Profile               | Region area | Mask smoothing | Corners | Curve smoothing | Optimization |
| --------------------- | ----------: | -------------: | ------: | --------------: | -----------: |
| Default               |       4 px² |         0.5 px |    0.65 |            0.50 |         0.20 |
| Smooth shapes         |       8 px² |         1.2 px |    0.35 |            0.80 |         0.65 |
| Sharp details         |      12 px² |         0.2 px |    0.90 |            0.15 |         0.15 |
| Thin line art         |       2 px² |         0.3 px |    0.85 |            0.40 |         0.25 |
| Small accents         |       1 px² |        0.15 px |    0.80 |            0.20 |         0.10 |
| Simplified background |      64 px² |         1.5 px |    0.25 |            0.90 |         0.80 |
| Fabrication clean     |      16 px² |         0.4 px |    0.75 |            0.35 |         0.60 |

These values are initial tuning values, not engine-native arguments.

Backend adapters MUST map them to backend-specific parameters.

Unsupported settings MUST NOT be silently ignored.

An adapter MUST either:

* Map the setting.
* Emulate the setting in preprocessing or post-processing.
* Report the setting as unsupported.
* Disable the control and explain why.

---

# 19. Destination presets

Built-in destination presets MUST be immutable and versioned.

Suggested file:

```text
palette_trace/data/destination_presets_v1.json
```

## 19.1 Illustration

Purpose:

* General vector artwork.
* Layered visual reproduction.
* Independent treatment of scans.

Defaults:

```text
Geometry: stacked
Underlap: 0.5 source px
Per-scan profiles: fully enabled
Exact picked colours: enabled
Closed-path validation: optional
Source image hidden after trace: no
```

## 19.2 Logo / branding

Purpose:

* Clean, low-node, exact-colour artwork.
* Restoration of brand palettes.

Defaults:

```text
Geometry: stacked
Underlap: 0.25 source px
Global profile: smooth shapes
Optimization: medium-high
Exact picked colours: enabled
Minimum palette separation: increased
Closed-path validation: enabled
```

## 19.3 Screen printing

Purpose:

* Separate colour layers with controlled trapping.

Defaults:

```text
Geometry: stacked_trapped
Trapping: 0.25 mm
Exact picked colours: enabled
One named group per colour
Background omission: suggested
Minimum feature warning: enabled
```

Future screen-print features:

* Registration marks.
* Spot-colour naming.
* Ink-order preview.
* Underbase generation.
* Knockout and overprint simulation.

## 19.4 Vinyl / paper cutting

Purpose:

* Separate closed colour shapes.
* Avoid unnecessary overlap and tiny islands.

Defaults:

```text
Geometry: exclusive_layers
Underlap: none
Global profile: fabrication clean
Closed paths: required
Minimum feature validation: enabled
Duplicate-geometry detection: enabled
```

The MVP MAY produce raster-exclusive rather than mathematically shared boundaries.

The interface MUST clearly state:

> Colour ownership is exclusive at source-pixel resolution. Adjacent vector boundaries may not be topologically identical.

Future work may add shared-boundary planar geometry.

## 19.5 Laser

Purpose:

* Prepare simplified paths grouped by operation.

Defaults:

```text
Geometry: separate_operations
Global profile: fabrication clean
Closed-path validation: enabled
Tiny-region removal: aggressive
Operation role controls: enabled
```

Operation roles:

* Cut
* Engrave
* Score
* Ignore

The extension MUST NOT generate machine commands or assume that one SVG colour has universal meaning across laser software.

## 19.6 Custom

No destination-specific restrictions beyond schema validity.

---

# 20. Geometry policies

## 20.1 Stacked

Scans are traced independently and arranged from bottom to top.

Underlap implementation:

```text
expanded lower mask =
    dilate(lower mask, underlap radius)
    intersect
    union of all classified subject pixels
```

This MUST prevent underlap from expanding the exterior silhouette unless `preserveOuterSilhouette` is false.

## 20.2 Stacked trapped

Similar to stacked, but the configured trapping measurement is used.

The output MUST retain one named group per scan.

## 20.3 Exclusive layers

At the raster classification level:

* Every included pixel belongs to exactly one label.
* Morphological cleanup SHOULD operate on the label map.
* Removed components SHOULD be reassigned rather than discarded.
* Per-scan offsets SHOULD be disabled.
* Path smoothing SHOULD be conservative.

The MVP does not promise identical shared Bézier boundaries.

## 20.4 Separate operations

Every palette entry is grouped according to operation role.

The extension MUST validate:

* Closed paths for cuts.
* Very small paths.
* Duplicate paths.
* Self-intersections where detectable.
* Empty operation layers.

---

# 21. Alpha and colour management

## 21.1 Working colour space

MVP pipeline:

```text
Decoded source
→ honour embedded profile when available
→ convert to sRGB
→ composite partial alpha
→ convert to OKLab/OKLCH for comparison
→ generate opaque sRGB SVG output
```

## 21.2 Transparency

* Fully transparent pixels MUST be ignored.
* Pixels below `ignoreBelow` MUST be ignored.
* Partially transparent pixels MUST be composited against a selected matte.
* Continuous output opacity is not supported in schema version 1.
* Generated paths MUST be opaque.

## 21.3 Matte default

Priority:

1. Selected background palette entry.
2. User-selected custom matte.
3. White.

The preview MUST visibly indicate the active matte.

## 21.4 ICC limitations

If colour-profile conversion is unavailable:

* Assume sRGB.
* Show a non-blocking warning.
* Record the limitation in diagnostics.

## 21.5 Future alpha work

Future versions MAY support:

* Alpha-band scans.
* SVG masks.
* Edge-colour decontamination.
* Reconstruction of foreground colours from a known matte.

---

# 22. Source-image handling

## 22.1 Embedded images

Decode embedded data without modifying the source element.

## 22.2 Linked local images

Resolve paths relative to the SVG document location.

If the SVG document is unsaved and the image path is relative, the extension MUST report that the source cannot be resolved.

## 22.3 Transform mapping

Tracing MUST occur in intrinsic source-pixel coordinates.

The output transform MUST be:

```text
intrinsic pixel coordinates
→ image viewport defined by x, y, width, height and preserveAspectRatio
→ image transform
→ parent document coordinates
```

The extension MUST not raster-resample the image merely because it is rotated or scaled in the SVG.

## 22.4 Clips, masks and SVG filters

MVP traces intrinsic raster data.

If the source image has an SVG clip, mask or filter:

* Display a warning.
* Continue only after user acknowledgement.
* Do not claim that the rendered appearance will be traced.

A future `rendered_source` mode MAY rasterize the displayed result through Inkscape.

## 22.5 EXIF orientation

Image decoding SHOULD honour EXIF orientation before fingerprinting and tracing.

---

# 23. Tracing-backend architecture

## 23.1 Mandatory interface

```python
from dataclasses import dataclass
from typing import Protocol

@dataclass(frozen=True)
class BackendCapabilities:
    backend_id: str
    version: str

    supports_binary_masks: bool
    supports_holes: bool
    supports_cancellation: bool
    deterministic: bool

    supported_canonical_settings: frozenset[str]

@dataclass(frozen=True)
class TraceRequest:
    width: int
    height: int
    packed_binary_mask: bytes
    profile: dict
    transform_hint: tuple[float, float, float, float, float, float] | None

@dataclass(frozen=True)
class TraceResult:
    svg_path_data: tuple[str, ...]
    fill_rule: str
    warnings: tuple[str, ...]
    statistics: dict

class TraceBackend(Protocol):
    def capabilities(self) -> BackendCapabilities:
        ...

    def trace_mask(
        self,
        request: TraceRequest,
        cancellation_token: object,
    ) -> TraceResult:
        ...
```

## 23.2 Backend responsibilities

A backend MUST:

* Accept one binary mask.
* Return SVG path data or an equivalent normalized path representation.
* Preserve holes.
* Report errors.
* Report unsupported options.
* Produce deterministic output under identical inputs and settings.
* Avoid changing palette colours.

A backend MUST NOT:

* Requantize the colour image.
* Select output colours.
* Merge scans based on colour.
* Reorder layers.
* apply destination rules.

## 23.3 Backend discovery

Recommended priority:

1. Explicit backend selected by user.
2. Bundled portable backend.
3. Compatible installed Python backend.
4. Compatible discovered CLI backend.
5. Experimental Inkscape-action backend.
6. Failure with actionable installation guidance.

## 23.4 Portable backend preference

The preferred portable implementation is:

* A compatible open-source tracing engine compiled to WebAssembly and bundled with the local interface, or
* A portable Python binding with maintained wheels for supported platforms.

WebAssembly is preferred when it:

* Runs without network access.
* Accepts binary masks.
* Returns path geometry.
* Provides adequate quality.
* Has a compatible open-source licence.
* Has reproducible build instructions.
* Can be bundled in compliance with its licence.

## 23.5 Candidate engines

Potential adapters include:

* VTracer.
* Potrace.
* AutoTrace.
* A future Inkscape-native mask tracing API.
* Another third-party open-source tracer.

VTracer currently exposes binary tracing and Python integration and describes a pluggable pipeline, making it a strong candidate for the required engine-evaluation spike.

AutoTrace is another open-source bitmap-vectorization engine already represented in Inkscape’s tracing architecture.

## 23.6 Engine-selection spike

Before completing the full UI, the coding agent MUST implement a backend conformance harness and evaluate at least:

* One portable/WASM or Python candidate.
* One CLI or native candidate.

Each candidate MUST be tested for:

* Binary-mask input.
* Hole preservation.
* Sharp corners.
* Smooth curves.
* Small components.
* Large masks.
* Determinism.
* Cancellation.
* Runtime.
* Node count.
* Licence compatibility.
* Windows, macOS and Linux distribution feasibility.

The engine that best satisfies the conformance tests SHOULD become the reference backend.

The rest of the extension MUST remain backend-neutral.

## 23.7 Inkscape CLI adapter

An Inkscape CLI adapter MAY be included as experimental or fallback functionality.

It MUST:

* Use `inkex.command`.
* Capability-detect the required actions.
* Batch masks in one invocation where possible.
* Never launch one process per scan.
* Report that the current `object-trace` action exposes only one global colour-trace configuration and may not provide full parity with prepared binary-mask tracing.

---

# 24. Processing pipeline

## Stage 1: Extension startup

1. Parse SVG.
2. Validate one selected image or linked generated group.
3. Resolve source image.
4. Read image settings.
5. Validate and migrate settings.
6. Fingerprint source.
7. Detect available backends.
8. Start local interface session.

## Stage 2: Source decoding

1. Decode raster through Pillow or equivalent.
2. Honour orientation.
3. Convert embedded profile to sRGB when possible.
4. Preserve intrinsic dimensions.
5. Compute alpha policy.
6. Build full-resolution source representation.
7. Build preview proxy.

## Stage 3: Preview proxy

The preview longest dimension SHOULD default to 1024 pixels.

The preview MUST retain:

* Correct aspect ratio.
* Colour conversion.
* Matte behaviour.
* Deterministic sampling.
* Scaled cleanup thresholds.

## Stage 4: Palette initialization

For a new image:

1. Create `scanCount` automatic entries with stable UUIDs.
2. Run deterministic automatic quantization.
3. Sort only the initial presentation by a documented rule.
4. Keep layer order independently editable.

Suggested initial order:

* Background candidate first when explicitly selected.
* Remaining entries by descending source-pixel population.
* Do not silently reorder after user changes layer order.

## Stage 5: Pinned-colour claim

1. Evaluate reserved pinned entries.
2. Resolve conflicting claims.
3. Mark claimed pixels in label map.
4. Display claimed percentage per pinned entry.
5. Warn if a pinned entry claims zero pixels.
6. Warn if one entry claims an unusually large percentage.

## Stage 6: Automatic quantization

1. Use only unclaimed pixels.
2. Include fixed-centre pinned entries where configured.
3. Calculate remaining automatic centres.
4. Assign remaining pixels.
5. Collapse indistinguishable entries when necessary.
6. Update automatic swatches.

## Stage 7: Background classification

Apply the selected background connectivity policy.

## Stage 8: Label-aware cleanup

For exclusive geometry, operate on the label map.

For stacked geometry, derive individual masks and apply per-scan cleanup.

## Stage 9: Destination geometry

Apply:

* Underlap.
* Trapping.
* Exclusive-layer constraints.
* Operation grouping.

## Stage 10: Vector tracing

For each enabled non-empty scan:

1. Build packed binary mask.
2. Resolve inherited trace profile.
3. Call backend adapter.
4. Normalize returned paths.
5. Apply exact output fill.
6. Collect statistics and warnings.

This stage SHOULD process scans incrementally and release masks when no longer needed.

## Stage 11: SVG assembly

1. Create result root group.
2. Create one labelled group per scan.
3. Insert normalized paths.
4. Apply source-to-document transform.
5. Add provenance attributes.
6. Preserve explicit layer order.
7. Apply background output policy.

## Stage 12: Validation

Destination-specific validation MUST run before Apply completes.

## Stage 13: Atomic commit

1. Build result in memory or a temporary document fragment.
2. Do not modify the live output tree until all required tracing succeeds.
3. Replace or create generated group.
4. Update source-image settings.
5. Return modified SVG.
6. Ensure the complete operation appears as one extension action in Inkscape.

---

# 25. Caching and performance

## 25.1 Cache boundaries

Cache:

* Decoded source.
* sRGB conversion.
* OKLab/OKLCH conversion.
* Histogram.
* Preview proxy.
* Pixel claims.
* Label map.
* Per-scan cleaned masks.
* Per-scan vector results.

## 25.2 Cache invalidation

Changing:

* Output colour only: MUST NOT reclassify pixels unless source anchor also changes.
* Vector optimization: MUST retrace only that scan.
* Mask cleanup: MUST rebuild and retrace only that scan where geometry permits.
* Colour reach: MUST reclassify pixels and update affected automatic palette.
* Scan count: MUST rerun automatic palette generation.
* Destination geometry: MUST reuse classification where possible.
* Matte: MUST invalidate colour conversion and all downstream stages.
* Source image: MUST invalidate everything.

## 25.3 Memory

The extension MUST NOT allocate one full-resolution RGBA image per scan.

It SHOULD retain:

* One decoded source.
* One converted working image.
* One label map.
* One or a small number of active binary masks.
* Cached compressed preview data.

## 25.4 Interaction

* Slider input MUST be debounced.
* Obsolete preview jobs MUST be cancellable or ignored by generation ID.
* UI MUST show progress for full-resolution tracing.
* Apply and Cancel MUST remain available.
* A failed scan MUST not silently disappear.

---

# 26. Saved-preset schema

```ts
interface PaletteTraceUserPresetV1 {
  schemaVersion: 1;
  presetUuid: UUID;

  name: string;
  description: string;

  createdAt: string;
  updatedAt: string;

  scope: "structure" | "palette" | "full";

  includes: {
    destination: boolean;
    scanCount: boolean;
    geometry: boolean;
    globalTraceProfile: boolean;
    perScanProfiles: boolean;
    paletteRoles: boolean;
    matchingSettings: boolean;
    exactPaletteColors: boolean;
    outputSettings: boolean;
  };

  configurationPatch: Record<string, unknown>;
}
```

## 26.1 Structure preset

Typically includes:

* Destination.
* Scan count.
* Roles.
* Trace profiles.
* Geometry.
* Layer order pattern.

It does not include exact colours.

## 26.2 Palette preset

Includes:

* Exact colours.
* Source anchors.
* Colour reaches.
* Roles.
* Output colours.

Useful for brand palettes or recurring fabrication workflows.

## 26.3 Full preset

Includes all reusable settings except source-specific identity and generation state.

## 26.4 Storage

User presets SHOULD be stored as individual JSON files in a Palette Trace user-configuration directory.

The UI MUST support:

* Save.
* Rename.
* Duplicate.
* Delete.
* Import.
* Export.

Preset files MUST contain data only and MUST NOT execute code.

---

# 27. Source changes and fingerprints

The extension MUST store a SHA-256 fingerprint of decoded, orientation-corrected source content or another stable canonical representation.

When reopening:

## Fingerprint unchanged

Restore settings normally.

## Fingerprint changed

Display:

> The source bitmap has changed since this Palette Trace configuration was last applied.

Options:

* Recalculate using existing settings.
* Reset automatic palette while preserving pinned colours.
* Start from destination defaults.
* Cancel.

## Missing source

Display:

> The linked source bitmap cannot be found.

Options:

* Locate replacement.
* Keep existing vectors unchanged.
* Cancel.

The extension MUST NOT silently trace a different file with the same filename.

---

# 28. Generated-result updates

When `updateExistingResult` is true:

1. Find the generated group referenced by `lastGeneratedGroupId`.
2. Confirm its `source-image-uuid` matches.
3. Replace generated scan groups atomically.
4. Preserve the outer group’s position in the document.
5. Warn if the generated group appears to have been manually edited.

Manual-edit detection MAY use:

* Stored generated-content hash.
* Unexpected child elements.
* Missing scan identifiers.
* Changed path hashes.

Warning:

> Retracing will replace generated paths and may discard manual edits inside this Palette Trace group.

The user MUST be able to choose:

* Replace existing result.
* Create a new result group.
* Cancel.

---

# 29. Accessibility requirements

The interface MUST meet WCAG 2.1 AA principles where applicable.

Required:

* All controls have visible labels.
* All form fields are keyboard operable.
* Colour is never the only status indicator.
* Palette rows include names, values and roles.
* Slider values are displayed numerically.
* Dynamic status uses accessible live regions.
* Focus order follows visual order.
* Focus remains visible.
* Drag-and-drop actions have button alternatives.
* Preview modes have textual descriptions.
* Mask overlays use patterns, outlines or labels in addition to colour.
* Error messages identify the affected control.
* The application supports browser zoom.
* No essential interaction depends on hover.

Generated scan groups SHOULD include SVG `<title>` or `<desc>` metadata where useful.

---

# 30. Error handling

The extension MUST handle:

* No selection.
* Multiple images selected.
* Unsupported selection.
* Missing linked image.
* Unsupported image type.
* Corrupt image.
* Invalid image settings.
* Unsupported settings schema.
* Backend unavailable.
* Backend crash.
* Backend timeout.
* Empty scan.
* Too many pinned colours.
* Failed colour-profile conversion.
* Insufficient memory.
* Temporary-directory failure.
* Browser launch failure.
* Local-server port failure.
* Session-token mismatch.
* User cancellation.

Errors MUST be actionable and MUST NOT expose a Python traceback as the only explanation.

A diagnostics view SHOULD include:

* Extension version.
* Inkscape version.
* Python version.
* Operating system.
* Available backend IDs and versions.
* Source dimensions and type.
* Settings schema version.
* Sanitized error details.

---

# 31. Security requirements

* No source image may be uploaded.
* No telemetry by default.
* No remote dependencies.
* No use of `eval`.
* No execution of preset content.
* No shell command interpolation.
* External commands MUST use safe argument arrays.
* Temporary files MUST use secure random names.
* Temporary directory permissions SHOULD be restricted to the current user.
* Local server MUST bind only to loopback.
* Session token MUST contain at least 128 bits of entropy.
* Server MUST accept only the active session.
* Server MUST stop after completion.
* File paths returned by browser code MUST be treated as untrusted.
* SVG fragments returned by a backend MUST be parsed and sanitized.
* Scripts, event attributes, external references and foreign objects from backend SVG output MUST be removed.
* Only supported path geometry and required metadata may be imported.

---

# 32. Repository structure

```text
palette-trace/
├── LICENSE
├── README.md
├── CHANGELOG.md
├── CONTRIBUTING.md
├── THIRD_PARTY_NOTICES.md
├── palette_trace.inx
├── palette_trace.py          # Inkscape host entry point
├── pyproject.toml
├── schemas/
│   ├── image-settings-v1.schema.json
│   └── user-preset-v1.schema.json
├── palette_trace/
│   ├── __init__.py
│   ├── extension.py
│   ├── standalone.py         # standalone host entry point (§9.4)
│   ├── settings.py           # host-neutral settings schema (§11)
│   ├── errors.py
│   ├── diagnostics.py
│   ├── capabilities.py
│   ├── svg_writer.py         # standalone SVG assembly (§9.4.4)
│   ├── sidecar.py            # standalone settings persistence (§9.4.3)
│   ├── image_source.py       # decoding, EXIF, OKLCH, fingerprint — host-neutral
│   ├── document/             # Inkscape host only — the sole `inkex` consumer
│   │   ├── selection.py
│   │   ├── transforms.py
│   │   ├── settings_store.py
│   │   ├── generated_groups.py
│   │   └── svg_sanitizer.py
│   ├── color/
│   │   ├── conversion.py
│   │   ├── reach.py
│   │   ├── claims.py
│   │   ├── quantizer.py
│   │   ├── histogram.py
│   │   └── background.py
│   ├── masks/
│   │   ├── label_map.py
│   │   ├── components.py
│   │   ├── morphology.py
│   │   ├── thin_features.py
│   │   └── geometry_policy.py
│   ├── tracing/
│   │   ├── protocol.py
│   │   ├── registry.py
│   │   ├── normalization.py
│   │   ├── backends/
│   │   │   ├── vtracer_adapter.py
│   │   │   ├── potrace_adapter.py
│   │   │   ├── autotrace_adapter.py
│   │   │   └── inkscape_cli_adapter.py
│   │   └── conformance/
│   │       ├── fixtures/
│   │       └── runner.py
│   ├── presets/
│   │   ├── destination.py
│   │   ├── user_presets.py
│   │   └── migrations.py
│   ├── pipeline/
│   │   ├── controller.py
│   │   ├── cache.py
│   │   ├── jobs.py
│   │   └── cancellation.py
│   ├── server/
│   │   ├── app_server.py
│   │   ├── session.py
│   │   └── api.py
│   ├── data/
│   │   ├── reach_mapping_v1.json
│   │   ├── trace_profiles_v1.json
│   │   └── destination_presets_v1.json
│   └── web/
│       ├── index.html
│       ├── app.js
│       ├── styles.css
│       ├── assets/
│       └── wasm/
└── tests/
    ├── unit/
    ├── integration/
    ├── golden/
    ├── accessibility/
    ├── security/
    └── cross_platform/
```

---

# 33. Testing requirements

## 33.1 Unit tests

Required:

* Hue wraparound.
* Low-chroma hue suppression.
* Linked reach interpolation.
* Custom channel tolerances.
* Claim conflict resolution.
* Exact output-colour preservation.
* Deterministic automatic clustering.
* Minimum palette separation.
* Empty automatic cluster handling.
* Edge-connected background detection.
* Component removal.
* Hole filling.
* Preview threshold scaling.
* Layer-order independence from claim priority.
* Schema validation.
* Preset migration.
* Source fingerprinting.
* Transform composition.

## 33.2 Backend conformance tests

Every backend MUST pass:

* Solid rectangle.
* Donut with hole.
* Concave polygon.
* Sharp star.
* Smooth circle.
* One-pixel noise.
* Thin diagonal line.
* Multiple disconnected components.
* Large mask.
* Empty mask.
* Full mask.
* Cancellation or documented lack thereof.
* Repeat-output determinism.

## 33.3 Golden-image tests

Corpus MUST include:

* Flat logo.
* Faded logo with exact replacement palette.
* Black comic outlines with coloured fills.
* Low-saturation image.
* Near-grayscale image.
* Transparent PNG.
* JPEG-compressed illustration.
* Tiny accent colour.
* Foreground sharing background colour.
* Coloured shadows.
* Smooth gradient.
* Pixel art.
* High-resolution scan.
* Multiple skin tones.
* Screen-print-like artwork.
* Vinyl-cut-like artwork.
* Laser silhouette.

Golden tests SHOULD compare:

* Classified-pixel percentages.
* Palette colours.
* Mask hashes.
* Path count.
* Node count.
* Bounds.
* Hole count.
* Deterministic output hash.

## 33.4 Accessibility tests

* Keyboard navigation.
* Visible focus.
* Accessible names.
* Live status announcements.
* No colour-only state.
* Zoom at 200%.
* Narrow viewport.
* High-contrast browser settings where practical.

## 33.5 Cross-platform tests

Required release targets:

* Windows.
* macOS.
* Linux.

CI SHOULD test:

* Core colour and mask logic on all platforms.
* Preset/schema handling on all platforms.
* Backend availability detection.
* At least one complete reference-backend trace where supported.

---

# 34. MVP acceptance criteria

The MVP is viable only when all of the following work.

Criteria are written in the vocabulary of the Inkscape host. Where a criterion names the document, the source `<image>` or the generated group, the standalone host satisfies the equivalent obligation defined in §9.4.5. Every criterion applies to both hosts except §34.23, which is inherent to attribute storage and is explicitly not applicable to the standalone host.

1. One selected embedded or linked local bitmap can be opened.
2. The interface restores settings stored on that image.
3. The user can choose 1–64 scans.
4. The user can pick one or more colours from the preview.
5. Picked colours become exact output colours.
6. Each picked colour has a Colour reach control.
7. Hue, chroma and lightness tolerances can be edited separately.
8. Neutral colours do not behave unpredictably because of meaningless hue.
9. Remaining scans are generated deterministically.
10. Automatic colours account for pinned colours.
11. Conflicting claims resolve deterministically.
12. Background may be kept, omitted or replaced.
13. Every scan may inherit or override a trace profile.
14. Black may use a smooth optimized profile while blue uses sharp detail and stronger speckle removal.
15. Illustration, logo, screen-printing, vinyl/paper and laser destinations produce distinct policies.
16. Stacked and trapped output work.
17. Exclusive-layer output maintains exclusive raster ownership.
18. Laser output creates named operation groups.
19. At least one portable or cross-platform tracing backend passes conformance tests.
20. Backend selection is abstracted.
21. Output is grouped and labelled.
22. Settings are stored on the source image.
23. Deleting the image also deletes image-specific settings.
24. Saved presets can be applied to another image.
25. Reapplying can replace the linked generated group.
26. Manual-edit risk is detected or warned.
27. No source data leaves the machine.
28. The interface is keyboard accessible.
29. Errors do not corrupt the document.
30. The result is deterministic under identical inputs.

---

# 35. Explicit anti-shortcut requirements

The implementation MUST NOT:

* Generate an automatic palette and merely replace some swatches afterward.
* Use ordinary RGB Euclidean distance as the sole colour-matching model.
* Treat layer order as pixel-claim priority.
* Treat picked colours only as suggestions.
* Ignore channel tolerances.
* Ignore low-chroma hue behaviour.
* Run independent random clustering each time the dialog opens.
* Launch one Inkscape process per scan.
* Hard-code one tracing engine into the palette pipeline.
* Store full masks or raster previews inside SVG settings.
* Store image settings only on generated vectors.
* Delete vectors when the source image is deleted.
* Assume that black is always an outline.
* Assume that the lightest colour is always the background.
* Assume that all destinations permit overlapping paths.
* Hide unsupported backend settings.
* Depend on drag-and-drop as the only ordering mechanism.
* Use colour as the only scan identifier.
* load browser assets from the internet.
* Commit partial SVG output after a failed trace.
* claim shared-boundary geometry when only raster-exclusive masks are produced.
* Use unversioned settings or presets.

---

# 36. Implementation phases

## Phase 0: Engine and colour-model spike

Deliver:

* Backend protocol.
* Conformance fixtures.
* At least two backend adapters or prototypes.
* OKLCH conversion.
* Colour reach.
* Claim-resolution tests.
* Deterministic quantizer prototype.
* Technical decision record selecting the reference backend.

No full UI should be built before this spike demonstrates acceptable path quality and portability.

## Phase 1: Headless core

Deliver:

* Image decoding.
* Settings schema.
* Claims.
* Quantization.
* Label map.
* Mask cleanup.
* Destination geometry.
* SVG assembly.
* Command-line development harness.
* Golden tests.

## Phase 2: Portable interface

Deliver:

* Secure loopback server.
* Local browser application.
* Preview.
* Palette editing.
* Colour picking.
* Reach controls.
* Per-scan profiles.
* Destination controls.
* Cancel and Apply.
* Standalone host: command-line entry point, sidecar persistence, SVG export (§9.4).

Phase 2 delivers a product that is usable on its own. The standalone host belongs here rather than later because it depends only on the headless core and the local interface, both of which this phase produces. Building it now also enforces the §9.4.1 rule that the core never imports `inkex`, which is far cheaper to maintain than to retrofit.

## Phase 3: Inkscape integration

Deliver:

* INX descriptor.
* Selection handling.
* Image settings persistence.
* Generated-group replacement.
* Transform handling.
* Linked and embedded image support.
* Diagnostics.

## Phase 4: Presets and production validation

Deliver:

* Saved presets.
* Destination validations.
* Screen-print trapping.
* Vinyl/paper checks.
* Laser operation groups.
* Import/export.

## Phase 5: Packaging and release

Deliver:

* Cross-platform installation instructions.
* Licence review.
* Third-party notices.
* Reproducible backend build process where applicable.
* Accessibility review.
* User documentation.
* Sample SVGs.
* Release archive.

---

# 37. Future-release direction

## 37.1 Native Inkscape integration

Potential upstream work:

* Native dockable panel.
* Main-canvas colour picker.
* Core API accepting prepared masks.
* Native asynchronous per-scan tracing.
* Undo-aware incremental previews.
* Shared backend registry.

Inkscape already has a generic tracing-engine abstraction and asynchronous trace operations, making future native integration structurally plausible.

## 37.2 Shared-boundary vectorization

Future destination policy:

```text
shared_boundaries
```

Requirements:

* One geometric edge shared by neighbouring regions.
* Planar subdivision.
* No duplicate adjacent boundaries.
* Destination-aware corner arbitration.
* Topological validation.

## 37.3 Manual mask correction

Future tools:

* Brush assign.
* Brush erase.
* Flood-fill seed.
* Protect region.
* Exclude region.
* Split connected component.
* Merge components.

## 37.4 Advanced colour workflows

* Imported swatch files.
* Spot-colour names.
* ICC-aware output.
* CMYK preview.
* Colour-blindness simulation.
* Palette harmony suggestions.
* Delta-E reporting.
* Automatic background proposals.
* Source and output gamut warnings.

## 37.5 Alpha and edge recovery

* Matte decontamination.
* Alpha-band vectorization.
* SVG masks.
* Semi-transparent edge layers.
* Foreground-colour reconstruction.

## 37.6 Fabrication workflows

* Screen-print registration marks.
* Underbase generation.
* Vinyl weeding aids.
* Minimum-cut-width repair.
* Kerf compensation.
* Duplicate-cut removal.
* Engrave/cut export profiles.
* Machine-specific exporters as separate modules.

## 37.7 Batch and automation

* Multiple selected images.
* Folder processing.
* Command-line preset application.
* Headless CI use.
* Preset-based server workflows.

---

# 38. Final product definition

Palette Trace is:

> A portable, destination-aware Inkscape extension that performs deterministic palette-constrained bitmap tracing. Users may pin exact output colours, control how broadly each pinned colour claims nearby source colours, generate remaining scans automatically and apply distinct mask and vectorization profiles to individual scans. Image-specific settings remain attached to the selected bitmap, while reusable presets can be applied to other images. The tracing engine is accessed through a replaceable backend interface so the project can adopt the most suitable open-source implementation without redesigning its palette, user-interface or document-persistence architecture.

The MVP is complete only when this definition is true in actual use—not merely represented by disabled controls, placeholder adapters or an interface mock-up.
