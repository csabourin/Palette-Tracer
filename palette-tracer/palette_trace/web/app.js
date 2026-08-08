// Palette Trace interface (SPEC §9).
//
// The interface is organised around what the user is trying to do — load a
// picture, pick the colours that matter, say what they are making, get the
// result — rather than around the settings document underneath it. Everything
// technical is still reachable; it is just not the first thing on screen.
"use strict";

const urlParams = new URLSearchParams(window.location.search);
const sessionToken = urlParams.get("token");

// -------------------------------------------------------------------------
// Wording
//
// One place for every user-facing string that names a concept, so the
// interface reads as one voice instead of leaking the settings schema's
// vocabulary ("scan", "reach", "assignment") into the screen.
// -------------------------------------------------------------------------

const DESTINATIONS = {
  illustration: {
    title: "An illustration",
    why: "Colours stack, with a hair of overlap so no gaps show through.",
  },
  logo_branding: {
    title: "A logo",
    why: "Smooth edges, closed shapes, and a check for duplicates.",
  },
  screen_printing: {
    title: "A screen print",
    why: "Colours are spread slightly into each other so a misaligned screen doesn’t show white.",
  },
  vinyl_paper: {
    title: "A vinyl or paper cut",
    why: "Every shape owns its own area, so nothing is cut twice.",
  },
  laser: {
    title: "A laser job",
    why: "Separate named operations with closed paths only.",
  },
  custom: {
    title: "Something else",
    why: "Plain stacked shapes with no special treatment.",
  },
};

const PROFILE_LABELS = {
  default: "Balanced",
  smooth_shapes: "Smooth shapes",
  sharp_details: "Sharp details",
  thin_line_art: "Thin lines",
  small_accents: "Small details",
  simplified_background: "Simplified background",
  fabrication_clean: "Clean for fabrication",
};

const ROLE_LABELS = {
  outline: "Outline",
  primary_fill: "Main colour",
  secondary_fill: "Supporting colour",
  accent: "Accent",
  highlight: "Highlight",
  shadow: "Shadow",
  operation: "Operation",
  custom: "Other",
};

const MODE_CAPTIONS = {
  source: "The picture exactly as it came in.",
  result: "The flat shapes you will get.",
  coverage: "Which colour has claimed which pixels. Outlines show where they meet.",
};

const COMMIT_WORDING = {
  document: { action: "Add to drawing", done: "The traced shapes were added to your drawing." },
  file: { action: "Save SVG", done: "The SVG file was written." },
  download: { action: "Download SVG", done: "Your SVG was downloaded." },
};

//: Reach values are a 0–100 dial in the settings document. These are the words
//: for the three parts of it that a person actually reasons about.
function reachWording(value) {
  if (value <= 15) return "only pixels very close to it";
  if (value <= 40) return "pixels close to it";
  if (value <= 70) return "a wide range around it";
  return "almost anything similar";
}

function label(map, id) {
  return map[id] || (id ? id.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase()) : "");
}

// -------------------------------------------------------------------------
// State
// -------------------------------------------------------------------------

const state = {
  session: null,
  settings: null,
  destinationPresets: { order: [], presets: {} },
  traceProfiles: { order: [], profiles: {} },
  userPresets: [],
  scanResults: [],
  claimStats: {},
  warnings: [],
  previewMode: "source",

  sourceImage: null,
  //: Pixel data for the source, used for the magnifier's live readout without
  //: a network round trip per pointer move.
  sourcePixels: null,

  //: Image-space → viewport-space transform. `null` scale means "not laid out
  //: yet"; `fitView()` fills it in as soon as the viewport has a size.
  view: { scale: 1, tx: 0, ty: 0, fitted: false },

  picking: false,
  sampleMode: "5x5_median",
  //: Where the magnifier is aimed, in source-pixel coordinates.
  target: null,

  openScanId: null,
  lastFocusedBeforeDialog: null,
};

// -------------------------------------------------------------------------
// Colour maths — mirrors palette_trace/color/conversion.py so the interface
// can apply the same §13.4 low-chroma rule the pipeline does, without asking
// the server on every pointer move.
// -------------------------------------------------------------------------

function srgbToLinear(c) {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

function srgbToOklch(r, g, b) {
  const lr = srgbToLinear(r), lg = srgbToLinear(g), lb = srgbToLinear(b);
  const l = 0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb;
  const m = 0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb;
  const s = 0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb;
  const l_ = Math.cbrt(l), m_ = Math.cbrt(m), s_ = Math.cbrt(s);
  const L = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_;
  const a = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_;
  const bb = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757993 * s_;
  const C = Math.sqrt(a * a + bb * bb);
  let h = (Math.atan2(bb, a) * 180) / Math.PI;
  if (h < 0) h += 360;
  return { L, C, h };
}

function hexToRgb01(hex) {
  const clean = String(hex || "#888888").replace("#", "");
  return {
    r: parseInt(clean.slice(0, 2), 16) / 255,
    g: parseInt(clean.slice(2, 4), 16) / 255,
    b: parseInt(clean.slice(4, 6), 16) / 255,
  };
}

function rgb255ToHex(r, g, b) {
  return "#" + [r, g, b].map((v) => Math.round(v).toString(16).padStart(2, "0")).join("").toUpperCase();
}

// -------------------------------------------------------------------------
// API client
// -------------------------------------------------------------------------

async function api(path, { method = "GET", body, quiet = false } = {}) {
  const res = await fetch(path, {
    method,
    headers: { "Content-Type": "application/json", "X-Session-Token": sessionToken },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  let data;
  try {
    data = await res.json();
  } catch {
    data = {};
  }
  if (!res.ok) {
    const message = data.error || `Something went wrong (${res.status}).`;
    if (!quiet) showAlert(message);
    throw new Error(message);
  }
  return data;
}

// -------------------------------------------------------------------------
// Announcements (§29)
// -------------------------------------------------------------------------

function announce(message) {
  document.getElementById("status-region").textContent = message;
}

let alertTimer = null;
function showAlert(message) {
  const el = document.getElementById("alert-region");
  el.textContent = message;
  el.hidden = false;
  clearTimeout(alertTimer);
  alertTimer = setTimeout(clearAlert, 9000);
}

function clearAlert() {
  const el = document.getElementById("alert-region");
  el.hidden = true;
  el.textContent = "";
}

function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str == null ? "" : String(str);
  return div.innerHTML;
}

function setView(name) {
  document.body.dataset.view = name;
}

// -------------------------------------------------------------------------
// Boot
// -------------------------------------------------------------------------

document.addEventListener("DOMContentLoaded", () => {
  if (!sessionToken) {
    setView("start");
    showAlert("This page is missing its session key. Reopen Palette Trace from the extension or the command line.");
    return;
  }
  boot().catch((err) => showAlert(err.message));
});

async function boot() {
  bindStartScreen();
  bindWorkspace();
  bindDialogs();

  const [session, destinations, profiles] = await Promise.all([
    api("/api/session"),
    api("/api/destination_presets"),
    api("/api/trace_profiles"),
  ]);

  state.session = session;
  state.settings = session.settings;
  state.destinationPresets = destinations;
  state.traceProfiles = profiles;

  renderDestinationChoices();
  applyCommitWording();

  if (!session.hasImage) {
    setView("start");
    return;
  }

  const preview = await api("/api/preview_source");
  await adoptSourceImage(preview.dataUri);
  await enterWorkspace(session.sourceChanged);
}

/** Everything that must happen once a bitmap exists, in either host. */
async function enterWorkspace(sourceChanged) {
  setView("workspace");
  document.getElementById("subject-name").textContent = state.session.imageName || "";
  renderResizeNotice();
  fitView();

  const run = await api("/api/update_settings", { method: "POST", body: { settings: state.settings } });
  applyServerResponse(run);

  if (sourceChanged) {
    document.getElementById("source-changed-badge").hidden = false;
    openDialog(document.getElementById("dialog-source-changed"));
  }
}

function applyCommitWording() {
  const wording = COMMIT_WORDING[state.session.commitTarget] || COMMIT_WORDING.download;
  for (const id of ["btn-commit", "btn-commit-bottom"]) {
    document.getElementById(id).textContent = wording.action;
  }
  // A session that owns no file has nothing to discard on the way out, so the
  // destructive-sounding Cancel is replaced by a plain close.
  const isDownload = state.session.commitTarget === "download";
  document.getElementById("btn-cancel-bottom").textContent = isDownload ? "Close" : "Cancel";
  document.getElementById("btn-change-image").hidden = !state.session.canLoadImage;
}

function renderResizeNotice() {
  const el = document.getElementById("resize-notice");
  const notice = state.session.resizeNotice;
  el.hidden = !notice;
  el.textContent = notice || "";
}

// -------------------------------------------------------------------------
// Loading a picture (§9.4.2)
// -------------------------------------------------------------------------

function bindStartScreen() {
  const fileInput = document.getElementById("file-input");
  const cameraInput = document.getElementById("camera-input");
  const dropzone = document.getElementById("dropzone");

  fileInput.addEventListener("change", () => handleChosenFile(fileInput.files[0]));
  cameraInput.addEventListener("change", () => handleChosenFile(cameraInput.files[0]));
  document.getElementById("btn-take-photo").addEventListener("click", () => cameraInput.click());

  for (const eventName of ["dragenter", "dragover"]) {
    dropzone.addEventListener(eventName, (e) => {
      e.preventDefault();
      dropzone.classList.add("is-hot");
    });
  }
  for (const eventName of ["dragleave", "drop"]) {
    dropzone.addEventListener(eventName, () => dropzone.classList.remove("is-hot"));
  }
  dropzone.addEventListener("drop", (e) => {
    e.preventDefault();
    handleChosenFile(e.dataTransfer.files && e.dataTransfer.files[0]);
  });

  // A drop anywhere else on the start screen would otherwise navigate the tab
  // to the file, which silently loses the session.
  window.addEventListener("dragover", (e) => e.preventDefault());
  window.addEventListener("drop", (e) => e.preventDefault());
}

async function handleChosenFile(file) {
  if (!file) return;
  clearAlert();

  const busy = document.getElementById("start-busy");
  busy.hidden = false;
  announce("Reading your picture.");

  try {
    const dataUri = await readFileAsDataUri(file);
    const data = await api("/api/load_image", {
      method: "POST",
      body: { dataUri, fileName: file.name },
    });

    state.session = data;
    state.settings = data.settings;
    state.scanResults = data.scanResults || [];
    state.claimStats = data.claimStats || {};
    state.warnings = data.warnings || [];

    await adoptSourceImage(data.dataUri);
    applyCommitWording();
    setView("workspace");
    document.getElementById("subject-name").textContent = data.imageName || "";
    renderResizeNotice();
    fitView();
    renderAll();

    announce(
      `Loaded ${data.imageName}, ${data.imageWidth} by ${data.imageHeight} pixels. ` +
      `${state.settings.palette.entries.length} colours were chosen for you.`
    );
  } catch (err) {
    showAlert(err.message);
  } finally {
    busy.hidden = true;
    document.getElementById("file-input").value = "";
    document.getElementById("camera-input").value = "";
  }
}

function readFileAsDataUri(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(new Error("That file could not be read."));
    reader.readAsDataURL(file);
  });
}

