// Palette Trace frontend client (SPEC §9).
"use strict";

const urlParams = new URLSearchParams(window.location.search);
const sessionToken = urlParams.get("token");

// -------------------------------------------------------------------------
// Labels
// -------------------------------------------------------------------------

const DESTINATION_LABELS = {
  illustration: "Illustration",
  logo_branding: "Logo / branding",
  screen_printing: "Screen printing",
  vinyl_paper: "Vinyl / paper cutting",
  laser: "Laser",
  custom: "Custom",
};

const PROFILE_LABELS = {
  default: "Default",
  smooth_shapes: "Smooth shapes",
  sharp_details: "Sharp details",
  thin_line_art: "Thin line art",
  small_accents: "Small accents",
  simplified_background: "Simplified background",
  fabrication_clean: "Fabrication clean",
};

const ROLE_LABELS = {
  outline: "Outline",
  primary_fill: "Primary fill",
  secondary_fill: "Secondary fill",
  accent: "Accent",
  highlight: "Highlight",
  shadow: "Shadow",
  operation: "Operation",
  custom: "Custom",
};

function label(map, id) {
  return map[id] || (id ? id.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase()) : "");
}

// -------------------------------------------------------------------------
// State
// -------------------------------------------------------------------------

const state = {
  settings: null,
  destinationPresets: { order: [], presets: {} },
  traceProfiles: { order: [], profiles: {} },
  userPresets: [],
  scanResults: [],
  claimStats: {},
  warnings: [],
  previewMode: "source",
  sourceImage: new Image(),
  lastFocusedBeforeDialog: null,
};

// -------------------------------------------------------------------------
// Colour math — mirrors palette_trace/color/conversion.py exactly so the
// interface can compute the same §13.4 low-chroma guardrail the pipeline
// applies, without a network round trip on every pointer move.
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
  const clean = hex.replace("#", "");
  return {
    r: parseInt(clean.slice(0, 2), 16) / 255,
    g: parseInt(clean.slice(2, 4), 16) / 255,
    b: parseInt(clean.slice(4, 6), 16) / 255,
  };
}

// -------------------------------------------------------------------------
// API client
// -------------------------------------------------------------------------

function headers() {
  return { "Content-Type": "application/json", "X-Session-Token": sessionToken };
}

