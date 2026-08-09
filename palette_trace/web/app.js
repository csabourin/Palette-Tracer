// Palette Trace interface (SPEC §9).
//
// The interface is organised around what the user is trying to do — load a
// picture, pick the colours that matter, say what they are making, get the
// result — rather than around the settings document underneath it. Everything
// technical is still reachable; it is just not the first thing on screen.
//
// On a phone the picture is the screen. Controls are icons at a 44px target,
// prose lives inside sheets, and the palette is a strip along the bottom that a
// finger can drag through.
"use strict";

const urlParams = new URLSearchParams(window.location.search);
const sessionToken = urlParams.get("token");

//: The settings schema allows 1–64 palette entries (image-settings-v1). The
//: interface no longer has a number to type, so it enforces the ceiling where
//: the palette grows instead.
const MAX_ENTRIES = 64;

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
  source: "",
  result: "The flat shapes you will get.",
  coverage: "Which colour has claimed which pixels.",
};

const COMMIT_WORDING = {
  document: { action: "Add", done: "The traced shapes were added to your drawing." },
  file: { action: "Save", done: "The SVG file was written." },
  download: { action: "Download", done: "Your SVG was downloaded." },
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

  //: Image-space → viewport-space transform. `fitted: false` means "not laid
  //: out yet"; `fitView()` fills it in as soon as the viewport has a size.
  view: { scale: 1, tx: 0, ty: 0, fitted: false },

  picking: false,
  //: A picked colour must be a colour the picture has. One pixel is the most
  //: literal answer to that, so it is where the picker starts; the wider
  //: samples are there for a noisy scan, and they too report a real pixel.
  sampleMode: "exact",
  //: Where the magnifier is aimed, in source-pixel coordinates.
  target: null,
  //: Set when picking was started from the palette sheet, which has to close to
  //: let the picture be touched at all. Picking started from the toolbar leaves
  //: it false, so ending there does not open a sheet nobody asked for.
  pickReturnsToPalette: false,

  openScanId: null,
  colourSheet: null,
  lastFocusedBeforeDialog: null,

  //: Palette edits are undoable (§9.2 is silent on this; every other tool a
  //: designer opens has it, and picking colours is exactly the kind of work
  //: that is done by trying things).
  history: { past: [], future: [] },
};

const HISTORY_LIMIT = 40;

// -------------------------------------------------------------------------
// Colour maths
//
// Mirrors palette_trace/color/conversion.py so the interface can apply the
// same §13.4 low-chroma rule the pipeline does, and can talk about a colour in
// whatever notation the user brought with them, without a round trip.
// -------------------------------------------------------------------------

const clamp01 = (v) => Math.max(0, Math.min(1, v));

function srgbToLinear(c) {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

function linearToSrgb(c) {
  return c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(Math.max(0, c), 1 / 2.4) - 0.055;
}

function srgbToOklab(r, g, b) {
  const lr = srgbToLinear(r), lg = srgbToLinear(g), lb = srgbToLinear(b);
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
  return {
    L: 0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
    a: 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
    b: 0.0259040371 * l + 0.7827717662 * m - 0.8086757993 * s,
  };
}

function oklabToSrgb(L, a, bb) {
  const l = (L + 0.3963377774 * a + 0.2158037573 * bb) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * bb) ** 3;
  const s = (L - 0.0894841775 * a - 1.2914855480 * bb) ** 3;
  return {
    r: clamp01(linearToSrgb(+4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s)),
    g: clamp01(linearToSrgb(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s)),
    b: clamp01(linearToSrgb(-0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s)),
  };
}

function srgbToOklch(r, g, b) {
  const { L, a, b: bb } = srgbToOklab(r, g, b);
  const C = Math.hypot(a, bb);
  let h = (Math.atan2(bb, a) * 180) / Math.PI;
  if (h < 0) h += 360;
  return { L, C, h };
}

function oklchToSrgb(L, C, h) {
  const rad = (h * Math.PI) / 180;
  return oklabToSrgb(L, C * Math.cos(rad), C * Math.sin(rad));
}

//: CSS `lab()` and `lch()` are D50-referred (CSS Color 4 §7). These are that
//: specification's own matrix and white point, so a value pasted out of a
//: browser's inspector round-trips.
const D50 = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
const XYZ_D50_TO_LINEAR = [
  [3.1341359569958707, -1.6173863321612538, -0.4906619460083532],
  [-0.978795502912089, 1.9161404236526823, 0.033442731161319486],
  [0.07195537988411677, -0.22897682641583285, 1.4053400641663395],
];

function labToSrgb(L, a, bb) {
  const fy = (L + 16) / 116, fx = fy + a / 500, fz = fy - bb / 200;
  const epsilon = 216 / 24389, kappa = 24389 / 27;
  const expand = (f, useCube) => (useCube ? f ** 3 : (116 * f - 16) / kappa);

  const xyz = [
    expand(fx, fx ** 3 > epsilon) * D50[0],
    (L > kappa * epsilon ? ((L + 16) / 116) ** 3 : L / kappa) * D50[1],
    expand(fz, fz ** 3 > epsilon) * D50[2],
  ];

  const [r, g, b] = XYZ_D50_TO_LINEAR.map(
    (row) => row[0] * xyz[0] + row[1] * xyz[1] + row[2] * xyz[2]
  );
  return { r: clamp01(linearToSrgb(r)), g: clamp01(linearToSrgb(g)), b: clamp01(linearToSrgb(b)) };
}

function hslToSrgb(h, s, l) {
  const f = (n) => {
    const k = (n + h / 30) % 12;
    return l - s * Math.min(l, 1 - l) * Math.max(-1, Math.min(k - 3, 9 - k, 1));
  };
  return { r: clamp01(f(0)), g: clamp01(f(8)), b: clamp01(f(4)) };
}

function srgbToHsl(r, g, b) {
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (!d) return { h: 0, s: 0, l };
  const s = d / (1 - Math.abs(2 * l - 1));
  let h;
  if (max === r) h = ((g - b) / d) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  h *= 60;
  return { h: h < 0 ? h + 360 : h, s, l };
}

function hsvToSrgb(h, s, v) {
  const l = v * (1 - s / 2);
  return hslToSrgb(h, l === 0 || l === 1 ? 0 : (v - l) / Math.min(l, 1 - l), l);
}

function cmykToSrgb(c, m, y, k) {
  return {
    r: clamp01((1 - c) * (1 - k)),
    g: clamp01((1 - m) * (1 - k)),
    b: clamp01((1 - y) * (1 - k)),
  };
}