/**
 * Decodes the preview PNG and caches its pixels.
 *
 * The magnifier needs a colour for every pointer move; asking the server each
 * time would put a network round trip inside a drag. The pixels are read once
 * here instead, and the committed sample still comes from the server so the
 * pipeline's value stays authoritative.
 */
async function adoptSourceImage(dataUri) {
  const image = await new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("The picture preview could not be displayed."));
    img.src = dataUri;
  });

  state.sourceImage = image;
  state.view.fitted = false;

  const scratch = document.createElement("canvas");
  scratch.width = image.width;
  scratch.height = image.height;
  const ctx = scratch.getContext("2d", { willReadFrequently: true });
  ctx.drawImage(image, 0, 0);
  state.sourcePixels = ctx.getImageData(0, 0, image.width, image.height);
}

// -------------------------------------------------------------------------
// The view transform: pan, pinch-zoom, fit, actual pixels (§9.2)
// -------------------------------------------------------------------------

const MIN_SCALE = 0.05;
const MAX_SCALE = 64;

function viewportSize() {
  const rect = document.getElementById("viewport").getBoundingClientRect();
  return { width: rect.width, height: rect.height, left: rect.left, top: rect.top };
}

function fitView() {
  if (!state.sourceImage) return;
  const { width, height } = viewportSize();
  if (!width || !height) return;

  const scale = Math.min(width / state.sourceImage.width, height / state.sourceImage.height);
  state.view.scale = scale;
  state.view.tx = (width - state.sourceImage.width * scale) / 2;
  state.view.ty = (height - state.sourceImage.height * scale) / 2;
  state.view.fitted = true;
  renderCanvas();
  renderZoomLevel();
}

function setScaleAbout(nextScale, anchorX, anchorY) {
  const clamped = Math.max(MIN_SCALE, Math.min(MAX_SCALE, nextScale));
  const { scale, tx, ty } = state.view;
  // Keep the image point under the anchor stationary.
  state.view.tx = anchorX - ((anchorX - tx) / scale) * clamped;
  state.view.ty = anchorY - ((anchorY - ty) / scale) * clamped;
  state.view.scale = clamped;
  renderCanvas();
  renderZoomLevel();
}

function zoomBy(factor) {
  const { width, height } = viewportSize();
  setScaleAbout(state.view.scale * factor, width / 2, height / 2);
}

function renderZoomLevel() {
  document.getElementById("zoom-level").textContent = `${Math.round(state.view.scale * 100)}%`;
}

function screenToImage(clientX, clientY) {
  const { left, top } = viewportSize();
  return {
    x: (clientX - left - state.view.tx) / state.view.scale,
    y: (clientY - top - state.view.ty) / state.view.scale,
  };
}

function clampToImage(point) {
  return {
    x: Math.max(0, Math.min(state.sourceImage.width - 1, Math.floor(point.x))),
    y: Math.max(0, Math.min(state.sourceImage.height - 1, Math.floor(point.y))),
  };
}

// -------------------------------------------------------------------------
// Canvas rendering
// -------------------------------------------------------------------------

function renderCanvas() {
  const canvas = document.getElementById("preview-canvas");
  if (!state.sourceImage) return;

  const { width, height } = viewportSize();
  if (!width || !height) return;

  const dpr = window.devicePixelRatio || 1;
  if (canvas.width !== Math.round(width * dpr) || canvas.height !== Math.round(height * dpr)) {
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
  }

  const ctx = canvas.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);

  ctx.save();
  ctx.translate(state.view.tx, state.view.ty);
  ctx.scale(state.view.scale, state.view.scale);
  // Past 2× the user is looking at individual pixels and wants to see their
  // edges, not a blur.
  ctx.imageSmoothingEnabled = state.view.scale < 2;

  if (state.previewMode === "source" || state.previewMode === "coverage") {
    ctx.drawImage(state.sourceImage, 0, 0);
  }

  if (state.previewMode === "result") {
    for (const scan of state.scanResults) {
      if (!scan.pathDatas || scan.pathDatas.length === 0) continue;
      // All sub-paths must be combined into ONE Path2D so the even-odd fill
      // rule can punch holes. Filling each curve individually makes inner
      // contours render as solid fills instead of transparent holes.
      ctx.fillStyle = scan.color;
      ctx.fill(new Path2D(scan.pathDatas.join(" ")), scan.fillRule || "evenodd");
    }
  }

  if (state.previewMode === "coverage") {
    // §9.2: colour must not be the only way to tell scans apart, so each
    // claimed region is outlined as well as tinted.
    for (const scan of state.scanResults) {
      if (!scan.pathDatas || scan.pathDatas.length === 0) continue;
      const combined = new Path2D(scan.pathDatas.join(" "));
      ctx.fillStyle = scan.color;
      ctx.globalAlpha = 0.55;
      ctx.strokeStyle = contrastingStroke(scan.color);
      ctx.lineWidth = 1.5 / state.view.scale;
      ctx.setLineDash([4 / state.view.scale, 3 / state.view.scale]);
      ctx.fill(combined, scan.fillRule || "evenodd");
      ctx.stroke(combined);
    }
    ctx.globalAlpha = 1;
    ctx.setLineDash([]);
  }

  ctx.restore();

  if (state.target) drawTargetMarker(ctx);
}

function drawTargetMarker(ctx) {
  const x = state.target.x * state.view.scale + state.view.tx;
  const y = state.target.y * state.view.scale + state.view.ty;
  ctx.save();
  ctx.lineWidth = 1.5;
  for (const [colour, offset] of [["rgba(0,0,0,.85)", 0], ["rgba(255,255,255,.95)", 1]]) {
    ctx.strokeStyle = colour;
    ctx.beginPath();
    ctx.arc(x, y, 11 + offset, 0, Math.PI * 2);
    ctx.stroke();
  }
  ctx.restore();
}

