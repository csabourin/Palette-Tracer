# Palette Trace

Art-directed multicolour bitmap tracing.

Palette Trace converts a raster image into a set of flat vector colour separations — *scans* — where **you** decide which colours matter. Colours you pick from the image become exact output colours and reserve the pixels within their reach; the remaining scans are filled in by a deterministic quantizer that works around your choices rather than over them.

It runs two ways:

* as an **Inkscape 1.x extension**, tracing a selected bitmap in place; and
* as a **standalone local web application**, tracing image files on disk.

Both hosts drive the same headless core, so results are identical.

`SPEC.md` is the authoritative implementation contract. `docs/IMPLEMENTATION_STATUS.md` records what is actually built and verified.

## Palette Tracer Engine (`engine/`)

`engine/` holds a second-generation tracing engine written in Rust to
`engine/SPEC.md`. Where this application's backend traces one binary mask per
colour, the engine builds one raster partition and one shared boundary graph, so
adjacent fills cannot drift apart or leave a seam.

It is early and it is not wired into the Python pipeline. Read
`engine/docs/IMPLEMENTATION_STATUS.md` before forming an expectation -- curve
fitting, strokes and gradients are not built yet. The engine is licensed
`MIT OR Apache-2.0`; the rest of this repository remains GPL-3.0-or-later.

```bash
make engine-test    # 292 tests
make engine-trace   # trace examples/sample.png end to end
```

## Status

Pre-release, under active development. See [docs/IMPLEMENTATION_STATUS.md](docs/IMPLEMENTATION_STATUS.md) for the current per-requirement state, and [.ai/HANDOFF.md](.ai/HANDOFF.md) for the current working slice.

## What makes it different from ordinary auto-tracing

* **Every colour is a real colour from the picture.** A colour you sample is the colour that comes out, and the ones chosen for you are snapped onto the nearest pixel colour the image actually contains — never an average across an edge that appears nowhere in it.
* **Colour reach, not RGB distance.** Matching happens in OKLCH with separate hue, chroma and lightness tolerances, so "this red" can be broad while "this specific grey" stays narrow.
* **Neutrals behave.** Hue is unreliable at low chroma, so its contribution is suppressed instead of producing arbitrary matches.
* **Deterministic.** The same image and settings produce the same paths, every run. No reseeded clustering on each dialog open.
* **Destination-aware.** Illustration, logo, screen printing, vinyl/paper cutting and laser destinations produce genuinely different geometry policies — stacked, trapped, exclusive layers, or named operation groups.
* **Backend-neutral.** Vector tracing goes through a single protocol, so the engine is swappable and evaluated by conformance tests rather than assumed.
* **Backdrops know what a hole is.** A patch of the backdrop colour sitting in the foreground — the white of an eye on a white ground — becomes a hole in whatever encloses it, or, where a hole cannot express it, a layer of its own in front.
* **Works on a phone.** Load a picture, pinch to zoom, and pick colours with a magnifier that shows the individual pixel you are aiming at.

## Requirements

* Python 3.11 or newer
* Inkscape 1.2+ (for extension mode only)
* A modern browser (for the interface, in either mode)

## Installation

### Development

```bash
python3 -m venv .venv
.venv/bin/python -m pip install -e ".[backends,test]"
```

Verify:

```bash
make verify-env
```

### As an Inkscape extension

Copy or symlink the repository contents into your Inkscape user extensions directory:

| Platform | Directory |
| -------- | --------- |
| Linux    | `~/.config/inkscape/extensions/` |
| macOS    | `~/Library/Application Support/org.inkscape.Inkscape/config/inkscape/extensions/` |
| Windows  | `%APPDATA%\inkscape\extensions\` |

Restart Inkscape. The extension appears under **Extensions → Raster → Palette Trace**.

## Usage

### Inkscape extension

Select a single embedded or linked local bitmap, then run **Extensions → Raster → Palette Trace**. A browser tab opens with the interface. Configure the palette, then **Apply** to write a labelled group of traced paths beside the source image, or **Cancel** to leave the document untouched.

Settings are stored on the source `<image>` element itself, so reopening the extension restores them — and deleting the image deletes its settings with it.

### Standalone web application

```bash
palette-trace-web
```

The interface opens in your browser and asks for a picture. Choose or drag one in — or take a photo, on a phone — and **Download SVG** hands the result back through the browser. Nothing is written to disk in this mode, and settings last for the session; save a preset to reuse a configuration.

Give it a path instead, and it works against that file:

```bash
palette-trace-web path/to/image.png
```

**Save SVG** then writes next to the source image, and settings persist in a `.palettetrace.json` sidecar file. (Load a different picture in the browser and it switches to the download route, so the file you named is never overwritten by an unrelated trace.)

Run `palette-trace-web --help` for host, port and output options.

### On a phone

The interface is built narrow-screen-first. The picture takes the screen; the
controls are icons at a comfortable touch size around it.

* Pinch to zoom, drag to pan, press and drag to pick a colour with a magnifier
  that shows the exact pixel under your finger.
* The palette is a strip along the bottom. Tap a colour to open it; **hold and
  drag** it to change what is drawn in front of what.
* One tap on **Backdrop** finds whatever runs around the edge of the picture and
  makes it the backdrop, behind everything else.
* **Add** takes a colour typed in any notation — hex, `rgb()`, `hsl()`, `hsv()`,
  `lab()`, `lch()`, `oklab()`, `oklch()`, `cmyk()`, or a CSS colour name — and
  offers to snap it to the closest colour the picture actually has.
* Loading shows real progress, and the picture appears before the trace of it
  is finished.
* Every edit is undoable, by button or by <kbd>Ctrl/Cmd</kbd>+<kbd>Z</kbd>.

Everything is reachable by keyboard too, including colour picking and reordering
(<kbd>Alt</kbd> + arrows on a swatch).

Because §9.1 binds the server to `127.0.0.1`, reaching it from a phone means running it somewhere the phone can see — set a `PORT` environment variable, or pass `--host`, and understand what you are exposing before you do.

## Privacy

No source image data leaves your machine in either mode. The interface is served from `127.0.0.1` on an ephemeral port, gated by a per-session token, and loads no assets from the internet.

## Development

```bash
make test        # full suite
make test-unit   # unit tests only
make conformance # backend conformance suite
make phase0      # Phase 0 gate check
```

Layout, testing requirements and implementation phases are defined in `SPEC.md` §32, §33 and §36. Agent-facing working instructions are in `AGENTS.md`.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).