function srgbToCmyk(r, g, b) {
  const k = 1 - Math.max(r, g, b);
  if (k >= 1) return { c: 0, m: 0, y: 0, k: 1 };
  return { c: (1 - r - k) / (1 - k), m: (1 - g - k) / (1 - k), y: (1 - b - k) / (1 - k), k };
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

function rgb01ToHex({ r, g, b }) {
  return rgb255ToHex(clamp01(r) * 255, clamp01(g) * 255, clamp01(b) * 255);
}

// -------------------------------------------------------------------------
// Reading a colour someone typed (§9.2: a colour may be entered as a value)
// -------------------------------------------------------------------------

//: Every notation below is parsed here rather than handed to the browser,
//: because the browser's answer depends on which colour syntaxes it happens to
//: support — and a value silently misread is worse than one refused.
const COLOUR_FUNCTIONS = {
  rgb: (parts) => ({
    r: channel(parts[0], 255), g: channel(parts[1], 255), b: channel(parts[2], 255),
  }),
  rgba: (parts) => COLOUR_FUNCTIONS.rgb(parts),
  hsl: (parts) => hslToSrgb(angle(parts[0]), ratio(parts[1]), ratio(parts[2])),
  hsla: (parts) => COLOUR_FUNCTIONS.hsl(parts),
  hsv: (parts) => hsvToSrgb(angle(parts[0]), ratio(parts[1]), ratio(parts[2])),
  hsb: (parts) => COLOUR_FUNCTIONS.hsv(parts),
  lab: (parts) => labToSrgb(number(parts[0], 100), number(parts[1]), number(parts[2])),
  lch: (parts) => {
    const [L, C, h] = [number(parts[0], 100), number(parts[1]), angle(parts[2])];
    const rad = (h * Math.PI) / 180;
    return labToSrgb(L, C * Math.cos(rad), C * Math.sin(rad));
  },
  oklab: (parts) => oklabToSrgb(number(parts[0], 1), number(parts[1]), number(parts[2])),
  oklch: (parts) => oklchToSrgb(number(parts[0], 1), number(parts[1]), angle(parts[2])),
  cmyk: (parts) => cmykToSrgb(ratio(parts[0]), ratio(parts[1]), ratio(parts[2]), ratio(parts[3])),
  device_cmyk: (parts) => COLOUR_FUNCTIONS.cmyk(parts),
};

/** A component that may be written as a percentage of `full`, or as a number. */
function channel(token, full) {
  if (token === undefined) return NaN;
  const value = parseFloat(token);
  if (Number.isNaN(value)) return NaN;
  return token.includes("%") ? clamp01(value / 100) : clamp01(value / full);
}

/** A component that is a ratio: `50%` and `0.5` mean the same thing. */
function ratio(token) {
  if (token === undefined) return NaN;
  const value = parseFloat(token);
  if (Number.isNaN(value)) return NaN;
  return clamp01(token.includes("%") ? value / 100 : value > 1 ? value / 100 : value);
}

/** A plain number, where `%` is read against `full` when one is given. */
function number(token, full) {
  if (token === undefined) return NaN;
  const value = parseFloat(token);
  if (Number.isNaN(value)) return NaN;
  return token.includes("%") && full !== undefined ? (value / 100) * full : value;
}

function angle(token) {
  if (token === undefined) return NaN;
  const value = parseFloat(token);
  if (Number.isNaN(value)) return NaN;
  if (token.includes("grad")) return value * 0.9;
  if (token.includes("rad")) return (value * 180) / Math.PI;
  if (token.includes("turn")) return value * 360;
  return value;
}

function parseHex(text) {
  const digits = text.replace(/^#/, "");
  if (!/^[0-9a-f]+$/i.test(digits)) return null;

  if (digits.length === 3 || digits.length === 4) {
    const [r, g, b] = [...digits.slice(0, 3)].map((d) => parseInt(d + d, 16) / 255);
    return { r, g, b };
  }
  if (digits.length === 6 || digits.length === 8) {
    return hexToRgb01("#" + digits.slice(0, 6));
  }
  return null;
}

/** The named CSS colours, resolved by the engine that defines them. */
function parseNamed(text) {
  const probe = document.getElementById("color-probe");
  if (!probe || !/^[a-z]+$/i.test(text)) return null;

  probe.style.color = "";
  probe.style.color = text;
  if (!probe.style.color) return null;

  const computed = getComputedStyle(probe).color;
  const numbers = computed.match(/[\d.]+/g);
  if (!numbers || numbers.length < 3) return null;
  return { r: +numbers[0] / 255, g: +numbers[1] / 255, b: +numbers[2] / 255 };
}

/**
 * Reads a colour in any notation this tool accepts.
 *
 * Returns `{r, g, b}` in 0..1, or null when the text does not name a colour.
 * Alpha is parsed so that pasting `rgba(…, .5)` is not an error, and then
 * dropped: a scan is a flat fill, and pretending otherwise would produce an
 * output colour the result cannot contain.
 */
function parseColorInput(text) {
  const trimmed = String(text || "").trim().toLowerCase();
  if (!trimmed) return null;

  const call = trimmed.match(/^([a-z-]+)\s*\(([^)]*)\)$/);
  if (call) {
    const name = call[1].replace(/-/g, "_");
    const handler = COLOUR_FUNCTIONS[name];
    if (!handler) return null;

    // Both `rgb(1, 2, 3)` and `rgb(1 2 3 / 50%)` are current syntax; the alpha
    // after the slash is parsed off and discarded.
    const parts = call[2].split("/")[0].trim().split(/[\s,]+/).filter(Boolean);
    const result = handler(parts);
    if (!result || [result.r, result.g, result.b].some((v) => Number.isNaN(v))) return null;
    return { r: clamp01(result.r), g: clamp01(result.g), b: clamp01(result.b) };
  }

  return parseHex(trimmed) || parseNamed(trimmed);
}

/** The same colour written every way this tool understands. */
function colorNotations(rgb) {
  const hsl = srgbToHsl(rgb.r, rgb.g, rgb.b);
  const oklch = srgbToOklch(rgb.r, rgb.g, rgb.b);
  const cmyk = srgbToCmyk(rgb.r, rgb.g, rgb.b);
  const round = (v, places = 0) => v.toFixed(places);

  return [
    rgb01ToHex(rgb),
    `rgb(${round(rgb.r * 255)} ${round(rgb.g * 255)} ${round(rgb.b * 255)})`,
    `hsl(${round(hsl.h)} ${round(hsl.s * 100)}% ${round(hsl.l * 100)}%)`,
    `oklch(${round(oklch.L * 100)}% ${round(oklch.C, 3)} ${round(oklch.h)})`,
    `cmyk(${round(cmyk.c * 100)}% ${round(cmyk.m * 100)}% ${round(cmyk.y * 100)}% ${round(cmyk.k * 100)}%)`,
  ];
}

// -------------------------------------------------------------------------
// API client
// -------------------------------------------------------------------------

/**
 * Every API call, and the one place that can tell "this failed" apart from
 * "something else answered".
 *
 * This server replies in JSON to every path under /api/, success or failure.
 * So a reply that is not JSON did not come from it: a proxy in front, a port
 * where nothing is listening, an address that reaches something else entirely.
 * Reporting that as "something went wrong" blames this application for a
 * problem that is not in it, and leaves the one useful fact — that the answer
 * came from somewhere else — unsaid.
 */
async function api(path, { method = "GET", body, quiet = false } = {}) {
  const res = await fetch(path, {
    method,
    headers: { "Content-Type": "application/json", "X-Session-Token": sessionToken },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });

  let raw = "";
  try {
    raw = await res.text();
  } catch {
    /* a body that cannot be read is handled as an unparseable one */
  }

  let data = null;
  try {
    data = raw ? JSON.parse(raw) : {};
  } catch {
    data = null;   // not JSON, therefore not this server
  }

  if (data === null) {
    const message = `${path} was answered by something other than Palette Trace `
      + `(HTTP ${res.status}). Check the address you opened, and anything sitting `
      + `in front of the server.`;
    if (!quiet) showAlert(message);
    throw new Error(message);
  }

  if (!res.ok) {
    const message = data.error || `Something went wrong (${res.status}).`;
    if (!quiet) showAlert(message);
    throw new Error(message);
  }
  return data;
}

/**
 * Posts a bitmap as its own bytes, reporting how much has gone.
 *
 * `fetch` cannot report upload progress, and the upload is the one part of
 * loading a picture whose length is genuinely known — so this stays on
 * XMLHttpRequest, which can.
 */
/**
 * Sends the chosen bitmap, by whichever route the server on the end understands.
 *
 * Raw bytes are the fast route. It depends on a `Content-Type` reaching the
 * other end intact, which is out of this page's hands — a reverse proxy sits in
 * front of some deployments, and a server from a different release may not read
 * a raw body at all. So a refusal falls back to the base64 `data:` URI form,
 * which is slower and always understood, rather than being reported as a
 * mysterious complaint about data URIs.
 */
async function sendImage(blob, meta, onProgress) {
  try {
    return await uploadImage(blob, meta, onProgress);
  } catch (error) {
    if (!error.recoverable) throw error;
  }

  loading.set(0.35, "Sending it the long way round…");
  return api("/api/load_image", {
    method: "POST",
    body: {
      dataUri: await readFileAsDataUri(blob),
      fileName: meta.fileName,
      deferTrace: meta.deferTrace,
      clientBitmap: meta.width ? { width: meta.width, height: meta.height } : undefined,
    },
  });
}

function readFileAsDataUri(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(new Error("That file could not be read."));
    reader.readAsDataURL(blob);
  });
}