function contrastingStroke(hex) {
  const { r, g, b } = hexToRgb01(hex);
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return luminance > 0.5 ? "rgba(0,0,0,0.7)" : "rgba(255,255,255,0.8)";
}

// -------------------------------------------------------------------------
// Sampling — mirrors server/api.py::sample_source_color so the magnifier can
// show what the chosen sample size will actually produce. The value committed
// on release still comes from the server.
// -------------------------------------------------------------------------

function pixelAt(x, y) {
  const data = state.sourcePixels;
  const index = (y * data.width + x) * 4;
  return [data.data[index], data.data[index + 1], data.data[index + 2]];
}

function sampleLocally(x, y, mode) {
  if (mode === "exact") return rgb255ToHex(...pixelAt(x, y));

  const radius = mode === "5x5_median" ? 2 : 7;
  const { width, height } = state.sourcePixels;
  const window = [];
  for (let wy = Math.max(0, y - radius); wy < Math.min(height, y + radius + 1); wy++) {
    for (let wx = Math.max(0, x - radius); wx < Math.min(width, x + radius + 1); wx++) {
      window.push(pixelAt(wx, wy));
    }
  }

  if (mode === "5x5_median") return rgb255ToHex(...medianOf(window));

  const buckets = new Map();
  for (const pixel of window) {
    const key = pixel.map((c) => Math.round((c / 255) * 31)).join(",");
    if (!buckets.has(key)) buckets.set(key, []);
    buckets.get(key).push(pixel);
  }
  let winner = [];
  for (const group of buckets.values()) {
    if (group.length > winner.length) winner = group;
  }
  return rgb255ToHex(...medianOf(winner));
}

function medianOf(pixels) {
  return [0, 1, 2].map((channel) => {
    const values = pixels.map((p) => p[channel]).sort((a, b) => a - b);
    const middle = values.length >> 1;
    return values.length % 2 ? values[middle] : (values[middle - 1] + values[middle]) / 2;
  });
}

// -------------------------------------------------------------------------
// The magnifier (§9.3)
// -------------------------------------------------------------------------

//: How many source pixels across the magnifier shows. Odd, so there is a
//: middle pixel to aim at.
const LOUPE_SOURCE_PIXELS = 13;

function showLoupe(clientX, clientY) {
  const loupe = document.getElementById("loupe");
  const { left, top, width, height } = viewportSize();

  // Sit the magnifier above the contact point, where a fingertip is not.
  const x = Math.max(70, Math.min(width - 70, clientX - left));
  const y = Math.max(150, clientY - top - 18);
  loupe.style.left = `${x}px`;
  loupe.style.top = `${y}px`;
  loupe.hidden = false;

  drawLoupe();
}

function drawLoupe() {
  if (!state.target) return;

  const canvas = document.getElementById("loupe-canvas");
  const ctx = canvas.getContext("2d");
  const size = canvas.width;
  const cell = size / LOUPE_SOURCE_PIXELS;
  const half = (LOUPE_SOURCE_PIXELS - 1) / 2;

  ctx.clearRect(0, 0, size, size);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(
    state.sourceImage,
    state.target.x - half, state.target.y - half, LOUPE_SOURCE_PIXELS, LOUPE_SOURCE_PIXELS,
    0, 0, size, size
  );

  // A pixel grid, so it is obvious that one square is one source pixel.
  ctx.strokeStyle = "rgba(0,0,0,.22)";
  ctx.lineWidth = 1;
  for (let i = 1; i < LOUPE_SOURCE_PIXELS; i++) {
    ctx.beginPath();
    ctx.moveTo(i * cell, 0); ctx.lineTo(i * cell, size);
    ctx.moveTo(0, i * cell); ctx.lineTo(size, i * cell);
    ctx.stroke();
  }

  // The centre cell is the pixel that will be sampled.
  const boxX = half * cell, boxY = half * cell;
  ctx.lineWidth = 2;
  ctx.strokeStyle = "rgba(255,255,255,.95)";
  ctx.strokeRect(boxX - 1, boxY - 1, cell + 2, cell + 2);
  ctx.strokeStyle = "rgba(0,0,0,.9)";
  ctx.strokeRect(boxX + 1, boxY + 1, cell - 2, cell - 2);

  document.getElementById("loupe-readout").textContent =
    sampleLocally(state.target.x, state.target.y, state.sampleMode);
}

function hideLoupe() {
  document.getElementById("loupe").hidden = true;
}

function setPicking(on) {
  if (on && !state.sourceImage) return;

  state.picking = on;
  document.getElementById("pickbar").hidden = !on;
  document.getElementById("viewport").classList.toggle("is-picking", on);
  document.getElementById("btn-pick").textContent =
    on ? "Stop picking" : "Pick a colour from the picture";

  if (on) {
    // Aim at the middle of what is on screen, so a keyboard user has somewhere
    // to start from without ever touching a pointer.
    // On a phone the controls scroll and the picture does not follow, so the
    // button that starts picking has to bring its subject back into view.
    document.getElementById("stage").scrollIntoView({ block: "start", behavior: "smooth" });

    const { left, top, width, height } = viewportSize();
    state.target = clampToImage(screenToImage(left + width / 2, top + height / 2));
    document.getElementById("preview-canvas").focus({ preventScroll: true });
    announce("Picking a colour. Press and drag on the picture, or use the arrow keys and press Enter.");
  } else {
    state.target = null;
    hideLoupe();
  }
  renderCanvas();
}

// -------------------------------------------------------------------------
// Pointer handling: one finger picks or pans, two fingers pinch (§9.2)
// -------------------------------------------------------------------------

const pointers = new Map();
let gesture = null;   // null | {type:"pick"} | {type:"pan",…} | {type:"pinch",…}

function bindStageGestures() {
  const viewport = document.getElementById("viewport");

  viewport.addEventListener("pointerdown", (e) => {
    if (!state.sourceImage) return;
    // The zoom controls sit inside the viewport. Capturing the pointer for a
    // pan would retarget the follow-up click away from the button that was
    // actually pressed, so those presses are left alone.
    if (e.target.closest("button")) return;

    viewport.setPointerCapture(e.pointerId);
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });

    if (pointers.size === 2) {
      startPinch();
      return;
    }
    if (pointers.size > 2) return;

    if (state.picking) {
      gesture = { type: "pick" };
      state.target = clampToImage(screenToImage(e.clientX, e.clientY));
      showLoupe(e.clientX, e.clientY);
      renderCanvas();
    } else {
      gesture = { type: "pan", x: e.clientX, y: e.clientY };
    }
  });

  viewport.addEventListener("pointermove", (e) => {
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });

    if (gesture && gesture.type === "pinch") {
      updatePinch();
    } else if (gesture && gesture.type === "pick") {
      state.target = clampToImage(screenToImage(e.clientX, e.clientY));
      showLoupe(e.clientX, e.clientY);
      renderCanvas();
    } else if (gesture && gesture.type === "pan") {
      state.view.tx += e.clientX - gesture.x;
      state.view.ty += e.clientY - gesture.y;
      gesture.x = e.clientX;
      gesture.y = e.clientY;
      renderCanvas();
    }
  });

  for (const eventName of ["pointerup", "pointercancel"]) {
    viewport.addEventListener(eventName, (e) => {
      const wasPicking = gesture && gesture.type === "pick";
      pointers.delete(e.pointerId);

      if (wasPicking && eventName === "pointerup") {
        hideLoupe();
        commitSample();
      } else if (wasPicking) {
        hideLoupe();
      }

      // Lifting one finger of a pinch leaves the other one panning.
      gesture = pointers.size >= 2 ? gesture
        : pointers.size === 1 ? { type: "pan", ...[...pointers.values()][0] }
        : null;
      if (pointers.size === 0) hideLoupe();
    });
  }

  viewport.addEventListener("wheel", (e) => {
    if (!state.sourceImage) return;
    e.preventDefault();
    const { left, top } = viewportSize();
    setScaleAbout(
      state.view.scale * Math.pow(0.999, e.deltaY),
      e.clientX - left,
      e.clientY - top
    );
  }, { passive: false });

  // Keyboard equivalent of press-and-drag. Press-and-drag is a pointer
  // gesture; without this, colour picking would be pointer-only (§29).
  document.getElementById("preview-canvas").addEventListener("keydown", onCanvasKeyDown);

  new ResizeObserver(() => {
    if (!state.view.fitted) fitView();
    else renderCanvas();
  }).observe(viewport);
}