async function api(path, { method = "GET", body } = {}) {
  const res = await fetch(path, {
    method,
    headers: headers(),
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  let data;
  try {
    data = await res.json();
  } catch {
    data = {};
  }
  if (!res.ok) {
    const message = data.error || `Request to ${path} failed (${res.status}).`;
    showAlert(message);
    throw new Error(message);
  }
  return data;
}

// -------------------------------------------------------------------------
// Status / alert announcements (§29: dynamic status uses accessible live regions)
// -------------------------------------------------------------------------

function announceStatus(message) {
  document.getElementById("status-region").textContent = message;
}

function showAlert(message) {
  const el = document.getElementById("alert-region");
  el.textContent = message;
  el.hidden = false;
}

function clearAlert() {
  const el = document.getElementById("alert-region");
  el.hidden = true;
  el.textContent = "";
}

// -------------------------------------------------------------------------
// Focus-preserving re-render — a settings change re-renders the whole
// palette list; without this, focus would be dropped to <body> after every
// edit, which breaks keyboard use far worse than a mouse user would notice.
// -------------------------------------------------------------------------

function withFocusPreserved(containerId, renderFn) {
  const active = document.activeElement;
  const activeId = active && active.id;
  const selectionStart = active && "selectionStart" in active ? active.selectionStart : null;
  renderFn();
  if (activeId) {
    const restored = document.getElementById(activeId);
    if (restored) {
      restored.focus();
      if (selectionStart !== null && "setSelectionRange" in restored) {
        try {
          restored.setSelectionRange(selectionStart, selectionStart);
        } catch {
          /* not a text-selectable input; ignore */
        }
      }
    }
  }
}

// -------------------------------------------------------------------------
// Init
// -------------------------------------------------------------------------

document.addEventListener("DOMContentLoaded", () => {
  if (!sessionToken) {
    showAlert("Missing session token. Reopen this interface from the extension or standalone host.");
    return;
  }
  init();
});

async function init() {
  bindStaticControls();
  bindDialogs();

  const [session, destinations, profiles] = await Promise.all([
    api("/api/session"),
    api("/api/destination_presets"),
    api("/api/trace_profiles"),
  ]);

  state.settings = session.settings;
  state.destinationPresets = destinations;
  state.traceProfiles = profiles;

  populateDestinationOptions();

  const imgData = await api("/api/preview_source");
  state.sourceImage.src = imgData.dataUri;
  state.sourceImage.onload = () => {
    syncControlsFromSettings();
    renderCanvas();
  };

  await refreshUserPresets();

  const initialRun = await api("/api/update_settings", { method: "POST", body: { settings: state.settings } });
  applyServerResponse(initialRun);

  if (session.sourceChanged) {
    openSourceChangedDialog();
  }
}

// -------------------------------------------------------------------------
// Applying a server response (every mutating endpoint returns this shape)
// -------------------------------------------------------------------------

function applyServerResponse(data) {
  clearAlert();
  state.settings = data.settings;
  state.scanResults = data.scanResults || [];
  state.claimStats = data.claimStats || {};
  state.warnings = data.warnings || [];
  syncControlsFromSettings();
  withFocusPreserved("palette-list", renderPaletteList);
  renderWarnings();
  renderCanvas();
}

async function pushSettings() {
  const data = await api("/api/update_settings", { method: "POST", body: { settings: state.settings } });
  applyServerResponse(data);
  announceStatus("Settings updated.");
}

// -------------------------------------------------------------------------
// Static controls: destination, scan count, backend, reset button
// -------------------------------------------------------------------------

function populateDestinationOptions() {
  const select = document.getElementById("select-destination");
  select.innerHTML = "";
  for (const id of state.destinationPresets.order) {
    const opt = document.createElement("option");
    opt.value = id;
    opt.textContent = label(DESTINATION_LABELS, id);
    select.appendChild(opt);
  }
}

function bindStaticControls() {
  document.getElementById("select-destination").addEventListener("change", async (e) => {
    const data = await api("/api/apply_destination", {
      method: "POST",
      body: { destinationId: e.target.value },
    });
    applyServerResponse(data);
    announceStatus(`Destination set to ${label(DESTINATION_LABELS, e.target.value)}.`);
  });

  document.getElementById("btn-reset-destination").addEventListener("click", async () => {
    const data = await api("/api/reset_destination_defaults", { method: "POST" });
    applyServerResponse(data);
    announceStatus("Reset to destination defaults.");
  });

  document.getElementById("input-scans").addEventListener("change", (e) => {
    const value = Math.max(1, Math.min(64, parseInt(e.target.value, 10) || 1));
    e.target.value = value;
    state.settings.scanCount = value;
    pushSettings();
  });

  document.getElementById("select-backend").addEventListener("change", (e) => {
    state.settings.backend.preferredBackendId = e.target.value;
    pushSettings();
  });

  document.getElementById("select-background-entry").addEventListener("change", (e) => {
    setBackgroundEntry(e.target.value || null);
  });

  for (const input of document.querySelectorAll('input[name="bg-matching"]')) {
    input.addEventListener("change", (e) => {
      if (!e.target.checked) return;
      state.settings.palette.backgroundMatching = e.target.value;
      pushSettings();
    });
  }

  for (const input of document.querySelectorAll('input[name="bg-output"]')) {
    input.addEventListener("change", (e) => {
      if (!e.target.checked) return;
      state.settings.output.backgroundOutput = e.target.value;
      pushSettings();
    });
  }

  for (const input of document.querySelectorAll('input[name="preview-mode"]')) {
    input.addEventListener("change", (e) => {
      if (!e.target.checked) return;
      state.previewMode = e.target.value;
      renderPreviewModeDescription();
      renderCanvas();
    });
  }

  document.getElementById("btn-apply").addEventListener("click", async () => {
    await api("/api/apply", { method: "POST" });
    window.close();
  });

  document.getElementById("btn-cancel").addEventListener("click", async () => {
    await api("/api/cancel", { method: "POST" });
    window.close();
  });

  document.getElementById("btn-zoom-in").addEventListener("click", () => zoomBy(1.25));
  document.getElementById("btn-zoom-out").addEventListener("click", () => zoomBy(0.8));
  document.getElementById("btn-zoom-reset").addEventListener("click", () => setZoom(null));

  const canvas = document.getElementById("preview-canvas");
  canvas.addEventListener("click", onCanvasClick);

  document.getElementById("btn-load-preset").addEventListener("click", openLoadPresetDialog);
  document.getElementById("btn-save-preset").addEventListener("click", openSavePresetDialog);

  document.getElementById("palette-list").addEventListener("click", onPaletteListClick);
  document.getElementById("palette-list").addEventListener("change", onPaletteListChange);
  document.getElementById("palette-list").addEventListener("input", onPaletteListInput);

  renderPreviewModeDescription();
}

function setBackgroundEntry(entryId) {
  const entries = state.settings.palette.entries;
  for (const entry of entries) {
    if (entry.id === entryId) {
      entry.role = "background";
    } else if (entry.role === "background") {
      entry.role = "primary_fill";
    }
  }
  state.settings.palette.backgroundEntryId = entryId || null;
  pushSettings();
}

function syncControlsFromSettings() {
  const s = state.settings;
  document.getElementById("select-destination").value = s.destination.id;
  document.getElementById("input-scans").value = s.scanCount;
  document.getElementById("select-backend").value = s.backend.preferredBackendId;

  const bgSelect = document.getElementById("select-background-entry");
  const currentBg = s.palette.backgroundEntryId || "";
  bgSelect.innerHTML = '<option value="">None</option>';
  for (const entry of s.palette.entries) {
    const opt = document.createElement("option");
    opt.value = entry.id;
    opt.textContent = entry.name;
    bgSelect.appendChild(opt);
  }
  bgSelect.value = currentBg;

  const matching = s.palette.backgroundMatching || "all_matching";
  const matchingInput = document.querySelector(`input[name="bg-matching"][value="${matching}"]`);
  if (matchingInput) matchingInput.checked = true;

  const outputMode = s.output.backgroundOutput || "keep_paths";
  const outputInput = document.querySelector(`input[name="bg-output"][value="${outputMode}"]`);
  if (outputInput) outputInput.checked = true;

  document.getElementById("scan-count-badge").textContent = `${s.palette.entries.length} scans`;

  const badge = document.getElementById("source-changed-badge");
  badge.hidden = true; // cleared once the dialog is resolved; see resolveSourceChange()
}

// -------------------------------------------------------------------------
// Palette list
// -------------------------------------------------------------------------

function warningsForEntry(entryId) {
  const scan = state.scanResults.find((s) => s.entryId === entryId);
  return scan ? scan.warnings || [] : [];
}

function renderPaletteList() {
  const list = document.getElementById("palette-list");
  const entries = state.settings.palette.entries || [];
  list.innerHTML = entries.map((entry, index) => renderEntryRow(entry, index, entries.length)).join("");
}

function outputHex(entry) {
  return (entry.output && entry.output.color && entry.output.color.srgb)
    || (entry.sourceAnchor && entry.sourceAnchor.srgb)
    || "#888888";
}

function renderEntryRow(entry, index, total) {
  const claimPct = state.claimStats[entry.id] !== undefined ? state.claimStats[entry.id].toFixed(1) : "0.0";
  const outHex = outputHex(entry);
  const anchorHex = entry.sourceAnchor && entry.sourceAnchor.srgb;
  const warnings = warningsForEntry(entry.id);
  const isBackground = state.settings.palette.backgroundEntryId === entry.id;
  const kindLabel = entry.kind === "pinned" ? "Pinned" : "Automatic";

  return `
    <li class="palette-row" id="entry-row-${entry.id}" data-entry-id="${entry.id}" draggable="true">
      <div class="row-top">
        <span class="color-swatch" style="background-color:${outHex}" aria-hidden="true"></span>
        <div class="row-identity">
          <span class="row-title">${escapeHtml(entry.name)}</span>
          <span class="row-kind-badge">${kindLabel}</span>
          ${warnings.length ? `<span class="row-warning" title="${escapeHtml(warnings.join(" "))}">⚠ <span class="visually-hidden">Warning:</span> ${warnings.length}</span>` : ""}
        </div>
        <div class="row-order-controls" role="group" aria-label="Reorder ${escapeHtml(entry.name)}">
          <button type="button" class="btn btn-icon" data-action="move-up" data-id="${entry.id}"
                  aria-label="Move ${escapeHtml(entry.name)} up" ${index === 0 ? "disabled" : ""}>▲</button>
          <button type="button" class="btn btn-icon" data-action="move-down" data-id="${entry.id}"
                  aria-label="Move ${escapeHtml(entry.name)} down" ${index === total - 1 ? "disabled" : ""}>▼</button>
        </div>
      </div>

      <dl class="row-meta">
        <div><dt>Claimed</dt><dd>${claimPct}%</dd></div>
        <div><dt>Source anchor</dt><dd>${anchorHex || "—"}</dd></div>
        <div><dt>Output</dt><dd>${outHex}</dd></div>
        <div><dt>Role</dt><dd>${isBackground ? "Background" : label(ROLE_LABELS, entry.role)}</dd></div>
        <div><dt>Trace profile</dt><dd>${traceProfileSummary(entry)}</dd></div>
      </dl>

      <div class="row-visibility">
        <input type="checkbox" id="vis-${entry.id}" data-action="toggle-visibility" data-id="${entry.id}"
               ${entry.enabled !== false ? "checked" : ""}>
        <label for="vis-${entry.id}">Visible in output</label>
      </div>

      <details class="row-editor">
        <summary>Edit ${escapeHtml(entry.name)}</summary>
        <div class="row-editor-body">
          ${renderEntryEditorBody(entry, isBackground)}
        </div>
      </details>
    </li>
  `;
}

function traceProfileSummary(entry) {
  const tp = entry.traceProfile || { mode: "inherit" };
  if (tp.mode === "inherit") return "Inherit global";
  if (tp.mode === "preset") return label(PROFILE_LABELS, tp.profileId);
  if (tp.mode === "override") return "Custom";
  return tp.mode;
}

function renderEntryEditorBody(entry, isBackground) {
  const id = entry.id;
  return `
    <div class="form-group">
      <label for="name-${id}">Name</label>
      <input type="text" id="name-${id}" data-field="name" data-id="${id}" value="${escapeHtml(entry.name)}">
    </div>

    ${isBackground ? `
      <p class="field-help">Role is fixed to Background while this scan is designated as the background entry (see Background section).</p>
    ` : `
      <div class="form-group">
        <label for="role-${id}">Role</label>
        <select id="role-${id}" data-field="role" data-id="${id}">
          ${Object.keys(ROLE_LABELS).filter((r) => r !== "background").map((r) =>
            `<option value="${r}" ${entry.role === r ? "selected" : ""}>${ROLE_LABELS[r]}</option>`
          ).join("")}
        </select>
      </div>
    `}

    ${entry.kind === "pinned" ? renderReachEditor(entry) : `<p class="field-help">Automatic colour — recomputed from unclaimed pixels.</p>`}

    ${renderTraceProfileEditor(entry)}
  `;
}

function renderReachEditor(entry) {
  const assignment = entry.assignment || {};
  const channels = assignment.channels || {};
  const mode = channels.mode || "linked";
  const anchor = entry.sourceAnchor && entry.sourceAnchor.srgb;
  let chromaHint = "";
  if (anchor) {
    const { r, g, b } = hexToRgb01(anchor);
    const { C } = srgbToOklch(r, g, b);
    if (C < 0.05) {
      chromaHint = `<p class="field-hint">Hue has little effect for this neutral colour.</p>`;
    }
  }

  return `
    <fieldset class="reach-fieldset">
      <legend>Colour reach</legend>
      <div class="form-group">
        <label for="reach-${entry.id}">Overall reach</label>
        <div class="range-row">
          <input type="range" id="reach-${entry.id}" min="0" max="100"
                 value="${assignment.overallReach ?? 25}" data-field="overallReach" data-id="${entry.id}">
          <output for="reach-${entry.id}" id="reach-value-${entry.id}">${assignment.overallReach ?? 25}</output>
        </div>
      </div>

      <fieldset class="radio-fieldset">
        <legend>Reach mode</legend>
        <div class="radio-row">
          <span class="radio-option">
            <input type="radio" name="channels-mode-${entry.id}" id="channels-linked-${entry.id}"
                   data-field="channelsMode" data-id="${entry.id}" value="linked" ${mode === "linked" ? "checked" : ""}>
            <label for="channels-linked-${entry.id}">Linked</label>
          </span>
          <span class="radio-option">
            <input type="radio" name="channels-mode-${entry.id}" id="channels-custom-${entry.id}"
                   data-field="channelsMode" data-id="${entry.id}" value="custom" ${mode === "custom" ? "checked" : ""}>
            <label for="channels-custom-${entry.id}">Custom</label>
          </span>
        </div>
      </fieldset>

      ${chromaHint}

      ${mode === "custom" ? renderChannelControls(entry, channels) : ""}
    </fieldset>
  `;
}

function renderChannelControls(entry, channels) {
  const specs = [
    { key: "hue", label: "Hue", min: 0, max: 180, step: 1, unit: "°" },
    { key: "chroma", label: "Saturation / chroma", min: 0, max: 0.4, step: 0.005, unit: "" },
    { key: "lightness", label: "Lightness", min: 0, max: 1, step: 0.01, unit: "" },
  ];
  return specs.map((spec) => {
    const cfg = channels[spec.key] || { enabled: true, tolerance: 0, weight: 1 };
    const idBase = `${spec.key}-${entry.id}`;
    return `
      <fieldset class="channel-fieldset">
        <legend>${spec.label}</legend>
        <div class="channel-row">
          <input type="checkbox" id="${idBase}-enabled" data-field="channelEnabled" data-channel="${spec.key}"
                 data-id="${entry.id}" ${cfg.enabled !== false ? "checked" : ""}>
          <label for="${idBase}-enabled">Participates in matching</label>
        </div>
        <div class="form-group">
          <label for="${idBase}-tolerance">Tolerance${spec.unit ? ` (${spec.unit})` : ""}</label>
          <div class="range-row">
            <input type="range" id="${idBase}-tolerance" min="${spec.min}" max="${spec.max}" step="${spec.step}"
                   value="${cfg.tolerance ?? spec.min}" data-field="channelTolerance" data-channel="${spec.key}" data-id="${entry.id}">
            <output for="${idBase}-tolerance" id="${idBase}-tolerance-value">${cfg.tolerance ?? spec.min}</output>
          </div>
        </div>
        <div class="form-group">
          <label for="${idBase}-weight">Weight</label>
          <input type="number" id="${idBase}-weight" min="0" step="0.1" value="${cfg.weight ?? 1}"
                 data-field="channelWeight" data-channel="${spec.key}" data-id="${entry.id}">
        </div>
      </fieldset>
    `;
  }).join("");
}

const PROFILE_FIELD_SPECS = {
  mask: [
    { key: "minimumRegionAreaPx2", label: "Minimum region area (px²)", min: 0, step: 1 },
    { key: "fillHolesAreaPx2", label: "Fill holes up to area (px²)", min: 0, step: 1 },
    { key: "closeGapsRadiusPx", label: "Close-gaps radius (px)", min: 0, step: 0.5 },
    { key: "smoothingRadiusPx", label: "Smoothing radius (px)", min: 0, step: 0.1 },
    { key: "minimumFeatureWidthPx", label: "Minimum protected feature width (px)", min: 0, step: 1 },
    { key: "offsetPx", label: "Offset (px, negative contracts)", min: -50, step: 0.5 },
  ],
  vector: [
    { key: "cornerSensitivity", label: "Corner sensitivity", min: 0, max: 1, step: 0.05 },
    { key: "curveSmoothing", label: "Curve smoothing", min: 0, max: 1, step: 0.05 },
    { key: "optimization", label: "Optimization", min: 0, max: 1, step: 0.05 },
    { key: "minimumPathAreaPx2", label: "Minimum path area (px²)", min: 0, step: 1 },
  ],
};

function resolvedBaseProfile(profileId) {
  if (profileId && state.traceProfiles.profiles[profileId]) {
    return state.traceProfiles.profiles[profileId];
  }
  return state.settings.globalTraceProfile;
}

function renderTraceProfileEditor(entry) {
  const tp = entry.traceProfile || { mode: "inherit" };
  const idBase = `profile-${entry.id}`;

  const presetOptions = state.traceProfiles.order.map((id) =>
    `<option value="${id}" ${tp.profileId === id ? "selected" : ""}>${label(PROFILE_LABELS, id)}</option>`
  ).join("");

  let body = `
    <fieldset class="profile-fieldset">
      <legend>Trace profile</legend>
      <div class="form-group">
        <label for="${idBase}-mode">Mode</label>
        <select id="${idBase}-mode" data-field="profileMode" data-id="${entry.id}">
          <option value="inherit" ${tp.mode === "inherit" ? "selected" : ""}>Inherit global profile</option>
          <option value="preset" ${tp.mode === "preset" ? "selected" : ""}>Use a named preset</option>
          <option value="override" ${tp.mode === "override" ? "selected" : ""}>Customize</option>
        </select>
      </div>
  `;

  if (tp.mode === "preset") {
    body += `
      <div class="form-group">
        <label for="${idBase}-preset">Preset</label>
        <select id="${idBase}-preset" data-field="profilePresetId" data-id="${entry.id}">${presetOptions}</select>
      </div>
    `;
  }

  if (tp.mode === "override") {
    const base = resolvedBaseProfile(tp.profileId);
    const values = tp.values || base;
    body += `
      <div class="form-group">
        <label for="${idBase}-base">Base on</label>
        <select id="${idBase}-base" data-field="profileBaseId" data-id="${entry.id}">
          <option value="" ${!tp.profileId ? "selected" : ""}>Global profile</option>
          ${presetOptions}
        </select>
      </div>
      <fieldset class="profile-section">
        <legend>Mask cleanup</legend>
        ${PROFILE_FIELD_SPECS.mask.map((f) => renderProfileField(entry.id, "mask", f, values.mask || base.mask)).join("")}
        <div class="channel-row">
          <input type="checkbox" id="${idBase}-preserveThin" data-field="profileValue" data-section="mask"
                 data-key="preserveThinFeatures" data-id="${entry.id}"
                 ${(values.mask || base.mask).preserveThinFeatures !== false ? "checked" : ""}>
          <label for="${idBase}-preserveThin">Preserve thin features</label>
        </div>
      </fieldset>
      <fieldset class="profile-section">
        <legend>Vector tracing</legend>
        ${PROFILE_FIELD_SPECS.vector.map((f) => renderProfileField(entry.id, "vector", f, values.vector || base.vector)).join("")}
        <div class="channel-row">
          <input type="checkbox" id="${idBase}-requireClosed" data-field="profileValue" data-section="vector"
                 data-key="requireClosedPaths" data-id="${entry.id}"
                 ${(values.vector || base.vector).requireClosedPaths ? "checked" : ""}>
          <label for="${idBase}-requireClosed">Require closed paths</label>
        </div>
      </fieldset>
    `;
  }

  body += `</fieldset>`;
  return body;
}

function renderProfileField(entryId, section, spec, values) {
  const id = `profile-${entryId}-${section}-${spec.key}`;
  const value = values ? values[spec.key] : spec.min;
  const minAttr = spec.min !== undefined ? ` min="${spec.min}"` : "";
  const maxAttr = spec.max !== undefined ? ` max="${spec.max}"` : "";
  return `
    <div class="form-group">
      <label for="${id}">${spec.label}</label>
      <input type="number" id="${id}" data-field="profileValue" data-section="${section}" data-key="${spec.key}"
             data-id="${entryId}"${minAttr}${maxAttr} step="${spec.step}" value="${value}">
    </div>
  `;
}

function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str == null ? "" : String(str);
  return div.innerHTML;
}

// -------------------------------------------------------------------------
// Palette list event delegation
// -------------------------------------------------------------------------

function findEntry(id) {
  return state.settings.palette.entries.find((e) => e.id === id);
}

function onPaletteListClick(e) {
  const target = e.target.closest("[data-action]");
  if (!target) return;
  const { action, id } = target.dataset;

  if (action === "move-up" || action === "move-down") {
    reorderEntry(id, action === "move-up" ? -1 : 1);
  }
}

function onPaletteListChange(e) {
  const target = e.target;
  const field = target.dataset.field;
  if (!field) return;
  const id = target.dataset.id;
  const entry = findEntry(id);

  switch (field) {
    case "toggle-visibility":
      break; // handled via data-action below
    case "name":
      entry.name = target.value || entry.name;
      pushSettings();
      break;
    case "role":
      entry.role = target.value;
      pushSettings();
      break;
    case "channelsMode": {
      entry.assignment.channels = entry.assignment.channels || {};
      entry.assignment.channels.mode = target.value;
      if (target.value === "custom" && !entry.assignment.channels.hue) {
        const anchor = entry.sourceAnchor && entry.sourceAnchor.srgb;
        let hueEnabled = true;
        if (anchor) {
          const { r, g, b } = hexToRgb01(anchor);
          hueEnabled = srgbToOklch(r, g, b).C >= 0.02;
        }
        entry.assignment.channels.hue = { enabled: hueEnabled, tolerance: 10.0, weight: 1.0 };
        entry.assignment.channels.chroma = { enabled: true, tolerance: 0.035, weight: 1.0 };
        entry.assignment.channels.lightness = { enabled: true, tolerance: 0.06, weight: 1.0 };
      }
      pushSettings();
      break;
    }
    case "channelEnabled": {
      const ch = target.dataset.channel;
      entry.assignment.channels[ch].enabled = target.checked;
      pushSettings();
      break;
    }
    case "channelWeight": {
      const ch = target.dataset.channel;
      entry.assignment.channels[ch].weight = parseFloat(target.value) || 0;
      pushSettings();
      break;
    }
    case "profileMode": {
      if (target.value === "inherit") {
        entry.traceProfile = { mode: "inherit" };
      } else if (target.value === "preset") {
        entry.traceProfile = { mode: "preset", profileId: "default" };
      } else {
        entry.traceProfile = {
          mode: "override",
          values: JSON.parse(JSON.stringify(state.settings.globalTraceProfile)),
        };
      }
      pushSettings();
      break;
    }
    case "profilePresetId":
      entry.traceProfile.profileId = target.value;
      pushSettings();
      break;
    case "profileBaseId": {
      const base = target.value ? state.traceProfiles.profiles[target.value] : state.settings.globalTraceProfile;
      entry.traceProfile.profileId = target.value || undefined;
      entry.traceProfile.values = JSON.parse(JSON.stringify(base));
      pushSettings();
      break;
    }
    case "profileValue": {
      const section = target.dataset.section, key = target.dataset.key;
      entry.traceProfile.values = entry.traceProfile.values || {};
      entry.traceProfile.values[section] = entry.traceProfile.values[section] || {};
      entry.traceProfile.values[section][key] = target.type === "checkbox" ? target.checked : parseFloat(target.value);
      pushSettings();
      break;
    }
  }

  if (target.dataset.action === "toggle-visibility") {
    entry.enabled = target.checked;
    pushSettings();
  }
}

function onPaletteListInput(e) {
  const target = e.target;
  const field = target.dataset.field;
  if (field === "overallReach") {
    document.getElementById(`reach-value-${target.dataset.id}`).textContent = target.value;
  } else if (field === "channelTolerance") {
    const ch = target.dataset.channel, id = target.dataset.id;
    document.getElementById(`${ch}-${id}-tolerance-value`).textContent = target.value;
  }
}

// Commit range-slider drags on release, not on every intermediate 'input'
// event — an /api round trip per pixel of drag would both hammer the server
// and continuously steal focus.
document.addEventListener("change", (e) => {
  if (e.target.dataset && e.target.dataset.field === "overallReach") {
    const entry = findEntry(e.target.dataset.id);
    entry.assignment.overallReach = parseInt(e.target.value, 10);
    pushSettings();
  } else if (e.target.dataset && e.target.dataset.field === "channelTolerance") {
    const entry = findEntry(e.target.dataset.id);
    const ch = e.target.dataset.channel;
    entry.assignment.channels[ch].tolerance = parseFloat(e.target.value);
    pushSettings();
  }
});

function reorderEntry(id, direction) {
  const order = state.settings.palette.layerOrder;
  const idx = order.indexOf(id);
  const swapWith = idx + direction;
  if (idx < 0 || swapWith < 0 || swapWith >= order.length) return;
  [order[idx], order[swapWith]] = [order[swapWith], order[idx]];

  // Keep the visual list (driven by palette.entries) in the same order as
  // layerOrder so the row a keyboard user just moved stays under focus.
  const entries = state.settings.palette.entries;
  const entryIdx = entries.findIndex((e) => e.id === id);
  const entrySwap = entryIdx + direction;
  if (entryIdx >= 0 && entrySwap >= 0 && entrySwap < entries.length) {
    [entries[entryIdx], entries[entrySwap]] = [entries[entrySwap], entries[entryIdx]];
  }

  pushSettings();
}

// Drag-and-drop is a pointer-only enhancement; the Move up/down buttons
// above are the keyboard- and screen-reader-accessible mechanism (§9.2).
let dragSourceId = null;
document.getElementById("palette-list") || null; // list may not exist yet at parse time
document.addEventListener("dragstart", (e) => {
  const row = e.target.closest(".palette-row");
  if (!row) return;
  dragSourceId = row.dataset.entryId;
  e.dataTransfer.effectAllowed = "move";
});
document.addEventListener("dragover", (e) => {
  if (e.target.closest(".palette-row")) e.preventDefault();
});
document.addEventListener("drop", (e) => {
  const row = e.target.closest(".palette-row");
  if (!row || !dragSourceId || row.dataset.entryId === dragSourceId) return;
  e.preventDefault();
  moveEntryBefore(dragSourceId, row.dataset.entryId);
  dragSourceId = null;
});

function moveEntryBefore(sourceId, targetId) {
  const order = state.settings.palette.layerOrder;
  const entries = state.settings.palette.entries;
  for (const arr of [order, entries]) {
    const key = arr === order ? (x) => x : (x) => x.id;
    const items = arr;
    const sourceIdx = items.findIndex((x) => key(x) === sourceId);
    const targetIdx = items.findIndex((x) => key(x) === targetId);
    if (sourceIdx < 0 || targetIdx < 0) continue;
    const [moved] = items.splice(sourceIdx, 1);
    items.splice(items.indexOf(items.find((x) => key(x) === targetId)), 0, moved);
  }
  pushSettings();
}

// -------------------------------------------------------------------------
// Warnings
// -------------------------------------------------------------------------

function renderWarnings() {
  const el = document.getElementById("warnings-summary");
  if (!state.warnings.length) {
    el.hidden = true;
    el.textContent = "";
    return;
  }
  el.hidden = false;
  el.textContent = `${state.warnings.length} warning(s): ${state.warnings.map((w) => w.message || w).join(" · ")}`;
}

// -------------------------------------------------------------------------
// Preview canvas
// -------------------------------------------------------------------------

const PREVIEW_MODE_DESCRIPTIONS = {
  source: "Shows the original source image unmodified.",
  quantized: "Shows every scan's traced shape filled with its output colour.",
  claim: "Shows each scan's claimed region, filled with its colour and outlined so overlapping claims stay distinguishable without relying on colour alone.",
  vector: "Shows the traced vector paths that will be written on Apply.",
};

function renderPreviewModeDescription() {
  document.getElementById("preview-mode-description").textContent =
    PREVIEW_MODE_DESCRIPTIONS[state.previewMode] || "";
}

function zoomBy(factor) {
  const canvas = document.getElementById("preview-canvas");
  const current = parseFloat(canvas.dataset.zoom || "1");
  setZoom(Math.max(0.1, Math.min(8, current * factor)));
}

function setZoom(value) {
  const canvas = document.getElementById("preview-canvas");
  if (value === null) {
    canvas.style.width = "";
    canvas.style.height = "";
    canvas.dataset.zoom = "1";
    document.getElementById("zoom-level").textContent = "Fit";
    return;
  }
  canvas.dataset.zoom = String(value);
  canvas.style.width = `${canvas.width * value}px`;
  canvas.style.height = `${canvas.height * value}px`;
  document.getElementById("zoom-level").textContent = `${Math.round(value * 100)}%`;
}

function contrastingStroke(hex) {
  const { r, g, b } = hexToRgb01(hex || "#888888");
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return luminance > 0.5 ? "rgba(0,0,0,0.65)" : "rgba(255,255,255,0.75)";
}

function renderCanvas() {
  const canvas = document.getElementById("preview-canvas");
  const ctx = canvas.getContext("2d");
  if (!state.sourceImage.width) return;

  canvas.width = state.sourceImage.width;
  canvas.height = state.sourceImage.height;
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  if (state.previewMode === "source") {
    ctx.drawImage(state.sourceImage, 0, 0);
    return;
  }

  if (state.previewMode === "quantized" || state.previewMode === "vector") {
    for (const scan of state.scanResults) {
      ctx.fillStyle = scan.color;
      for (const d of scan.pathDatas) {
        const p = new Path2D(d);
        ctx.fill(p, scan.fillRule || "evenodd");
      }
    }
    return;
  }

  if (state.previewMode === "claim") {
    ctx.drawImage(state.sourceImage, 0, 0);
    for (const scan of state.scanResults) {
      ctx.fillStyle = scan.color;
      ctx.globalAlpha = 0.55;
      ctx.strokeStyle = contrastingStroke(scan.color);
      ctx.lineWidth = 1.5;
      ctx.setLineDash([4, 3]);
      for (const d of scan.pathDatas) {
        const p = new Path2D(d);
        ctx.fill(p, scan.fillRule || "evenodd");
        ctx.stroke(p);
      }
    }
    ctx.globalAlpha = 1;
    ctx.setLineDash([]);
  }
}

async function onCanvasClick(e) {
  const canvas = document.getElementById("preview-canvas");
  const rect = canvas.getBoundingClientRect();
  const scaleX = canvas.width / rect.width;
  const scaleY = canvas.height / rect.height;
  const x = Math.floor((e.clientX - rect.left) * scaleX);
  const y = Math.floor((e.clientY - rect.top) * scaleY);

  const sample = await api("/api/sample_color", { method: "POST", body: { x, y, mode: "5x5_median" } });
  if (sample.hex) {
    pinColor(sample.hex, sample.r, sample.g, sample.b);
  }
}

async function pinColor(hex, r, g, b) {
  const entries = state.settings.palette.entries;
  const target = entries.find((entry) => entry.kind === "automatic") || entries[0];
  if (!target) return;

  const { C } = srgbToOklch(r, g, b);
  const hueEnabled = C >= 0.02; // §13.4: neutral colours default hue matching off

  target.kind = "pinned";
  target.sourceAnchor = { srgb: hex };
  target.output = { mode: "exact", color: { srgb: hex } };
  target.assignment = {
    mode: "reserve_within_reach",
    overallReach: 25,
    channels: {
      mode: "linked",
      hue: { enabled: hueEnabled, tolerance: 10.0, weight: 1.0 },
      chroma: { enabled: true, tolerance: 0.035, weight: 1.0 },
      lightness: { enabled: true, tolerance: 0.06, weight: 1.0 },
    },
  };
  await pushSettings();
  announceStatus(`Pinned ${hex} to ${target.name}.`);
}

// -------------------------------------------------------------------------
// Dialogs — native <dialog> gives focus trapping and Escape-to-close for
// free; we only need to remember and restore the triggering element (§29).
// -------------------------------------------------------------------------

function openDialog(dialog, trigger) {
  state.lastFocusedBeforeDialog = trigger || document.activeElement;
  dialog.showModal();
}

function closeDialog(dialog) {
  dialog.close();
  if (state.lastFocusedBeforeDialog) {
    state.lastFocusedBeforeDialog.focus();
    state.lastFocusedBeforeDialog = null;
  }
}

function bindDialogs() {
  for (const dialog of document.querySelectorAll(".app-dialog")) {
    dialog.addEventListener("cancel", () => closeDialog(dialog)); // Escape key
    for (const btn of dialog.querySelectorAll("[data-dialog-cancel]")) {
      btn.addEventListener("click", () => closeDialog(dialog));
    }
  }

  // §27 source-changed recovery choices
  const sourceDialog = document.getElementById("dialog-source-changed");
  for (const btn of sourceDialog.querySelectorAll(".dialog-choice")) {
    btn.addEventListener("click", () => resolveSourceChange(btn.dataset.action));
  }

  document.getElementById("form-save-preset").addEventListener("submit", async (e) => {
    e.preventDefault();
    const name = document.getElementById("preset-name").value.trim();
    const description = document.getElementById("preset-description").value.trim();
    const scope = document.querySelector('input[name="preset-scope"]:checked').value;
    if (!name) return;
    await api("/api/user_presets", { method: "POST", body: { name, description, scope } });
    announceStatus(`Saved preset “${name}”.`);
    document.getElementById("preset-name").value = "";
    document.getElementById("preset-description").value = "";
    closeDialog(document.getElementById("dialog-save-preset"));
  });
}

function openSourceChangedDialog() {
  document.getElementById("source-changed-badge").hidden = false;
  openDialog(document.getElementById("dialog-source-changed"), document.getElementById("btn-apply"));
}

async function resolveSourceChange(action) {
  const dialog = document.getElementById("dialog-source-changed");
  if (action === "cancel") {
    await api("/api/resolve_source_change", { method: "POST", body: { action: "cancel" } });
    window.close();
    return;
  }
  const data = await api("/api/resolve_source_change", { method: "POST", body: { action } });
  applyServerResponse(data);
  document.getElementById("source-changed-badge").hidden = true;
  closeDialog(dialog);
  announceStatus("Source-change resolved.");
}

function openSavePresetDialog() {
  openDialog(document.getElementById("dialog-save-preset"), document.getElementById("btn-save-preset"));
  document.getElementById("preset-name").focus();
}

async function refreshUserPresets() {
  const data = await api("/api/user_presets");
  state.userPresets = data.presets || [];
}

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
      <div class="preset-row-info">
        <span class="row-title">${escapeHtml(preset.name)}</span>
        <span class="badge">${escapeHtml(preset.scope)}</span>
        ${preset.description ? `<p class="field-help">${escapeHtml(preset.description)}</p>` : ""}
      </div>
      <div class="preset-row-actions">
        <button type="button" class="btn btn-secondary" data-preset-action="apply" data-preset-id="${preset.presetUuid}">Apply</button>
        <button type="button" class="btn btn-secondary" data-preset-action="delete" data-preset-id="${preset.presetUuid}"
                aria-label="Delete preset ${escapeHtml(preset.name)}">Delete</button>
      </div>
    </li>
  `).join("");

  list.querySelectorAll("[data-preset-action]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const presetId = btn.dataset.presetId;
      if (btn.dataset.presetAction === "apply") {
        const data = await api("/api/user_presets/apply", { method: "POST", body: { presetUuid: presetId } });
        applyServerResponse(data);
        announceStatus("Preset applied.");
        closeDialog(document.getElementById("dialog-load-preset"));
      } else {
        await api("/api/user_presets/delete", { method: "POST", body: { presetUuid: presetId } });
        await refreshUserPresets();
        renderPresetList();
        announceStatus("Preset deleted.");
      }
    });
  });
}

async function openLoadPresetDialog() {
  await refreshUserPresets();
  renderPresetList();
  openDialog(document.getElementById("dialog-load-preset"), document.getElementById("btn-load-preset"));
}