function uploadImage(blob, { fileName, width, height, deferTrace }, onProgress) {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("POST", "/api/load_image");
    request.setRequestHeader("Content-Type", blob.type || "application/octet-stream");
    request.setRequestHeader("X-Session-Token", sessionToken);
    request.setRequestHeader("X-File-Name", encodeURIComponent(fileName || "image"));
    if (width && height) {
      request.setRequestHeader("X-Client-Width", String(width));
      request.setRequestHeader("X-Client-Height", String(height));
    }
    if (deferTrace) request.setRequestHeader("X-Defer-Trace", "1");

    request.upload.addEventListener("progress", (event) => {
      if (event.lengthComputable && onProgress) onProgress(event.loaded / event.total);
    });
    request.addEventListener("load", () => {
      let data = null;
      try {
        data = JSON.parse(request.responseText);
      } catch {
        data = null;   // not JSON, therefore not this server — see api()
      }
      if (data !== null && request.status >= 200 && request.status < 300) {
        resolve(data);
        return;
      }
      // A 4xx here means the server would not take the bytes. That may be
      // because the picture is genuinely unusable, or because this route never
      // reached it intact — the caller tries the other route before deciding.
      const failure = new Error(
        data === null
          ? `The upload was answered by something other than Palette Trace `
            + `(HTTP ${request.status}).`
          : data.error || `That picture could not be loaded (${request.status}).`
      );
      // A reply that is not this server's JSON is the very case the fallback
      // exists for — something in the middle mangled the raw route — so it is
      // worth trying the long way round before giving up on the picture.
      failure.recoverable =
        data === null || (request.status >= 400 && request.status < 500);
      reject(failure);
    });
    request.addEventListener("error", () => reject(new Error("The picture could not be sent.")));
    request.addEventListener("abort", () => reject(new Error("Loading was cancelled.")));
    request.send(blob);
  });
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

function haptic(pattern) {
  if (navigator.vibrate) navigator.vibrate(pattern);
}

// -------------------------------------------------------------------------
// Progress
//
// Two kinds. Loading a picture has real quantities behind it — bytes decoded,
// bytes sent — so its bar is the truth. Tracing does not report from inside,
// so its bar eases towards, but never reaches, done: it says "still working"
// honestly rather than inventing a percentage.
// -------------------------------------------------------------------------

const loading = {
  fill: null,
  text: null,
  show(message) {
    this.fill = document.getElementById("start-loader-fill");
    this.text = document.getElementById("start-loader-text");
    document.getElementById("start-loader").hidden = false;
    this.set(0, message);
  },
  set(fraction, message) {
    if (this.fill) this.fill.style.width = `${Math.round(clamp01(fraction) * 100)}%`;
    if (message && this.text) this.text.textContent = message;
    if (message) announce(message);
  },
  hide() {
    document.getElementById("start-loader").hidden = true;
  },
};

const tracing = {
  timer: null,
  depth: 0,
  progress: 0,
  begin() {
    this.depth += 1;
    if (this.depth > 1) return;

    const bar = document.getElementById("topline");
    const fill = document.getElementById("topline-fill");
    bar.hidden = false;
    fill.style.opacity = "1";
    this.progress = 0.06;
    fill.style.width = "6%";

    // Each tick closes some of the remaining distance, so the bar always moves
    // and never claims to be finished before the answer is back.
    this.timer = setInterval(() => {
      this.progress += (0.94 - this.progress) * 0.08;
      fill.style.width = `${(this.progress * 100).toFixed(1)}%`;
    }, 180);
  },
  end() {
    this.depth = Math.max(0, this.depth - 1);
    if (this.depth) return;

    clearInterval(this.timer);
    const bar = document.getElementById("topline");
    const fill = document.getElementById("topline-fill");
    fill.style.width = "100%";
    setTimeout(() => {
      fill.style.opacity = "0";
      setTimeout(() => {
        bar.hidden = true;
        fill.style.width = "0%";
      }, 300);
    }, 140);
  },
};

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
  renderSubject();
  renderResizeNotice();
  fitView();

  await pushSettings();

  if (sourceChanged) {
    openDialog(document.getElementById("dialog-source-changed"));
  }
}

function applyCommitWording() {
  const wording = COMMIT_WORDING[state.session.commitTarget] || COMMIT_WORDING.download;
  const commit = document.getElementById("btn-commit");
  document.getElementById("btn-commit-text").textContent = wording.action;
  commit.setAttribute("aria-label", wording.action);
  commit.title = wording.action;

  // A session that owns no file has nothing to discard on the way out, so the
  // destructive-sounding Cancel is replaced by a plain close.
  const isDownload = state.session.commitTarget === "download";
  document.getElementById("btn-cancel-text").textContent = isDownload ? "Close" : "Cancel and discard";
  document.getElementById("btn-change-image").hidden = !state.session.canLoadImage;
}

function renderSubject() {
  document.getElementById("subject-name").textContent = state.session.imageName || "Untitled";
  const { imageWidth, imageHeight } = state.session;
  document.getElementById("subject-size").textContent =
    imageWidth && imageHeight ? `${imageWidth}×${imageHeight}` : "";
}

function renderResizeNotice() {
  const el = document.getElementById("resize-notice");
  const notice = state.session.resizeNotice;
  el.hidden = !notice;
  el.textContent = notice || "";
}

// -------------------------------------------------------------------------
// Loading a picture (§9.4.2)
//
// The picture is on screen before the colours are, because decoding a bitmap
// is fast and tracing it is not, and a blank screen for the length of both is
// the difference between a tool that feels quick and one that feels broken.
// -------------------------------------------------------------------------

//: Types the server will decode. Anything else — a HEIC straight off a phone,
//: say — is re-encoded here first, using whatever the browser can decode.
const SERVER_TYPES = new Set([
  "image/png", "image/jpeg", "image/gif", "image/bmp", "image/tiff", "image/webp",
]);

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

  // A drop anywhere else would otherwise navigate the tab to the file, which
  // silently loses the session.
  window.addEventListener("dragover", (e) => e.preventDefault());
  window.addEventListener("drop", (e) => e.preventDefault());
}

async function handleChosenFile(file) {
  if (!file) return;
  clearAlert();
  setView("start");
  loading.show("Reading your picture…");

  try {
    const prepared = await prepareUpload(file);
    loading.set(0.3, "Sending it over…");

    const data = await sendImage(
      prepared.blob,
      {
        fileName: file.name,
        width: prepared.width,
        height: prepared.height,
        deferTrace: true,
      },
      (fraction) => loading.set(0.3 + fraction * 0.5)
    );

    loading.set(0.85, "Opening the workspace…");

    state.session = data;
    state.settings = data.settings;
    state.scanResults = [];
    state.claimStats = {};
    state.warnings = [];
    state.history = { past: [], future: [] };

    if (data.usesClientBitmap && prepared.bitmap) {
      adoptBitmap(prepared.bitmap);
    } else {
      await adoptSourceImage(data.dataUri || (await api("/api/preview_source")).dataUri);
    }

    applyCommitWording();
    loading.set(1);
    loading.hide();

    await enterWorkspace(false);
    announce(
      `${data.imageName} is open, ${data.imageWidth} by ${data.imageHeight} pixels. ` +
      `${state.settings.palette.entries.length} colours were chosen for you.`
    );
  } catch (err) {
    loading.hide();
    showAlert(err.message);
  } finally {
    document.getElementById("file-input").value = "";
    document.getElementById("camera-input").value = "";
  }
}

/**
 * Gets the chosen file ready to send.
 *
 * Two things are worth doing here rather than on the server. A picture larger
 * than the working budget is going to be scaled down anyway, and scaling it
 * first means the oversized version is never encoded, never transferred and
 * never decoded. And a file the server cannot read — a phone's HEIC — becomes
 * a PNG the moment the browser proves it can decode it, instead of an error.
 */
async function prepareUpload(file) {
  const budget = (state.session && state.session.maxWorkingPixels) || 4000000;

  if (!window.createImageBitmap) {
    return { blob: file, width: 0, height: 0, bitmap: null };
  }

  let bitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    // Nothing here can decode it; the server's decoder gets its turn.
    return { blob: file, width: 0, height: 0, bitmap: null };
  }

  const pixels = bitmap.width * bitmap.height;
  const needsResize = pixels > budget;
  const needsReencode = !SERVER_TYPES.has(file.type);

  if (!needsResize && !needsReencode) {
    return { blob: file, width: bitmap.width, height: bitmap.height, bitmap };
  }

  loading.set(0.15, needsResize ? "Scaling it down to size…" : "Converting it…");

  let output = bitmap;
  if (needsResize) {
    const scale = Math.sqrt(budget / pixels);
    output = await createImageBitmap(bitmap, {
      resizeWidth: Math.max(1, Math.round(bitmap.width * scale)),
      resizeHeight: Math.max(1, Math.round(bitmap.height * scale)),
      resizeQuality: "high",
    });
    bitmap.close();
  }

  const canvas = document.createElement("canvas");
  canvas.width = output.width;
  canvas.height = output.height;
  canvas.getContext("2d").drawImage(output, 0, 0);

  const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
  return { blob, width: output.width, height: output.height, bitmap: output };
}