function startPinch() {
  const [a, b] = [...pointers.values()];
  gesture = {
    type: "pinch",
    distance: Math.hypot(a.x - b.x, a.y - b.y),
    scale: state.view.scale,
  };
  hideLoupe();
}

function updatePinch() {
  const [a, b] = [...pointers.values()];
  const distance = Math.hypot(a.x - b.x, a.y - b.y);
  if (!gesture.distance) return;
  const { left, top } = viewportSize();
  setScaleAbout(
    gesture.scale * (distance / gesture.distance),
    (a.x + b.x) / 2 - left,
    (a.y + b.y) / 2 - top
  );
}

function onCanvasKeyDown(e) {
  const step = e.shiftKey ? 10 : 1;
  const nudges = {
    ArrowUp: [0, -step], ArrowDown: [0, step],
    ArrowLeft: [-step, 0], ArrowRight: [step, 0],
  };

  if (state.picking && nudges[e.key]) {
    e.preventDefault();
    const [dx, dy] = nudges[e.key];
    state.target = clampToImage({ x: state.target.x + dx, y: state.target.y + dy });
    const { left, top } = viewportSize();
    showLoupe(
      left + state.target.x * state.view.scale + state.view.tx,
      top + state.target.y * state.view.scale + state.view.ty
    );
    renderCanvas();
    announce(`${sampleLocally(state.target.x, state.target.y, state.sampleMode)} at ${state.target.x}, ${state.target.y}.`);
    return;
  }

  if (state.picking && (e.key === "Enter" || e.key === " ")) {
    e.preventDefault();
    hideLoupe();
    commitSample();
    return;
  }

  if (nudges[e.key]) {
    e.preventDefault();
    const [dx, dy] = nudges[e.key];
    state.view.tx -= dx * 12;
    state.view.ty -= dy * 12;
    renderCanvas();
    return;
  }

  if (e.key === "+" || e.key === "=") { e.preventDefault(); zoomBy(1.25); }
  if (e.key === "-") { e.preventDefault(); zoomBy(0.8); }
  if (e.key === "0") { e.preventDefault(); fitView(); }
}

async function commitSample() {
  if (!state.target) return;
  const { x, y } = state.target;
  try {
    const sample = await api("/api/sample_color", {
      method: "POST",
      body: { x, y, mode: state.sampleMode },
    });
    if (sample.hex) await addPickedColour(sample.hex, sample.r, sample.g, sample.b);
  } catch {
    // api() has already surfaced the reason; a failed sample must not leave
    // the gesture handler in a rejected state.
  }
}

// -------------------------------------------------------------------------
// Palette entries
// -------------------------------------------------------------------------

function findEntry(id) {
  return state.settings.palette.entries.find((e) => e.id === id);
}

function entryHex(entry) {
  return (entry.output && entry.output.color && entry.output.color.srgb)
    || (entry.sourceAnchor && entry.sourceAnchor.srgb)
    || "#888888";
}

/**
 * Share of the picture an entry ends up owning, or null when it has none yet.
 *
 * The pipeline reports coverage per scan for every entry; `claimStats` covers
 * only the pinned ones. An entry with no scan at all (a backdrop set to be
 * left out, say) genuinely has no figure, and reporting it as 0% would read
 * as "this colour found nothing" rather than "this colour isn't drawn".
 */
function coverageOf(entryId) {
  const scan = state.scanResults.find((s) => s.entryId === entryId);
  if (scan && typeof scan.coveragePercent === "number") return scan.coveragePercent;
  const claimed = state.claimStats[entryId];
  return typeof claimed === "number" ? claimed : null;
}

function coverageText(entryId) {
  const value = coverageOf(entryId);
  return value === null ? "not drawn" : `${value.toFixed(value < 10 ? 1 : 0)}% of the picture`;
}

/** Mirrors settings.py::create_palette_entry so a new colour can be added in one round trip. */
function createEntry(index) {
  return {
    id: (crypto.randomUUID && crypto.randomUUID()) || `entry-${Date.now()}-${index}`,
    name: `Colour ${index + 1}`,
    enabled: true,
    kind: "automatic",
    sourceAnchor: null,
    output: { mode: "automatic_centroid", color: null },
    assignment: {
      mode: "automatic",
      overallReach: 25,
      channels: {
        mode: "linked",
        hue: { enabled: true, tolerance: 10.0, weight: 1.0 },
        chroma: { enabled: true, tolerance: 0.035, weight: 1.0 },
        lightness: { enabled: true, tolerance: 0.06, weight: 1.0 },
      },
    },
    role: "primary_fill",
    traceProfile: { mode: "inherit" },
  };
}

function pinEntryTo(entry, hex, chroma) {
  entry.kind = "pinned";
  entry.sourceAnchor = { srgb: hex };
  entry.output = { mode: "exact", color: { srgb: hex } };
  entry.assignment = {
    mode: "reserve_within_reach",
    overallReach: 25,
    channels: {
      mode: "linked",
      // §13.4: hue is unreliable at low chroma, so it starts out of the match
      // for a neutral rather than producing arbitrary hits.
      hue: { enabled: chroma >= 0.02, tolerance: 10.0, weight: 1.0 },
      chroma: { enabled: true, tolerance: 0.035, weight: 1.0 },
      lightness: { enabled: true, tolerance: 0.06, weight: 1.0 },
    },
  };
}

/**
 * Adds a picked colour.
 *
 * Picking is an additive act: if every slot is already spoken for, the palette
 * grows rather than quietly overwriting a colour the user chose earlier.
 */
async function addPickedColour(hex, r, g, b) {
  const entries = state.settings.palette.entries;
  const existing = entries.find((entry) => entry.kind === "pinned"
    && (entry.sourceAnchor || {}).srgb === hex);
  if (existing) {
    announce(`${hex} is already in the palette as ${existing.name}.`);
    return;
  }

  const { C } = srgbToOklch(r, g, b);
  let target = entries.find((entry) => entry.kind === "automatic");

  if (!target) {
    target = createEntry(entries.length);
    entries.push(target);
    state.settings.palette.layerOrder.push(target.id);
    state.settings.scanCount = entries.length;
  }

  pinEntryTo(target, hex, C);
  target.name = `Picked ${hex}`;

  await pushSettings();
  announce(`Added ${hex}. It now covers ${coverageText(target.id)}.`);
}

// -------------------------------------------------------------------------
// Server round trips
// -------------------------------------------------------------------------

function applyServerResponse(data) {
  clearAlert();
  state.settings = data.settings;
  state.scanResults = data.scanResults || [];
  state.claimStats = data.claimStats || {};
  state.warnings = data.warnings || [];
  renderAll();
}

async function pushSettings() {
  const data = await api("/api/update_settings", {
    method: "POST",
    body: { settings: state.settings },
  });
  applyServerResponse(data);
}

// -------------------------------------------------------------------------
// Rendering the panel
// -------------------------------------------------------------------------

/**
 * Re-renders without dropping the keyboard user where they were standing.
 *
 * Every settings change rebuilds the swatch list and the open sheet from
 * scratch. Without this, focus lands back on <body> after each edit, which is
 * barely noticeable with a mouse and makes the interface unusable without one.
 */
function withFocusPreserved(render) {
  const active = document.activeElement;
  const id = active && active.id;
  const caret = active && "selectionStart" in active ? active.selectionStart : null;
  const scrolled = active && active.closest ? active.closest(".sheet-body, .panel") : null;
  const scrollTop = scrolled ? scrolled.scrollTop : null;

  render();

  const restored = id && document.getElementById(id);
  if (!restored) return;
  restored.focus();
  if (scrolled && scrolled.isConnected) scrolled.scrollTop = scrollTop;
  if (caret !== null && "setSelectionRange" in restored) {
    try {
      restored.setSelectionRange(caret, caret);
    } catch {
      /* not a text-selectable input */
    }
  }
}

function renderAll() {
  withFocusPreserved(() => {
    renderSwatches();
    if (state.openScanId) renderScanSheet(state.openScanId);
  });
  renderCounts();
  renderDestinationSelection();
  renderAdvancedControls();
  renderWarnings();
  renderModeCaption();
  renderCanvas();
}