/**
 * Takes a bitmap the browser already holds as the picture on screen.
 *
 * The magnifier needs a colour for every pointer move; asking the server each
 * time would put a network round trip inside a drag. The pixels are read once
 * here instead, and the committed sample still comes from the server so the
 * pipeline's value stays authoritative.
 */
function adoptBitmap(bitmap) {
  state.sourceImage = bitmap;
  state.view.fitted = false;

  const scratch = document.createElement("canvas");
  scratch.width = bitmap.width;
  scratch.height = bitmap.height;
  const ctx = scratch.getContext("2d", { willReadFrequently: true });
  ctx.drawImage(bitmap, 0, 0);
  state.sourcePixels = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
}

/** The same, for a picture the server had to send us. */
async function adoptSourceImage(dataUri) {
  const image = await new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("The picture preview could not be displayed."));
    img.src = dataUri;
  });
  adoptBitmap(image);
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
      ctx.fillStyle = scan.color;
      ctx.fill(scanPath(scan), scan.fillRule || "evenodd");
    }
  }

  if (state.previewMode === "coverage") {
    // §9.2: colour must not be the only way to tell scans apart, so each
    // claimed region is outlined as well as tinted.
    for (const scan of state.scanResults) {
      const path = scanPath(scan);
      ctx.fillStyle = scan.color;
      ctx.globalAlpha = 0.55;
      ctx.strokeStyle = contrastingStroke(scan.color);
      ctx.lineWidth = 1.5 / state.view.scale;
      ctx.setLineDash([4 / state.view.scale, 3 / state.view.scale]);
      ctx.fill(path, scan.fillRule || "evenodd");
      ctx.stroke(path);
    }
    ctx.globalAlpha = 1;
    ctx.setLineDash([]);
  }

  ctx.restore();

  if (state.target) drawTargetMarker(ctx);
}

/**
 * One path per scan, holding every subpath the backend returned.
 *
 * A fill rule decides what is inside by counting the subpaths a ray crosses,
 * which only works when they belong to the same path. Filling them one at a
 * time paints each hole back in as a solid shape.
 */
function scanPath(scan) {
  const path = new Path2D();
  for (const d of scan.pathDatas) path.addPath(new Path2D(d));
  return path;
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

  if (mode === "5x5_median") return rgb255ToHex(...representativeOf(window));

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
  return rgb255ToHex(...representativeOf(winner));
}

/** The pixel of `pixels` closest to their per-channel median — a real colour. */
function representativeOf(pixels) {
  const median = [0, 1, 2].map((channel) => {
    const values = pixels.map((p) => p[channel]).sort((a, b) => a - b);
    const middle = values.length >> 1;
    return values.length % 2 ? values[middle] : (values[middle - 1] + values[middle]) / 2;
  });

  let best = pixels[0];
  let bestDistance = Infinity;
  for (const pixel of pixels) {
    const distance = (pixel[0] - median[0]) ** 2 + (pixel[1] - median[1]) ** 2 + (pixel[2] - median[2]) ** 2;
    if (distance < bestDistance) {
      bestDistance = distance;
      best = pixel;
    }
  }
  return best;
}

// -------------------------------------------------------------------------
// The magnifier (§9.3)
// -------------------------------------------------------------------------

//: How many source pixels across the magnifier shows. Odd, so there is a
//: middle pixel to aim at.
const LOUPE_SOURCE_PIXELS = 13;