function renderCounts() {
  const entries = state.settings.palette.entries;
  const pinned = entries.filter((e) => e.kind === "pinned").length;
  const automatic = entries.length - pinned;

  document.getElementById("colour-count-badge").textContent =
    `${entries.length} ${entries.length === 1 ? "colour" : "colours"}`;

  document.getElementById("colours-result").textContent = pinned
    ? `${pinned} picked by you, ${automatic} chosen for you.`
    : `All ${automatic} chosen for you. Pick any that must come out exactly right.`;

  document.getElementById("input-scans").value = state.settings.scanCount;
  document.getElementById("scans-result").textContent =
    `Your picture will be reduced to ${state.settings.scanCount} flat ` +
    `${state.settings.scanCount === 1 ? "colour" : "colours"}.`;
}

function renderSwatches() {
  const list = document.getElementById("swatch-list");
  const entries = state.settings.palette.entries;
  const backgroundId = state.settings.palette.backgroundEntryId;

  list.innerHTML = entries.map((entry) => {
    const hex = entryHex(entry);
    const warnings = warningsForEntry(entry.id);
    const facts = [
      `<span class="swatch-hex">${escapeHtml(hex)}</span>`,
      `<span>${coverageText(entry.id)}</span>`,
      entry.kind === "pinned" ? `<span class="swatch-flag">Picked</span>` : "",
      backgroundId === entry.id ? `<span class="swatch-flag">Backdrop</span>` : "",
      entry.enabled === false ? `<span>Left out</span>` : "",
      warnings.length ? `<span class="swatch-warn">⚠ ${warnings.length} warning${warnings.length > 1 ? "s" : ""}</span>` : "",
    ].filter(Boolean).join("");

    return `
      <li draggable="true" data-entry-id="${entry.id}">
        <button type="button" class="swatch-row" data-open-scan="${entry.id}"
                data-disabled="${entry.enabled === false}">
          <span class="swatch-chip" style="background-color:${escapeHtml(hex)}" aria-hidden="true"></span>
          <span class="swatch-text">
            <span class="swatch-name">${escapeHtml(entry.name)}</span>
            <span class="swatch-facts">${facts}</span>
          </span>
          <span class="swatch-go" aria-hidden="true">›</span>
        </button>
      </li>
    `;
  }).join("");
}

function warningsForEntry(entryId) {
  const scan = state.scanResults.find((s) => s.entryId === entryId);
  return scan ? scan.warnings || [] : [];
}

function renderWarnings() {
  const el = document.getElementById("warnings-summary");
  // One backend limitation produces the same sentence once per scan. Repeating
  // it four times makes the panel look broken and says nothing extra; the
  // per-colour sheets still show which scans are affected.
  const unique = [...new Set(state.warnings.map((w) => w.message || String(w)))];

  el.hidden = unique.length === 0;
  el.textContent = unique.join(" · ");
}

function renderModeCaption() {
  document.getElementById("mode-caption").textContent = MODE_CAPTIONS[state.previewMode] || "";
}

function renderDestinationChoices() {
  const list = document.getElementById("destination-list");
  list.innerHTML = state.destinationPresets.order.map((id) => {
    const wording = DESTINATIONS[id] || { title: label({}, id), why: "" };
    return `
      <li>
        <button type="button" class="choice" role="radio" aria-checked="false"
                data-destination="${id}">
          <span class="choice-mark" aria-hidden="true"></span>
          <span class="choice-text">
            <span class="choice-title">${escapeHtml(wording.title)}</span>
            <span class="choice-why">${escapeHtml(wording.why)}</span>
          </span>
        </button>
      </li>
    `;
  }).join("");
}

function renderDestinationSelection() {
  const current = state.settings.destination.id;
  for (const button of document.querySelectorAll("[data-destination]")) {
    button.setAttribute("aria-checked", String(button.dataset.destination === current));
  }
  const wording = DESTINATIONS[current];
  document.getElementById("destination-result").textContent = wording ? wording.why : "";
}

function renderAdvancedControls() {
  const settings = state.settings;

  document.getElementById("select-backend").value = settings.backend.preferredBackendId;

  const bgSelect = document.getElementById("select-background-entry");
  const currentBg = settings.palette.backgroundEntryId || "";
  bgSelect.innerHTML = '<option value="">No backdrop colour</option>';
  for (const entry of settings.palette.entries) {
    const option = document.createElement("option");
    option.value = entry.id;
    option.textContent = `${entry.name} — ${entryHex(entry)}`;
    bgSelect.appendChild(option);
  }
  bgSelect.value = currentBg;

  // The backdrop's matching and output modes mean nothing until a backdrop
  // colour exists, so they are not on screen until one does.
  document.getElementById("background-detail").hidden = !currentBg;

  const matching = settings.palette.backgroundMatching || "all_matching";
  const matchingInput = document.querySelector(`input[name="bg-matching"][value="${matching}"]`);
  if (matchingInput) matchingInput.checked = true;

  const output = settings.output.backgroundOutput || "keep_paths";
  const outputInput = document.querySelector(`input[name="bg-output"][value="${output}"]`);
  if (outputInput) outputInput.checked = true;
}

// -------------------------------------------------------------------------
// The one-colour sheet
// -------------------------------------------------------------------------

function openScanSheet(entryId) {
  state.openScanId = entryId;
  renderScanSheet(entryId);
  openDialog(document.getElementById("sheet-scan"));
}

function renderScanSheet(entryId) {
  const entry = findEntry(entryId);
  if (!entry) {
    closeDialog(document.getElementById("sheet-scan"));
    return;
  }

  const entries = state.settings.palette.entries;
  const index = entries.indexOf(entry);
  const hex = entryHex(entry);
  const isBackground = state.settings.palette.backgroundEntryId === entry.id;
  const warnings = warningsForEntry(entry.id);

  document.getElementById("sheet-scan-title").textContent = entry.name;
  document.getElementById("sheet-scan-body").innerHTML = `
    <div class="scan-hero">
      <span class="scan-hero-chip" style="background-color:${escapeHtml(hex)}" aria-hidden="true"></span>
      <span class="scan-hero-text">
        <span class="scan-hero-hex">${escapeHtml(hex)}</span>
        <span class="scan-hero-fact">
          ${entry.kind === "pinned" ? "You picked this. It comes out exactly as shown." : "Chosen for you from what was left over."}
        </span>
        <span class="scan-hero-fact">Covers ${coverageText(entry.id)}.</span>
      </span>
    </div>

    ${warnings.length ? `<p class="scan-warning">${escapeHtml(warnings.join(" "))}</p>` : ""}

    <div class="field">
      <label for="scan-name">Call it</label>
      <input type="text" id="scan-name" value="${escapeHtml(entry.name)}" data-field="name">
    </div>

    <div class="switch-row">
      <label for="scan-enabled">Include this colour in the result</label>
      <input type="checkbox" id="scan-enabled" data-field="enabled" ${entry.enabled !== false ? "checked" : ""}>
    </div>

    <div class="field">
      <span class="field-help">Layer ${index + 1} of ${entries.length}, counting from the back.</span>
      <div class="order-row">
        <button type="button" class="btn btn-secondary" data-move="-1" ${index === 0 ? "disabled" : ""}>
          Move further back
        </button>
        <button type="button" class="btn btn-secondary" data-move="1" ${index === entries.length - 1 ? "disabled" : ""}>
          Bring further forward
        </button>
      </div>
    </div>

    ${entry.kind === "automatic" ? `
      <button type="button" class="btn btn-accent btn-wide" data-action="lock-exact">
        Lock this colour in exactly
      </button>
      <p class="card-hint">
        Chosen colours shift when you change anything else. Locking keeps this
        exact colour and lets it reserve the pixels around it.
      </p>
    ` : renderReachControls(entry)}

    ${isBackground ? `
      <p class="card-hint">This is the backdrop colour. Its role is fixed — change it under Fine-tuning.</p>
    ` : `
      <div class="field">
        <label for="scan-role">What this colour is for</label>
        <select id="scan-role" data-field="role">
          ${Object.entries(ROLE_LABELS).map(([value, text]) =>
            `<option value="${value}" ${entry.role === value ? "selected" : ""}>${escapeHtml(text)}</option>`
          ).join("")}
        </select>
        <p class="field-help">Names the layer in the result. It does not change the shapes.</p>
      </div>
    `}

    <details class="nested">
      <summary>Shape cleanup for this colour</summary>
      <div>${renderTraceProfileControls(entry)}</div>
    </details>

    <button type="button" class="btn btn-menu btn-menu-quiet" data-action="remove"
            ${entries.length <= 1 ? "disabled" : ""}>
      Remove this colour
    </button>
  `;
}