function showLoupe(clientX, clientY) {
  const loupe = document.getElementById("loupe");
  const { left, top, width } = viewportSize();

  // Sit the magnifier above the contact point, where a fingertip is not.
  const x = Math.max(80, Math.min(width - 80, clientX - left));
  const y = Math.max(160, clientY - top - 20);
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
  const button = document.getElementById("btn-pick");
  button.classList.toggle("is-on", on);
  button.setAttribute("aria-pressed", String(on));

  if (on) {
    // Aim at the middle of what is on screen, so a keyboard user has somewhere
    // to start from without ever touching a pointer.
    const { left, top, width, height } = viewportSize();
    state.target = clampToImage(screenToImage(left + width / 2, top + height / 2));
    document.getElementById("preview-canvas").focus({ preventScroll: true });
    announce("Picking a colour. Press and drag on the picture, or use the arrow keys and press Enter.");
  } else {
    state.target = null;
    hideLoupe();
    // Picking that was started from the palette sheet returns to it, so adding
    // several colours in a row does not mean reopening the sheet by hand each
    // time. Picking started from the toolbar returns to the picture.
    if (state.pickReturnsToPalette) {
      state.pickReturnsToPalette = false;
      openPaletteSheet();
    }
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
    // The floating controls sit inside the viewport. Capturing the pointer for
    // a pan would retarget the follow-up click away from the button that was
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

      if (wasPicking) {
        hideLoupe();
        if (eventName === "pointerup") commitSample();
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
  return resolvedHex(entry) || "#888888";
}

/**
 * The entry's colour, or null when it does not have one yet.
 *
 * An automatic entry with nothing left to claim never gets a colour resolved
 * for it. Showing the placeholder grey as though it were a swatch would be a
 * colour the picture does not contain, standing in a palette that promises
 * every colour in it is one that does.
 */
function resolvedHex(entry) {
  return (entry.output && entry.output.color && entry.output.color.srgb)
    || (entry.sourceAnchor && entry.sourceAnchor.srgb)
    || null;
}

function scanFor(entryId) {
  return state.scanResults.find((s) => s.entryId === entryId);
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
  const scan = scanFor(entryId);
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
 * Adds a colour to the palette.
 *
 * Adding always makes the palette longer. It used to consume an automatic slot
 * first, which kept the total steady back when a separate number decided how
 * many colours there were. That number is gone: the palette *is* the list, so
 * an add that silently replaced one of its rows would be a lie about what the
 * gesture did. Colours that are not wanted are removed from the palette sheet.
 */
async function addPinnedColour(hex, { name, announceAs } = {}) {
  const entries = state.settings.palette.entries;
  const existing = entries.find((entry) => entry.kind === "pinned"
    && (entry.sourceAnchor || {}).srgb === hex);
  if (existing) {
    announce(`${hex} is already in the palette as ${existing.name}.`);
    return existing;
  }

  if (entries.length >= MAX_ENTRIES) {
    showAlert(`A trace can hold ${MAX_ENTRIES} colours. Remove one before adding another.`);
    return null;
  }

  pushHistory();

  const { r, g, b } = hexToRgb01(hex);
  const { C } = srgbToOklch(r, g, b);

  const target = createEntry(entries.length);
  entries.push(target);
  state.settings.palette.layerOrder.push(target.id);
  state.settings.scanCount = entries.length;

  pinEntryTo(target, hex, C);
  target.name = name || `Picked ${hex}`;

  await pushSettings();
  announce(`${announceAs || `Added ${hex}`}. It now covers ${coverageText(target.id)}.`);
  return target;
}

/**
 * Removes a colour from the palette.
 *
 * Shared by the palette sheet and by one colour's own sheet, so the two cannot
 * drift into removing different amounts of an entry.
 */
async function removeEntry(entry) {
  const entries = state.settings.palette.entries;
  const index = entries.indexOf(entry);
  if (index < 0) return;

  // A trace with no colours has nothing to draw, and the settings schema
  // requires at least one. The buttons that get here are disabled at this
  // point; this guards the path rather than trusting them.
  if (entries.length <= 1) {
    showAlert("A trace needs at least one colour.");
    return;
  }

  pushHistory();
  entries.splice(index, 1);
  state.settings.palette.layerOrder =
    state.settings.palette.layerOrder.filter((id) => id !== entry.id);
  if (state.settings.palette.backgroundEntryId === entry.id) {
    state.settings.palette.backgroundEntryId = null;
  }
  state.settings.scanCount = entries.length;

  await pushSettings();
  announce(`Removed ${entry.name}. ${entries.length} ${entries.length === 1 ? "colour" : "colours"} left.`);
}

function addPickedColour(hex) {
  return addPinnedColour(hex);
}

// -------------------------------------------------------------------------
// Undo (§9.2 does not require it; every tool that edits by trial does)
// -------------------------------------------------------------------------

function snapshot() {
  return JSON.parse(JSON.stringify(state.settings));
}

/**
 * Records a settings state to come back to.
 *
 * `before` lets a caller that has to look at the settings before deciding
 * whether to change them take its snapshot first, so a decision not to change
 * anything does not leave an undo step that undoes nothing.
 */
function pushHistory(before) {
  state.history.past.push(before || snapshot());
  if (state.history.past.length > HISTORY_LIMIT) state.history.past.shift();
  state.history.future.length = 0;
  renderHistoryButtons();
}

async function stepHistory(from, to) {
  if (!from.length) return;
  to.push(snapshot());
  state.settings = from.pop();
  renderHistoryButtons();
  await pushSettings();
  announce(to === state.history.future ? "Undone." : "Redone.");
}

function renderHistoryButtons() {
  document.getElementById("btn-undo").disabled = !state.history.past.length;
  document.getElementById("btn-redo").disabled = !state.history.future.length;
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

//: One trace at a time. Dragging a slider produces a change per step, and
//: queueing a full pipeline run behind each of them would spend minutes
//: computing results nobody will ever see, so only the newest is sent on.
let inFlight = null;
let queued = false;

async function pushSettings() {
  if (inFlight) {
    queued = true;
    return inFlight;
  }

  try {
    do {
      queued = false;
      inFlight = runPush();
      await inFlight;
    } while (queued);
  } finally {
    inFlight = null;
    queued = false;
  }
}

async function runPush() {
  // What is being sent, as it was when it was sent. A change made while the
  // trace is running would otherwise be overwritten by the answer to the
  // question that was asked before it — the run in progress knows nothing
  // about it, and its reply carries the settings as they were.
  const sent = JSON.stringify(state.settings);

  tracing.begin();
  try {
    const data = await api("/api/update_settings", { method: "POST", body: { settings: state.settings } });

    clearAlert();
    if (JSON.stringify(state.settings) === sent) {
      state.settings = data.settings;
    }
    state.scanResults = data.scanResults || [];
    state.claimStats = data.claimStats || {};
    state.warnings = data.warnings || [];
    renderAll();
  } finally {
    tracing.end();
  }
}

// -------------------------------------------------------------------------
// Rendering
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
  const scrolled = active && active.closest ? active.closest(".sheet-body, .palette-scroller") : null;
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
    renderPaletteSheet();
    if (state.openScanId) renderScanSheet(state.openScanId);
  });
  renderDestinationSelection();
  renderSetupControls();
  renderWarnings();
  renderModeCaption();
  renderBackdropButton();
  renderHistoryButtons();
  renderCanvas();
}

/**
 * The palette sheet: every colour in the trace, with a way to remove each one.
 *
 * This is where the number of colours lives now. It reads from the same entries
 * and the same helpers as the swatch strip, so the two views cannot disagree
 * about what is in the palette.
 */
function renderPaletteSheet() {
  const sheet = document.getElementById("sheet-palette");
  if (!sheet || !sheet.open) return;

  const list = document.getElementById("palette-manager-list");
  const entries = state.settings.palette.entries;
  const backgroundId = state.settings.palette.backgroundEntryId;
  const localWarnings = perScanWarnings();
  const onlyOne = entries.length <= 1;

  document.getElementById("palette-count").textContent = onlyOne
    ? "One colour. Add another from the picture below."
    : `${entries.length} colours, back to front.`;

  const row = (options) => `
    <li class="pm-row ${options.className || ""}">
      <span class="pm-chip ${options.hex ? "" : "is-empty"}" aria-hidden="true"
            ${options.hex ? `style="background-color:${escapeHtml(options.hex)}"` : ""}></span>
      <span class="pm-text">
        <span class="pm-name">${escapeHtml(options.name)}</span>
        <span class="pm-meta">${escapeHtml(options.meta)}</span>
      </span>
      ${options.action || ""}
    </li>
  `;

  const removeButton = (entry) => `
    <button type="button" class="btn btn-icon pm-remove" data-remove-id="${escapeHtml(entry.id)}"
            aria-label="Remove ${escapeHtml(entry.name)}" title="Remove"
            ${onlyOne ? "disabled" : ""}>
      <svg class="icon" aria-hidden="true"><use href="#i-trash"/></svg>
    </button>
  `;

  // Layers the pipeline invented — a backdrop-coloured shape lifted in front of
  // what encloses it — are listed so the palette is the whole truth, but they
  // carry no Remove: there is nothing in the settings to remove them from.
  const derived = state.scanResults.filter((scan) => scan.derived);

  list.innerHTML = entries.map((entry) => {
    const notes = [
      entry.kind === "pinned" ? "picked by you" : "chosen for you",
      resolvedHex(entry) || "no colour of its own yet",
      coverageText(entry.id),
      backgroundId === entry.id ? "the backdrop" : "",
      entry.enabled === false ? "left out" : "",
      (localWarnings.get(entry.id) || []).length ? "worth a look" : "",
    ].filter(Boolean);

    return row({
      hex: resolvedHex(entry),
      name: entry.name,
      meta: notes.join(" · "),
      className: entry.enabled === false ? "is-disabled" : "",
      action: removeButton(entry),
    });
  }).join("") + derived.map((scan) => row({
    hex: scan.color,
    name: scan.name,
    meta: "added automatically in front",
    className: "is-derived",
    action: "",
  })).join("");
}

function bindPaletteSheet() {
  const sheet = document.getElementById("sheet-palette");

  sheet.addEventListener("click", async (e) => {
    const remove = e.target.closest("[data-remove-id]");
    if (!remove) return;
    const entry = findEntry(remove.dataset.removeId);
    if (!entry) return;
    await removeEntry(entry);
    // The button that was just used no longer exists, so focus has to be put
    // somewhere deliberate or a keyboard user is dropped back on <body>.
    document.getElementById("btn-palette-pick").focus();
  });

  // The picture is behind this dialog, and a modal <dialog> is exactly what
  // stops it being touched. Picking therefore closes the sheet, hands the
  // gesture to the picture, and comes back when picking ends.
  document.getElementById("btn-palette-pick").addEventListener("click", () => {
    closeDialog(sheet);
    state.pickReturnsToPalette = true;
    setPicking(true);
  });
}

function openPaletteSheet() {
  openDialog(document.getElementById("sheet-palette"));
  renderPaletteSheet();
}

function renderSwatches() {
  const list = document.getElementById("swatch-list");
  const entries = state.settings.palette.entries;
  const backgroundId = state.settings.palette.backgroundEntryId;

  // Layers the pipeline invented — a backdrop-coloured shape lifted in front of
  // what encloses it — are shown so the result is never a surprise, but they
  // carry no controls: there is nothing in the settings for them to write to.
  const derived = state.scanResults.filter((scan) => scan.derived);
  const localWarnings = perScanWarnings();

  const swatch = (options) => {
    const coverage = Math.max(0, Math.min(100, options.coverage || 0));
    const chip = options.hex
      ? `<span class="swatch-chip" style="background-color:${escapeHtml(options.hex)}" aria-hidden="true"></span>`
      : `<span class="swatch-chip is-empty" aria-hidden="true"></span>`;
    return `
      <li>
        <button type="button" class="swatch ${options.className || ""}"
                data-entry-id="${escapeHtml(options.id)}"
                ${options.draggable ? 'data-draggable="true"' : ""}
                data-disabled="${!!options.disabled}"
                aria-label="${escapeHtml(options.aria)}">
          ${chip}
          <span class="swatch-flags" aria-hidden="true">${options.flags || ""}</span>
          <span class="swatch-name">${escapeHtml(options.name)}</span>
          <span class="swatch-meta">
            <span class="swatch-hex">${escapeHtml(options.hex || "nothing left")}</span>
            <span class="swatch-bar"><span style="width:${coverage.toFixed(1)}%"></span></span>
          </span>
        </button>
      </li>
    `;
  };

  const flagIcon = (icon) => `<svg class="icon" aria-hidden="true"><use href="#${icon}"/></svg>`;

  list.innerHTML = entries.map((entry, index) => {
    const scan = scanFor(entry.id);
    const flags = [
      entry.kind === "pinned" ? flagIcon("i-lock") : "",
      backgroundId === entry.id ? flagIcon("i-backdrop") : "",
      entry.enabled === false ? flagIcon("i-eye-off") : "",
      (localWarnings.get(entry.id) || []).length ? flagIcon("i-warn") : "",
    ].join("");

    const aria = [
      entry.name,
      resolvedHex(entry) || "no colour of its own yet",
      coverageText(entry.id),
      entry.kind === "pinned" ? "picked by you" : "chosen for you",
      backgroundId === entry.id ? "the backdrop" : "",
      entry.enabled === false ? "left out" : "",
      `layer ${index + 1} of ${entries.length}`,
    ].filter(Boolean).join(", ");

    return swatch({
      id: entry.id,
      hex: resolvedHex(entry),
      name: entry.name,
      coverage: coverageOf(entry.id) || 0,
      disabled: entry.enabled === false,
      draggable: true,
      className: backgroundId === entry.id ? "is-background" : "",
      flags,
      aria,
    });
  }).join("") + derived.map((scan) => swatch({
    id: scan.entryId,
    hex: scan.color,
    name: scan.name,
    coverage: scan.coveragePercent,
    className: "is-derived",
    flags: flagIcon("i-wand"),
    aria: `${scan.name}, ${scan.color}, added automatically in front`,
  })).join("") + `
    <li>
      <button type="button" class="swatch swatch-add" id="btn-add-swatch"
              aria-label="Add a colour from the picture" title="Add a colour from the picture">
        <svg class="icon" aria-hidden="true"><use href="#i-dropper"/></svg>
      </button>
    </li>
  `;
}

/**
 * The warnings that are genuinely about one scan.
 *
 * A backend limitation produces the identical sentence on every scan at once.
 * Flagging all four colours for it says nothing about any of them, and turns
 * the one flag that would mean something into wallpaper.
 */
function perScanWarnings() {
  const scans = state.scanResults;
  const counts = new Map();
  for (const scan of scans) {
    for (const message of scan.warnings || []) {
      counts.set(message, (counts.get(message) || 0) + 1);
    }
  }

  const local = new Map();
  for (const scan of scans) {
    local.set(scan.entryId, (scan.warnings || []).filter(
      (message) => counts.get(message) < scans.length
    ));
  }
  return local;
}


function renderWarnings() {
  // One backend limitation produces the same sentence once per scan. Repeating
  // it four times makes the interface look broken and says nothing extra; the
  // per-colour sheets still show which scans are affected.
  const unique = [...new Set(state.warnings.map((w) => w.message || String(w)))];

  const summary = document.getElementById("warnings-summary");
  summary.hidden = unique.length === 0;
  summary.textContent = unique.join(" · ");

  document.getElementById("btn-warnings").hidden = unique.length === 0;
  document.getElementById("warning-list").innerHTML = unique.map((message) => `
    <li><svg class="icon" aria-hidden="true"><use href="#i-warn"/></svg><span>${escapeHtml(message)}</span></li>
  `).join("");
}

function renderModeCaption() {
  document.getElementById("mode-caption").textContent = MODE_CAPTIONS[state.previewMode] || "";
  for (const button of document.querySelectorAll("[data-mode]")) {
    button.setAttribute("aria-checked", String(button.dataset.mode === state.previewMode));
  }
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

function renderSetupControls() {
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

  const check = (name, value) => {
    const input = document.querySelector(`input[name="${name}"][value="${value}"]`);
    if (input) input.checked = true;
  };
  check("bg-matching", settings.palette.backgroundMatching || "all_matching");
  check("bg-output", settings.output.backgroundOutput || "keep_paths");
  check("bg-foreground", settings.palette.backgroundForegroundPolicy || "hole_or_layer");

  // The hole-or-layer question only arises when the matching mode can leave
  // backdrop-coloured pixels out of the backdrop in the first place.
  document.getElementById("background-foreground-set").hidden =
    (settings.palette.backgroundMatching || "all_matching") === "all_matching";
}

function renderBackdropButton() {
  const button = document.getElementById("btn-auto-backdrop");
  const current = state.settings.palette.backgroundEntryId;
  const entry = current && findEntry(current);

  button.classList.toggle("is-on", !!entry);
  button.setAttribute("aria-pressed", String(!!entry));
  const wording = entry
    ? `${entry.name} is the backdrop. Tap to clear it.`
    : "Use the colour around the edge as the backdrop";
  button.setAttribute("aria-label", wording);
  button.title = wording;
}

// -------------------------------------------------------------------------
// The one-colour sheet
// -------------------------------------------------------------------------

function openScanSheet(entryId) {
  state.openScanId = entryId;
  renderScanSheet(entryId);
  openDialog(document.getElementById("sheet-scan"));
}

function iconButton({ action, icon, text, extra = "", className = "btn-secondary" }) {
  return `
    <button type="button" class="btn ${className}" data-action="${action}" ${extra}>
      <svg class="icon" aria-hidden="true"><use href="#${icon}"/></svg>
      <span>${escapeHtml(text)}</span>
    </button>
  `;
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
  const scan = scanFor(entry.id);
  const warnings = perScanWarnings().get(entry.id) || [];
  const border = scan ? scan.borderSharePercent : null;

  document.getElementById("sheet-scan-title").textContent = entry.name;
  document.getElementById("sheet-scan-body").innerHTML = `
    <div class="scan-hero">
      <span class="scan-hero-chip" style="background-color:${escapeHtml(hex)}" aria-hidden="true"></span>
      <span class="scan-hero-text">
        <span class="scan-hero-hex">${escapeHtml(hex)}</span>
        <span class="scan-hero-fact">
          ${entry.kind === "pinned" ? "You chose this. It comes out exactly as shown." : "Chosen for you from what was left over."}
        </span>
        <span class="scan-hero-fact">Covers ${coverageText(entry.id)}${
          typeof border === "number" && border >= 40 ? `, and ${Math.round(border)}% of the frame` : ""
        }.</span>
      </span>
    </div>

    ${warnings.length ? `<p class="scan-warning">${escapeHtml(warnings.join(" "))}</p>` : ""}

    <div class="scan-actions">
      ${iconButton({ action: "edit-colour", icon: "i-dropper", text: "Change" })}
      ${iconButton({
        action: "toggle-background", icon: "i-backdrop",
        text: isBackground ? "Not backdrop" : "Backdrop",
        className: isBackground ? "btn-secondary is-on btn-accent" : "btn-secondary",
      })}
      ${iconButton({
        action: "toggle-enabled",
        icon: entry.enabled === false ? "i-eye-off" : "i-eye",
        text: entry.enabled === false ? "Left out" : "Included",
      })}
      ${iconButton({
        action: "remove", icon: "i-trash", text: "Remove",
        className: "btn-danger", extra: entries.length <= 1 ? "disabled" : "",
      })}
    </div>

    <div class="field">
      <label for="scan-name">Call it</label>
      <input type="text" id="scan-name" value="${escapeHtml(entry.name)}" data-field="name">
    </div>

    <div class="field">
      <span class="field-help">Layer ${index + 1} of ${entries.length}, counting from the back.</span>
      <div class="scan-actions">
        <button type="button" class="btn btn-secondary" data-move="-1" ${index === 0 ? "disabled" : ""}>
          <svg class="icon" aria-hidden="true"><use href="#i-send-back"/></svg>
          <span>Further back</span>
        </button>
        <button type="button" class="btn btn-secondary" data-move="1" ${index === entries.length - 1 ? "disabled" : ""}>
          <svg class="icon" aria-hidden="true"><use href="#i-bring-front"/></svg>
          <span>Further forward</span>
        </button>
      </div>
    </div>

    ${entry.kind === "automatic" ? `
      <button type="button" class="btn btn-accent btn-wide" data-action="lock-exact">
        <svg class="icon" aria-hidden="true"><use href="#i-lock"/></svg>
        <span>Lock this colour in</span>
      </button>
      <p class="card-hint">
        Chosen colours shift when you change anything else. Locking keeps this
        exact colour and lets it reserve the pixels around it.
      </p>
    ` : renderReachControls(entry)}

    ${isBackground ? `
      <p class="card-hint">This is the backdrop colour. Its role is fixed — change how it behaves under Setup.</p>
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
      pushHistory();
      reorderEntry(entry.id, parseInt(move.dataset.move, 10));
      await pushSettings();
      announce(`${entry.name} moved.`);
      return;
    }

    const action = e.target.closest("[data-action]");
    if (!action) return;

    switch (action.dataset.action) {
      case "lock-exact": {
        pushHistory();
        const hex = entryHex(entry);
        const { r, g, b } = hexToRgb01(hex);
        pinEntryTo(entry, hex, srgbToOklch(r, g, b).C);
        await pushSettings();
        announce(`${entry.name} is locked to ${hex}.`);
        break;
      }
      case "edit-colour":
        openColourSheet(entry);
        break;
      case "toggle-background":
        pushHistory();
        await setBackgroundEntry(
          state.settings.palette.backgroundEntryId === entry.id ? null : entry.id
        );
        break;
      case "toggle-enabled":
        pushHistory();
        entry.enabled = entry.enabled === false;
        await pushSettings();
        announce(`${entry.name} is ${entry.enabled ? "included" : "left out"}.`);
        break;
      case "remove":
        state.openScanId = null;
        closeDialog(sheet);
        await removeEntry(entry);
        break;
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

    const before = snapshot();

    switch (field) {
      case "name":
        entry.name = e.target.value.trim() || entry.name;
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
            values: JSON.parse(JSON.stringify(state.settings.globalTraceProfile)),
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
        entry.traceProfile.values = JSON.parse(JSON.stringify(base));
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

    pushHistory(before);
    await pushSettings();
  });
}

function reorderEntry(id, direction) {
  const entries = state.settings.palette.entries;
  const index = entries.findIndex((e) => e.id === id);
  moveEntryTo(id, index + direction);
}

/** Moves an entry to an absolute position, keeping layer order in step. */
function moveEntryTo(id, position) {
  const entries = state.settings.palette.entries;
  const from = entries.findIndex((e) => e.id === id);
  const to = Math.max(0, Math.min(entries.length - 1, position));
  if (from < 0 || from === to) return false;

  entries.splice(to, 0, entries.splice(from, 1)[0]);

  // Layer order is the list that actually drives stacking (§10.4); the visible
  // list is kept in step with it so a moved row stays where the user put it.
  state.settings.palette.layerOrder = entries.map((entry) => entry.id);
  return true;
}

// -------------------------------------------------------------------------
// Typing a colour
//
// Only ever to *change* a colour already in the palette, never to create one.
// Colours enter the palette from the picture, through the picker. Typing stays
// because a screen print or a vinyl cut sometimes has to hit an exact stated
// value, which no amount of aiming at a photograph will land on.
// -------------------------------------------------------------------------

/** Opens the colour sheet to change `entry`. */
function openColourSheet(entry) {
  state.colourSheet = { entryId: entry.id };

  document.getElementById("sheet-colour-title").textContent = `Change ${entry.name}`;

  const start = entryHex(entry);
  document.getElementById("colour-input").value = start;
  document.getElementById("colour-native").value = start.toLowerCase();
  renderColourPreview();

  openDialog(document.getElementById("sheet-colour"));
  document.getElementById("colour-input").select();
}

function renderColourPreview() {
  const text = document.getElementById("colour-input").value;
  const parsed = parseColorInput(text);
  const chip = document.getElementById("colour-preview-chip");
  const hexLabel = document.getElementById("colour-preview-hex");
  const note = document.getElementById("colour-preview-note");
  const formats = document.getElementById("colour-formats");
  const confirm = document.getElementById("btn-colour-confirm");

  if (!parsed) {
    chip.style.backgroundColor = "transparent";
    hexLabel.textContent = "—";
    note.textContent = text.trim() ? "Not a colour this understands." : "Type or paste a colour.";
    note.classList.toggle("is-bad", !!text.trim());
    formats.innerHTML = "";
    confirm.disabled = true;
    return;
  }

  const hex = rgb01ToHex(parsed);
  chip.style.backgroundColor = hex;
  hexLabel.textContent = hex;
  note.textContent = "";
  note.classList.remove("is-bad");
  confirm.disabled = false;
  document.getElementById("colour-native").value = hex.toLowerCase();

  formats.innerHTML = colorNotations(parsed).map((notation) => `
    <button type="button" class="format-chip" data-notation="${escapeHtml(notation)}"
            title="Use this notation">${escapeHtml(notation)}</button>
  `).join("");
}

async function confirmColourSheet() {
  const parsed = parseColorInput(document.getElementById("colour-input").value);
  if (!parsed) return;

  let hex = rgb01ToHex(parsed);
  const snap = document.getElementById("colour-snap").checked;

  if (snap && state.session && state.session.hasImage) {
    try {
      const nearest = await api("/api/nearest_source_color", { method: "POST", body: { hex } });
      if (nearest.hex && nearest.hex !== hex) {
        announce(`${hex} is not in the picture; using ${nearest.hex}, the closest colour that is.`);
        hex = nearest.hex;
      }
    } catch {
      // The typed colour stands; api() has already said why the lookup failed.
    }
  }

  const target = state.colourSheet && findEntry(state.colourSheet.entryId);

  closeDialog(document.getElementById("sheet-colour"));
  if (!target) return;

  pushHistory();
  const { r, g, b } = hexToRgb01(hex);
  pinEntryTo(target, hex, srgbToOklch(r, g, b).C);
  await pushSettings();
  announce(`${target.name} is now ${hex}.`);
}

function bindColourSheet() {
  const sheet = document.getElementById("sheet-colour");
  const input = document.getElementById("colour-input");

  input.addEventListener("input", renderColourPreview);
  document.getElementById("colour-native").addEventListener("input", (e) => {
    input.value = e.target.value.toUpperCase();
    renderColourPreview();
  });

  document.getElementById("colour-formats").addEventListener("click", (e) => {
    const chip = e.target.closest("[data-notation]");
    if (!chip) return;
    input.value = chip.dataset.notation;
    renderColourPreview();
    input.focus();
  });

  document.getElementById("form-colour").addEventListener("submit", (e) => {
    e.preventDefault();
    confirmColourSheet().catch((err) => showAlert(err.message));
  });

  sheet.addEventListener("close", () => { state.colourSheet = null; });
}

// -------------------------------------------------------------------------
// Reordering by long press
//
// Holding a swatch lifts it out of the strip; dragging slides the others aside
// to show where it will land. The press has to be long enough not to fire on a
// scroll of the strip, and short enough not to feel stuck.
// -------------------------------------------------------------------------

const LIFT_DELAY_MS = 320;
const LIFT_CANCEL_PX = 12;

function bindSwatchReordering() {
  const list = document.getElementById("swatch-list");
  let press = null;
  let drag = null;

  const cancelPress = () => {
    if (press) clearTimeout(press.timer);
    press = null;
  };

  const beginDrag = () => {
    if (!press) return;
    const items = [...list.querySelectorAll('[data-draggable="true"]')];
    const index = items.indexOf(press.swatch);
    if (index < 0 || items.length < 2) return cancelPress();

    const first = items[0].getBoundingClientRect();
    const second = items[1].getBoundingClientRect();
    const vertical = Math.abs(second.top - first.top) > Math.abs(second.left - first.left);
    const stride = vertical ? second.top - first.top : second.left - first.left;

    drag = { items, index, target: index, vertical, stride, swatch: press.swatch };
    list.classList.add("is-reordering");
    press.swatch.classList.add("is-lifted");
    haptic(12);
    announce(`Moving ${press.swatch.getAttribute("aria-label")}. Drag to reorder.`);
  };

  list.addEventListener("pointerdown", (e) => {
    const swatch = e.target.closest('[data-draggable="true"]');
    if (!swatch || e.button > 0) return;

    press = { swatch, x: e.clientX, y: e.clientY, pointerId: e.pointerId, moved: false };
    press.timer = setTimeout(beginDrag, LIFT_DELAY_MS);
    swatch.setPointerCapture(e.pointerId);
  });

  // `touch-action` cannot be changed once a gesture has begun, so the only way
  // to stop the strip scrolling out from under a lifted swatch is to refuse the
  // touch itself. The hold is stationary, so nothing has started scrolling by
  // the time this begins refusing.
  list.addEventListener("touchmove", (e) => {
    if (drag) e.preventDefault();
  }, { passive: false });

  list.addEventListener("pointermove", (e) => {
    if (!press || e.pointerId !== press.pointerId) return;

    const dx = e.clientX - press.x;
    const dy = e.clientY - press.y;

    if (!drag) {
      // Movement before the hold completes is a scroll of the strip, not a
      // drag of one swatch.
      if (Math.hypot(dx, dy) > LIFT_CANCEL_PX) {
        press.moved = true;
        cancelPress();
      }
      return;
    }

    e.preventDefault();
    const travel = drag.vertical ? dy : dx;
    drag.swatch.style.transform = `translate(${drag.vertical ? 0 : dx}px, ${drag.vertical ? dy : 0}px) scale(1.06)`;

    const wanted = Math.max(0, Math.min(
      drag.items.length - 1,
      drag.index + Math.round(travel / drag.stride)
    ));
    if (wanted === drag.target) return;

    drag.target = wanted;
    drag.items.forEach((item, index) => {
      if (item === drag.swatch) return;
      let shift = 0;
      if (index > drag.index && index <= wanted) shift = -drag.stride;
      if (index < drag.index && index >= wanted) shift = drag.stride;
      item.style.transform = drag.vertical ? `translateY(${shift}px)` : `translateX(${shift}px)`;
    });
  });

  for (const eventName of ["pointerup", "pointercancel"]) {
    list.addEventListener(eventName, async (e) => {
      if (!press || e.pointerId !== press.pointerId) return;

      const wasDragging = drag;
      const tapped = !drag && !press.moved && eventName === "pointerup";
      const swatch = press.swatch;
      cancelPress();

      if (wasDragging) {
        list.classList.remove("is-reordering");
        for (const item of wasDragging.items) item.style.transform = "";
        swatch.classList.remove("is-lifted");
        drag = null;

        if (wasDragging.target !== wasDragging.index) {
          pushHistory();
          moveEntryTo(swatch.dataset.entryId, wasDragging.target);
          haptic(8);
          await pushSettings();
          announce(`Moved to layer ${wasDragging.target + 1}.`);
        }
        return;
      }

      if (tapped) openScanSheet(swatch.dataset.entryId);
    });
  }

  // The same move, for anyone not holding a pointer (§29).
  list.addEventListener("keydown", async (e) => {
    const swatch = e.target.closest('[data-draggable="true"]');
    if (!swatch || !e.altKey) return;

    const direction = { ArrowLeft: -1, ArrowUp: -1, ArrowRight: 1, ArrowDown: 1 }[e.key];
    if (!direction) return;

    e.preventDefault();
    pushHistory();
    reorderEntry(swatch.dataset.entryId, direction);
    await pushSettings();
    announce(direction < 0 ? "Moved further back." : "Moved further forward.");
  });

  list.addEventListener("click", (e) => {
    if (e.target.closest("#btn-add-swatch")) setPicking(true);
  });
}

// -------------------------------------------------------------------------
// Workspace controls
// -------------------------------------------------------------------------

function bindWorkspace() {
  bindStageGestures();
  bindScanSheet();
  bindColourSheet();
  bindPaletteSheet();
  bindSwatchReordering();

  document.getElementById("btn-pick").addEventListener("click", () => setPicking(!state.picking));
  document.getElementById("btn-stop-picking").addEventListener("click", () => setPicking(false));
  document.getElementById("btn-colours").addEventListener("click", openPaletteSheet);

  document.querySelector(".pickbar .segmented").addEventListener("click", (e) => {
    const button = e.target.closest("[data-sample]");
    if (!button) return;
    state.sampleMode = button.dataset.sample;
    for (const other of document.querySelectorAll("[data-sample]")) {
      other.setAttribute("aria-checked", String(other === button));
    }
    if (state.target) drawLoupe();
  });

  document.querySelector(".stage-modes").addEventListener("click", (e) => {
    const button = e.target.closest("[data-mode]");
    if (!button) return;
    state.previewMode = button.dataset.mode;
    renderModeCaption();
    renderCanvas();
  });

  document.getElementById("btn-zoom-in").addEventListener("click", () => zoomBy(1.25));
  document.getElementById("btn-zoom-out").addEventListener("click", () => zoomBy(0.8));
  document.getElementById("btn-zoom-fit").addEventListener("click", fitView);
  document.getElementById("btn-zoom-actual").addEventListener("click", () => {
    const { width, height } = viewportSize();
    setScaleAbout(1, width / 2, height / 2);
  });

  document.getElementById("btn-auto-backdrop").addEventListener("click", toggleAutoBackdrop);
  document.getElementById("btn-tune").addEventListener("click", () => {
    openDialog(document.getElementById("sheet-setup"));
  });
  document.getElementById("btn-warnings").addEventListener("click", () => {
    openDialog(document.getElementById("sheet-warnings"));
  });

  document.getElementById("btn-undo").addEventListener("click", () => {
    stepHistory(state.history.past, state.history.future);
  });
  document.getElementById("btn-redo").addEventListener("click", () => {
    stepHistory(state.history.future, state.history.past);
  });

  document.getElementById("destination-list").addEventListener("click", async (e) => {
    const button = e.target.closest("[data-destination]");
    if (!button) return;
    pushHistory();
    tracing.begin();
    try {
      const data = await api("/api/apply_destination", {
        method: "POST",
        body: { destinationId: button.dataset.destination },
      });
      applyServerResponse(data);
    } finally {
      tracing.end();
    }
    announce(`Set up for ${DESTINATIONS[button.dataset.destination].title.toLowerCase()}.`);
  });

  document.getElementById("select-backend").addEventListener("change", async (e) => {
    pushHistory();
    state.settings.backend.preferredBackendId = e.target.value;
    await pushSettings();
    announce("Tracing engine changed.");
  });

  document.getElementById("select-background-entry").addEventListener("change", (e) => {
    pushHistory();
    setBackgroundEntry(e.target.value || null);
  });

  for (const [name, apply] of [
    ["bg-matching", (value) => { state.settings.palette.backgroundMatching = value; }],
    ["bg-output", (value) => { state.settings.output.backgroundOutput = value; }],
    ["bg-foreground", (value) => { state.settings.palette.backgroundForegroundPolicy = value; }],
  ]) {
    for (const input of document.querySelectorAll(`input[name="${name}"]`)) {
      input.addEventListener("change", async (e) => {
        if (!e.target.checked) return;
        pushHistory();
        apply(e.target.value);
        await pushSettings();
      });
    }
  }

  document.getElementById("btn-reset-destination").addEventListener("click", async () => {
    pushHistory();
    tracing.begin();
    try {
      const data = await api("/api/reset_destination_defaults", { method: "POST" });
      applyServerResponse(data);
    } finally {
      tracing.end();
    }
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

  document.getElementById("btn-commit").addEventListener("click", commit);
  document.getElementById("btn-cancel").addEventListener("click", cancel);

  document.addEventListener("keydown", onGlobalKeyDown);
}

function onGlobalKeyDown(e) {
  if (document.body.dataset.view !== "workspace") return;

  const target = e.target instanceof Element ? e.target : null;
  const typing = target && target.matches("input, textarea, select");

  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
    e.preventDefault();
    if (e.shiftKey) stepHistory(state.history.future, state.history.past);
    else stepHistory(state.history.past, state.history.future);
    return;
  }

  // A single letter is a shortcut only where it cannot also be typing.
  if (typing || e.altKey || e.ctrlKey || e.metaKey) return;
  if (document.querySelector("dialog[open]")) return;

  if (e.key === "i" || e.key === "p") { e.preventDefault(); setPicking(!state.picking); }
  if (e.key === "Escape" && state.picking) { e.preventDefault(); setPicking(false); }
}

async function setBackgroundEntry(entryId) {
  for (const entry of state.settings.palette.entries) {
    if (entry.id === entryId) {
      entry.role = "background";
    } else if (entry.role === "background") {
      entry.role = "primary_fill";
    }
  }
  state.settings.palette.backgroundEntryId = entryId || null;

  // A backdrop is what everything else is in front of. Left where it was in
  // the stack it would paint over the picture, which is not what anyone means
  // by the word — so choosing one sends it to the back.
  if (entryId) moveEntryTo(entryId, 0);

  await pushSettings();
  const entry = entryId && findEntry(entryId);
  announce(entry ? `${entry.name} is now the backdrop, behind everything else.` : "No backdrop colour.");
}

/**
 * One tap to a backdrop.
 *
 * Whatever runs around the edge of a picture is almost always its backdrop, so
 * the scan owning the most of the frame is the candidate, and `edge_connected`
 * matching comes with it — that is the setting that stops an enclosed patch of
 * the same colour from being swallowed.
 */
async function toggleAutoBackdrop() {
  if (state.settings.palette.backgroundEntryId) {
    pushHistory();
    await setBackgroundEntry(null);
    return;
  }

  const candidates = state.scanResults
    .filter((scan) => !scan.derived && findEntry(scan.entryId))
    .sort((a, b) => (b.borderSharePercent || 0) - (a.borderSharePercent || 0));

  if (!candidates.length || (candidates[0].borderSharePercent || 0) < 25) {
    showAlert("No colour owns enough of the edge to be an obvious backdrop. Choose one under Setup.");
    return;
  }

  pushHistory();
  state.settings.palette.backgroundMatching = "edge_connected";
  await setBackgroundEntry(candidates[0].entryId);
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
      pushHistory();
      tracing.begin();
      try {
        const data = await api("/api/user_presets/apply", { method: "POST", body: { presetUuid } });
        applyServerResponse(data);
      } finally {
        tracing.end();
      }
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

  tracing.begin();
  try {
    const data = await api("/api/resolve_source_change", { method: "POST", body: { action } });
    applyServerResponse(data);
  } finally {
    tracing.end();
  }
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