function renderReachControls(entry) {
  const assignment = entry.assignment || {};
  const channels = assignment.channels || {};
  const reach = assignment.overallReach ?? 25;
  const anchor = (entry.sourceAnchor || {}).srgb;

  let neutralHint = "";
  if (anchor) {
    const { r, g, b } = hexToRgb01(anchor);
    if (srgbToOklch(r, g, b).C < 0.05) {
      neutralHint = `<p class="field-help">This colour is nearly grey, so hue barely matters for it.</p>`;
    }
  }

  return `
    <div class="field">
      <label for="scan-reach">How much this colour grabs</label>
      <div class="range-row">
        <input type="range" id="scan-reach" min="0" max="100" value="${reach}" data-field="reach">
        <output for="scan-reach" id="scan-reach-out">${reach}</output>
      </div>
      <p class="field-help" id="scan-reach-words">Keeps ${reachWording(reach)}.</p>
      ${neutralHint}
    </div>

    <details class="nested">
      <summary>Match on hue, saturation and lightness separately</summary>
      <div>
        <p class="card-hint">
          Off by default: one dial covers most cases. Separate the three when a
          colour must be broad in one respect and strict in another.
        </p>
        <div class="switch-row">
          <label for="scan-channels-custom">Control the three separately</label>
          <input type="checkbox" id="scan-channels-custom" data-field="channelsCustom"
                 ${channels.mode === "custom" ? "checked" : ""}>
        </div>
        ${channels.mode === "custom" ? renderChannelControls(entry, channels) : ""}
      </div>
    </details>
  `;
}

const CHANNEL_SPECS = [
  { key: "hue", label: "Hue", help: "How far around the colour wheel still counts.", min: 0, max: 180, step: 1 },
  { key: "chroma", label: "Saturation", help: "How much duller or richer still counts.", min: 0, max: 0.4, step: 0.005 },
  { key: "lightness", label: "Lightness", help: "How much lighter or darker still counts.", min: 0, max: 1, step: 0.01 },
];

function renderChannelControls(entry, channels) {
  return CHANNEL_SPECS.map((spec) => {
    const config = channels[spec.key] || { enabled: true, tolerance: spec.min, weight: 1 };
    return `
      <fieldset class="group">
        <legend>${spec.label}</legend>
        <div class="switch-row">
          <label for="ch-${spec.key}-on">Take ${spec.label.toLowerCase()} into account</label>
          <input type="checkbox" id="ch-${spec.key}-on" data-field="channelEnabled"
                 data-channel="${spec.key}" ${config.enabled !== false ? "checked" : ""}>
        </div>
        <div class="field">
          <label for="ch-${spec.key}-tol">${spec.help}</label>
          <div class="range-row">
            <input type="range" id="ch-${spec.key}-tol" min="${spec.min}" max="${spec.max}" step="${spec.step}"
                   value="${config.tolerance ?? spec.min}" data-field="channelTolerance" data-channel="${spec.key}">
            <output for="ch-${spec.key}-tol" id="ch-${spec.key}-out">${config.tolerance ?? spec.min}</output>
          </div>
        </div>
        <div class="field">
          <label for="ch-${spec.key}-weight">How much it counts relative to the others</label>
          <input type="number" id="ch-${spec.key}-weight" min="0" step="0.1" value="${config.weight ?? 1}"
                 data-field="channelWeight" data-channel="${spec.key}">
        </div>
      </fieldset>
    `;
  }).join("");
}

const PROFILE_FIELDS = {
  mask: [
    { key: "minimumRegionAreaPx2", label: "Drop specks smaller than (px²)", min: 0, step: 1 },
    { key: "fillHolesAreaPx2", label: "Fill holes smaller than (px²)", min: 0, step: 1 },
    { key: "closeGapsRadiusPx", label: "Close gaps up to (px)", min: 0, step: 0.5 },
    { key: "smoothingRadiusPx", label: "Smooth edges by (px)", min: 0, step: 0.1 },
    { key: "minimumFeatureWidthPx", label: "Protect features at least this wide (px)", min: 0, step: 1 },
    { key: "offsetPx", label: "Grow or shrink the shape (px)", min: -50, step: 0.5 },
  ],
  vector: [
    { key: "cornerSensitivity", label: "Keep corners sharp", min: 0, max: 1, step: 0.05 },
    { key: "curveSmoothing", label: "Smooth the curves", min: 0, max: 1, step: 0.05 },
    { key: "optimization", label: "Simplify the paths", min: 0, max: 1, step: 0.05 },
    { key: "minimumPathAreaPx2", label: "Drop paths smaller than (px²)", min: 0, step: 1 },
  ],
};

function renderTraceProfileControls(entry) {
  const profile = entry.traceProfile || { mode: "inherit" };
  const presetOptions = state.traceProfiles.order.map((id) =>
    `<option value="${id}" ${profile.profileId === id ? "selected" : ""}>${escapeHtml(label(PROFILE_LABELS, id))}</option>`
  ).join("");

  let html = `
    <div class="field">
      <label for="profile-mode">How this colour’s shapes are cleaned up</label>
      <select id="profile-mode" data-field="profileMode">
        <option value="inherit" ${profile.mode === "inherit" ? "selected" : ""}>Same as everything else</option>
        <option value="preset" ${profile.mode === "preset" ? "selected" : ""}>Use a ready-made style</option>
        <option value="override" ${profile.mode === "override" ? "selected" : ""}>Set it myself</option>
      </select>
    </div>
  `;

  if (profile.mode === "preset") {
    html += `
      <div class="field">
        <label for="profile-preset">Which style</label>
        <select id="profile-preset" data-field="profilePresetId">${presetOptions}</select>
      </div>
    `;
  }

  if (profile.mode === "override") {
    const base = (profile.profileId && state.traceProfiles.profiles[profile.profileId])
      || state.settings.globalTraceProfile;
    const values = profile.values || base;

    html += `
      <div class="field">
        <label for="profile-base">Start from</label>
        <select id="profile-base" data-field="profileBaseId">
          <option value="" ${!profile.profileId ? "selected" : ""}>The overall settings</option>
          ${presetOptions}
        </select>
      </div>
      <fieldset class="group">
        <legend>Shape cleanup</legend>
        ${PROFILE_FIELDS.mask.map((f) => profileField("mask", f, values.mask || base.mask)).join("")}
        <div class="switch-row">
          <label for="pf-preserveThinFeatures">Protect thin features from cleanup</label>
          <input type="checkbox" id="pf-preserveThinFeatures" data-field="profileValue"
                 data-section="mask" data-key="preserveThinFeatures"
                 ${(values.mask || base.mask).preserveThinFeatures !== false ? "checked" : ""}>
        </div>
      </fieldset>
      <fieldset class="group">
        <legend>Drawing the outlines</legend>
        ${PROFILE_FIELDS.vector.map((f) => profileField("vector", f, values.vector || base.vector)).join("")}
        <div class="switch-row">
          <label for="pf-requireClosedPaths">Every path must close</label>
          <input type="checkbox" id="pf-requireClosedPaths" data-field="profileValue"
                 data-section="vector" data-key="requireClosedPaths"
                 ${(values.vector || base.vector).requireClosedPaths ? "checked" : ""}>
        </div>
      </fieldset>
    `;
  }

  return html;
}

function profileField(section, spec, values) {
  const id = `pf-${section}-${spec.key}`;
  return `
    <div class="field">
      <label for="${id}">${escapeHtml(spec.label)}</label>
      <input type="number" id="${id}" data-field="profileValue" data-section="${section}"
             data-key="${spec.key}" value="${values ? values[spec.key] : spec.min}"
             ${spec.min !== undefined ? `min="${spec.min}"` : ""}
             ${spec.max !== undefined ? `max="${spec.max}"` : ""} step="${spec.step}">
    </div>
  `;
}

// -------------------------------------------------------------------------
// Scan sheet interaction
// -------------------------------------------------------------------------

function bindScanSheet() {
  const sheet = document.getElementById("sheet-scan");

  sheet.addEventListener("click", async (e) => {
    const entry = findEntry(state.openScanId);
    if (!entry) return;

    const move = e.target.closest("[data-move]");
    if (move) {
      reorderEntry(entry.id, parseInt(move.dataset.move, 10));
      await pushSettings();
      announce(`${entry.name} moved.`);
      return;
    }

    const action = e.target.closest("[data-action]");
    if (!action) return;

    if (action.dataset.action === "lock-exact") {
      const hex = entryHex(entry);
      const { r, g, b } = hexToRgb01(hex);
      pinEntryTo(entry, hex, srgbToOklch(r, g, b).C);
      await pushSettings();
      announce(`${entry.name} is locked to ${hex}.`);
    }

    if (action.dataset.action === "remove") {
      const entries = state.settings.palette.entries;
      const index = entries.indexOf(entry);
      entries.splice(index, 1);
      state.settings.palette.layerOrder =
        state.settings.palette.layerOrder.filter((id) => id !== entry.id);
      if (state.settings.palette.backgroundEntryId === entry.id) {
        state.settings.palette.backgroundEntryId = null;
      }
      state.settings.scanCount = entries.length;
      state.openScanId = null;
      closeDialog(sheet);
      await pushSettings();
      announce(`Removed ${entry.name}. ${entries.length} colours left.`);
    }
  });

  sheet.addEventListener("input", (e) => {
    // Live readouts only. The value is committed on `change`, because a round
    // trip per pixel of slider travel would hammer the pipeline.
    if (e.target.dataset.field === "reach") {
      document.getElementById("scan-reach-out").textContent = e.target.value;
      document.getElementById("scan-reach-words").textContent =
        `Keeps ${reachWording(parseInt(e.target.value, 10))}.`;
    } else if (e.target.dataset.field === "channelTolerance") {
      document.getElementById(`ch-${e.target.dataset.channel}-out`).textContent = e.target.value;
    }
  });

  sheet.addEventListener("change", async (e) => {
    const entry = findEntry(state.openScanId);
    const field = e.target.dataset.field;
    if (!entry || !field) return;

    switch (field) {
      case "name":
        entry.name = e.target.value.trim() || entry.name;
        break;
      case "enabled":
        entry.enabled = e.target.checked;
        break;
      case "role":
        entry.role = e.target.value;
        break;
      case "reach":
        entry.assignment.overallReach = parseInt(e.target.value, 10);
        break;
      case "channelsCustom": {
        entry.assignment.channels = entry.assignment.channels || {};
        entry.assignment.channels.mode = e.target.checked ? "custom" : "linked";
        if (e.target.checked && !entry.assignment.channels.hue) {
          const anchor = (entry.sourceAnchor || {}).srgb;
          const { r, g, b } = hexToRgb01(anchor || "#888888");
          entry.assignment.channels.hue =
            { enabled: srgbToOklch(r, g, b).C >= 0.02, tolerance: 10.0, weight: 1.0 };
          entry.assignment.channels.chroma = { enabled: true, tolerance: 0.035, weight: 1.0 };
          entry.assignment.channels.lightness = { enabled: true, tolerance: 0.06, weight: 1.0 };
        }
        break;
      }
      case "channelEnabled":
        entry.assignment.channels[e.target.dataset.channel].enabled = e.target.checked;
        break;
      case "channelTolerance":
        entry.assignment.channels[e.target.dataset.channel].tolerance = parseFloat(e.target.value);
        break;
      case "channelWeight":
        entry.assignment.channels[e.target.dataset.channel].weight = parseFloat(e.target.value) || 0;
        break;
      case "profileMode":
        if (e.target.value === "inherit") {
          entry.traceProfile = { mode: "inherit" };
        } else if (e.target.value === "preset") {
          entry.traceProfile = { mode: "preset", profileId: "default" };
        } else {
          entry.traceProfile = {
            mode: "override",
            values: structuredClone(state.settings.globalTraceProfile),
          };
        }
        break;
      case "profilePresetId":
        entry.traceProfile.profileId = e.target.value;
        break;
      case "profileBaseId": {
        const base = e.target.value
          ? state.traceProfiles.profiles[e.target.value]
          : state.settings.globalTraceProfile;
        entry.traceProfile.profileId = e.target.value || undefined;
        entry.traceProfile.values = structuredClone(base);
        break;
      }
      case "profileValue": {
        const { section, key } = e.target.dataset;
        entry.traceProfile.values = entry.traceProfile.values || {};
        entry.traceProfile.values[section] = entry.traceProfile.values[section] || {};
        entry.traceProfile.values[section][key] =
          e.target.type === "checkbox" ? e.target.checked : parseFloat(e.target.value);
        break;
      }
      default:
        return;
    }

    await pushSettings();
  });
}

function reorderEntry(id, direction) {
  const entries = state.settings.palette.entries;
  const order = state.settings.palette.layerOrder;

  const index = entries.findIndex((e) => e.id === id);
  const swap = index + direction;
  if (index < 0 || swap < 0 || swap >= entries.length) return;
  [entries[index], entries[swap]] = [entries[swap], entries[index]];

  // Layer order is the list that actually drives stacking (§10.4); the visible
  // list is kept in step with it so a moved row stays where the user put it.
  const orderIndex = order.indexOf(id);
  const orderSwap = orderIndex + direction;
  if (orderIndex >= 0 && orderSwap >= 0 && orderSwap < order.length) {
    [order[orderIndex], order[orderSwap]] = [order[orderSwap], order[orderIndex]];
  }
}

// -------------------------------------------------------------------------
// Workspace controls
// -------------------------------------------------------------------------

function bindWorkspace() {
  bindStageGestures();
  bindScanSheet();

  document.getElementById("swatch-list").addEventListener("click", (e) => {
    const button = e.target.closest("[data-open-scan]");
    if (button) openScanSheet(button.dataset.openScan);
  });

  // Drag to reorder is a pointer-only convenience; Move back / Move forward in
  // each colour's sheet is the mechanism that works everywhere (§9.2).
  bindSwatchDragging();

  document.getElementById("btn-pick").addEventListener("click", () => setPicking(!state.picking));
  document.getElementById("btn-stop-picking").addEventListener("click", () => setPicking(false));

  for (const input of document.querySelectorAll('input[name="sample-mode"]')) {
    input.addEventListener("change", (e) => {
      if (!e.target.checked) return;
      state.sampleMode = e.target.value;
      if (state.target) drawLoupe();
    });
  }

  for (const input of document.querySelectorAll('input[name="preview-mode"]')) {
    input.addEventListener("change", (e) => {
      if (!e.target.checked) return;
      state.previewMode = e.target.value;
      renderModeCaption();
      renderCanvas();
    });
  }

  document.getElementById("btn-zoom-in").addEventListener("click", () => zoomBy(1.25));
  document.getElementById("btn-zoom-out").addEventListener("click", () => zoomBy(0.8));
  document.getElementById("btn-zoom-fit").addEventListener("click", fitView);
  document.getElementById("btn-zoom-actual").addEventListener("click", () => {
    const { width, height } = viewportSize();
    setScaleAbout(1, width / 2, height / 2);
  });

  document.getElementById("input-scans").addEventListener("change", (e) => {
    setScanCount(parseInt(e.target.value, 10));
  });
  document.getElementById("btn-more").addEventListener("click", () => {
    setScanCount(state.settings.scanCount + 1);
  });
  document.getElementById("btn-fewer").addEventListener("click", () => {
    setScanCount(state.settings.scanCount - 1);
  });

  document.getElementById("destination-list").addEventListener("click", async (e) => {
    const button = e.target.closest("[data-destination]");
    if (!button) return;
    const data = await api("/api/apply_destination", {
      method: "POST",
      body: { destinationId: button.dataset.destination },
    });
    applyServerResponse(data);
    announce(`Set up for ${DESTINATIONS[button.dataset.destination].title.toLowerCase()}.`);
  });

  document.getElementById("select-backend").addEventListener("change", async (e) => {
    state.settings.backend.preferredBackendId = e.target.value;
    await pushSettings();
    announce("Tracing engine changed.");
  });

  document.getElementById("select-background-entry").addEventListener("change", async (e) => {
    setBackgroundEntry(e.target.value || null);
    await pushSettings();
    announce(e.target.value ? "Backdrop colour set." : "No backdrop colour.");
  });

  for (const input of document.querySelectorAll('input[name="bg-matching"]')) {
    input.addEventListener("change", async (e) => {
      if (!e.target.checked) return;
      state.settings.palette.backgroundMatching = e.target.value;
      await pushSettings();
    });
  }

  for (const input of document.querySelectorAll('input[name="bg-output"]')) {
    input.addEventListener("change", async (e) => {
      if (!e.target.checked) return;
      state.settings.output.backgroundOutput = e.target.value;
      await pushSettings();
    });
  }

  document.getElementById("btn-reset-destination").addEventListener("click", async () => {
    const data = await api("/api/reset_destination_defaults", { method: "POST" });
    applyServerResponse(data);
    announce("Technical settings reset. Your colours were kept.");
  });

  document.getElementById("btn-menu").addEventListener("click", () => {
    openDialog(document.getElementById("sheet-menu"));
  });
  document.getElementById("btn-change-image").addEventListener("click", () => {
    closeDialog(document.getElementById("sheet-menu"));
    setPicking(false);
    setView("start");
  });

  for (const id of ["btn-commit", "btn-commit-bottom"]) {
    document.getElementById(id).addEventListener("click", commit);
  }
  for (const id of ["btn-cancel", "btn-cancel-bottom"]) {
    document.getElementById(id).addEventListener("click", cancel);
  }
}

function setScanCount(value) {
  const next = Math.max(1, Math.min(64, value || 1));
  if (next === state.settings.scanCount) return;
  state.settings.scanCount = next;
  pushSettings().then(() => {
    announce(`Now tracing ${next} ${next === 1 ? "colour" : "colours"}.`);
  });
}

function setBackgroundEntry(entryId) {
  for (const entry of state.settings.palette.entries) {
    if (entry.id === entryId) {
      entry.role = "background";
    } else if (entry.role === "background") {
      entry.role = "primary_fill";
    }
  }
  state.settings.palette.backgroundEntryId = entryId || null;
}

function bindSwatchDragging() {
  const list = document.getElementById("swatch-list");
  let dragId = null;

  list.addEventListener("dragstart", (e) => {
    const row = e.target.closest("[data-entry-id]");
    if (!row) return;
    dragId = row.dataset.entryId;
    e.dataTransfer.effectAllowed = "move";
  });

  list.addEventListener("dragover", (e) => {
    if (e.target.closest("[data-entry-id]")) e.preventDefault();
  });

  list.addEventListener("drop", async (e) => {
    const row = e.target.closest("[data-entry-id]");
    if (!row || !dragId || row.dataset.entryId === dragId) return;
    e.preventDefault();

    const entries = state.settings.palette.entries;
    const from = entries.findIndex((entry) => entry.id === dragId);
    const to = entries.findIndex((entry) => entry.id === row.dataset.entryId);
    const step = from < to ? 1 : -1;
    for (let i = from; i !== to; i += step) reorderEntry(dragId, step);

    dragId = null;
    await pushSettings();
    announce("Order changed.");
  });
}

// -------------------------------------------------------------------------
// Finishing
// -------------------------------------------------------------------------

async function commit() {
  const target = state.session.commitTarget;

  if (target === "download") {
    const data = await api("/api/export", { method: "POST" });
    downloadText(data.fileName, data.svg, "image/svg+xml");
    announce(`Downloaded ${data.fileName}.`);
    showDoneToast(`Downloaded ${data.fileName}. Keep adjusting if you want another version.`);
    return;
  }

  await api("/api/apply", { method: "POST" });
  finish(COMMIT_WORDING[target].done);
}

async function cancel() {
  if (state.session.commitTarget === "download") {
    finish("Nothing was saved.");
    return;
  }
  await api("/api/cancel", { method: "POST", quiet: true }).catch(() => {});
  finish("Cancelled. Nothing was changed.");
}

function finish(message) {
  document.getElementById("done-detail").textContent = message;
  setView("done");
  // The server shuts down on apply or cancel, so a close is a courtesy that
  // only works for script-opened tabs; the done screen is what the user
  // actually sees.
  window.close();
}

/** A download does not end the session, so it reports itself in place. */
function showDoneToast(message) {
  const el = document.getElementById("alert-region");
  el.textContent = message;
  el.hidden = false;
  clearTimeout(alertTimer);
  alertTimer = setTimeout(clearAlert, 7000);
}

function downloadText(fileName, text, mimeType) {
  const blob = new Blob([text], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  document.body.appendChild(link);
  link.click();
  link.remove();
  // Revoked on the next tick: revoking synchronously can beat the download.
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

// -------------------------------------------------------------------------
// Dialogs — native <dialog> gives focus trapping and Escape for free (§29)
// -------------------------------------------------------------------------

function openDialog(dialog) {
  state.lastFocusedBeforeDialog = document.activeElement;
  dialog.showModal();
}

function closeDialog(dialog) {
  dialog.close();
  if (state.lastFocusedBeforeDialog && state.lastFocusedBeforeDialog.isConnected) {
    state.lastFocusedBeforeDialog.focus();
  }
  state.lastFocusedBeforeDialog = null;
  if (dialog.id === "sheet-scan") state.openScanId = null;
}

function bindDialogs() {
  for (const dialog of document.querySelectorAll("dialog")) {
    dialog.addEventListener("cancel", (e) => {
      e.preventDefault();
      closeDialog(dialog);
    });
    for (const button of dialog.querySelectorAll("[data-dialog-cancel]")) {
      button.addEventListener("click", () => closeDialog(dialog));
    }
  }

  const sourceDialog = document.getElementById("dialog-source-changed");
  for (const button of sourceDialog.querySelectorAll("[data-action]")) {
    button.addEventListener("click", () => resolveSourceChange(button.dataset.action));
  }

  document.getElementById("btn-save-preset").addEventListener("click", () => {
    closeDialog(document.getElementById("sheet-menu"));
    openDialog(document.getElementById("dialog-save-preset"));
    document.getElementById("preset-name").focus();
  });

  document.getElementById("btn-load-preset").addEventListener("click", async () => {
    closeDialog(document.getElementById("sheet-menu"));
    await refreshUserPresets();
    renderPresetList();
    openDialog(document.getElementById("dialog-load-preset"));
  });

  document.getElementById("form-save-preset").addEventListener("submit", async (e) => {
    e.preventDefault();
    const name = document.getElementById("preset-name").value.trim();
    if (!name) return;
    await api("/api/user_presets", {
      method: "POST",
      body: {
        name,
        description: document.getElementById("preset-description").value.trim(),
        scope: document.querySelector('input[name="preset-scope"]:checked').value,
      },
    });
    document.getElementById("preset-name").value = "";
    document.getElementById("preset-description").value = "";
    closeDialog(document.getElementById("dialog-save-preset"));
    announce(`Saved “${name}”.`);
  });

  document.getElementById("preset-list").addEventListener("click", async (e) => {
    const button = e.target.closest("[data-preset-action]");
    if (!button) return;
    const presetUuid = button.dataset.presetId;

    if (button.dataset.presetAction === "apply") {
      const data = await api("/api/user_presets/apply", { method: "POST", body: { presetUuid } });
      applyServerResponse(data);
      closeDialog(document.getElementById("dialog-load-preset"));
      announce("Saved settings applied.");
    } else {
      await api("/api/user_presets/delete", { method: "POST", body: { presetUuid } });
      await refreshUserPresets();
      renderPresetList();
      announce("Deleted.");
    }
  });
}

async function resolveSourceChange(action) {
  const dialog = document.getElementById("dialog-source-changed");

  if (action === "cancel") {
    await api("/api/resolve_source_change", { method: "POST", body: { action: "cancel" } });
    closeDialog(dialog);
    finish("Cancelled. Nothing was changed.");
    return;
  }

  const data = await api("/api/resolve_source_change", { method: "POST", body: { action } });
  applyServerResponse(data);
  document.getElementById("source-changed-badge").hidden = true;
  closeDialog(dialog);
  announce("Sorted out.");
}

async function refreshUserPresets() {
  const data = await api("/api/user_presets");
  state.userPresets = data.presets || [];
}

const PRESET_SCOPE_LABELS = {
  full: "Everything",
  palette: "Colours only",
  structure: "Setup only",
};

function renderPresetList() {
  const list = document.getElementById("preset-list");
  const empty = document.getElementById("preset-list-empty");

  if (!state.userPresets.length) {
    list.innerHTML = "";
    empty.hidden = false;
    return;
  }

  empty.hidden = true;
  list.innerHTML = state.userPresets.map((preset) => `
    <li class="preset-row">
      <div>
        <div class="swatch-name">${escapeHtml(preset.name)}</div>
        <span class="badge">${escapeHtml(label(PRESET_SCOPE_LABELS, preset.scope))}</span>
        ${preset.description ? `<p class="card-hint">${escapeHtml(preset.description)}</p>` : ""}
      </div>
      <div class="preset-row-actions">
        <button type="button" class="btn btn-primary" data-preset-action="apply"
                data-preset-id="${preset.presetUuid}">Use these</button>
        <button type="button" class="btn btn-secondary" data-preset-action="delete"
                data-preset-id="${preset.presetUuid}"
                aria-label="Delete ${escapeHtml(preset.name)}">Delete</button>
      </div>
    </li>
  `).join("");
}
