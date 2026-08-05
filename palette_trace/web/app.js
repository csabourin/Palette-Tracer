// Palette Trace Frontend Client App
const urlParams = new URLSearchParams(window.location.search);
const sessionToken = urlParams.get("token");

let currentSettings = null;
let currentPreviewMode = "source";
let sourceImage = new Image();
let scanResults = [];
let claimStats = {};

document.addEventListener("DOMContentLoaded", () => {
  if (!sessionToken) {
    alert("Error: Missing session token.");
    return;
  }

  initEventListeners();
  loadSessionData();
});

function getHeaders() {
  return {
    "Content-Type": "application/json",
    "X-Session-Token": sessionToken,
  };
}

async function loadSessionData() {
  try {
    const res = await fetch("/api/session", { headers: getHeaders() });
    const data = await res.json();
    currentSettings = data.settings;

    // Load source preview image
    const imgRes = await fetch("/api/preview_source", { headers: getHeaders() });
    const imgData = await imgRes.json();

    sourceImage.src = imgData.dataUri;
    sourceImage.onload = () => {
      renderCanvas();
      updateSettingsAndReevaluate();
    };
  } catch (err) {
    console.error("Failed to load session data:", err);
  }
}

function initEventListeners() {
  document.getElementById("select-destination").addEventListener("change", (e) => {
    currentSettings.destination.id = e.target.value;
    updateSettingsAndReevaluate();
  });

  document.getElementById("input-scans").addEventListener("change", (e) => {
    currentSettings.scanCount = parseInt(e.target.value, 10);
    updateSettingsAndReevaluate();
  });

  document.querySelectorAll(".mode-btn").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      document.querySelectorAll(".mode-btn").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      currentPreviewMode = btn.dataset.mode;
      renderCanvas();
    });
  });

  document.getElementById("btn-apply").addEventListener("click", async () => {
    await fetch("/api/apply", { method: "POST", headers: getHeaders() });
    window.close();
  });

  document.getElementById("btn-cancel").addEventListener("click", async () => {
    await fetch("/api/cancel", { method: "POST", headers: getHeaders() });
    window.close();
  });

  // Canvas click color picker
  const canvas = document.getElementById("preview-canvas");
  canvas.addEventListener("click", async (e) => {
    const rect = canvas.getBoundingClientRect();
    const scaleX = canvas.width / rect.width;
    const scaleY = canvas.height / rect.height;

    const x = Math.floor((e.clientX - rect.left) * scaleX);
    const y = Math.floor((e.clientY - rect.top) * scaleY);

    const sampleRes = await fetch("/api/sample_color", {
      method: "POST",
      headers: getHeaders(),
      body: JSON.stringify({ x, y, mode: "5x5_median" }),
    });

    const sample = await sampleRes.json();
    if (sample.hex) {
      pinNewColor(sample.hex);
    }
  });
}

async function updateSettingsAndReevaluate() {
  const res = await fetch("/api/update_settings", {
    method: "POST",
    headers: getHeaders(),
    body: JSON.stringify({ settings: currentSettings }),
  });

  const data = await res.json();
  currentSettings.palette.entries = data.paletteEntries;
  claimStats = data.claimStats || {};
  scanResults = data.scanResults || [];

  renderPaletteUI();
  renderCanvas();
}

function pinNewColor(hexColor) {
  const entries = currentSettings.palette.entries;
  // Replace first automatic entry or update active entry
  let target = entries.find((e) => e.kind === "automatic");
  if (!target) target = entries[0];

  if (target) {
    target.kind = "pinned";
    target.sourceAnchor = { srgb: hexColor };
    target.output = { mode: "exact", color: { srgb: hexColor } };
    target.assignment = {
      mode: "reserve_within_reach",
      overallReach: 25,
      channels: { mode: "linked" },
    };
    updateSettingsAndReevaluate();
  }
}

function renderPaletteUI() {
  const list = document.getElementById("palette-list");
  list.innerHTML = "";

  const entries = currentSettings.palette.entries || [];
  document.getElementById("scan-count-badge").textContent = `${entries.length} scans`;

  entries.forEach((entry, idx) => {
    const row = document.createElement("div");
    row.className = "palette-row";

    const outHex = entry.output?.color?.srgb || entry.sourceAnchor?.srgb || "#888888";
    const claimPct = claimStats[entry.id] ? claimStats[entry.id].toFixed(1) : "0.0";

    row.innerHTML = `
      <div class="row-top">
        <div class="swatch-group">
          <div class="color-swatch" style="background-color: ${outHex}"></div>
          <span class="row-title">${entry.name} (${entry.kind})</span>
        </div>
        <span class="badge">${claimPct}% claimed</span>
      </div>
      ${
        entry.kind === "pinned"
          ? `
        <div class="reach-slider">
          <label style="font-size:0.75rem">Colour Reach:</label>
          <input type="range" min="0" max="100" value="${entry.assignment?.overallReach || 25}" data-id="${entry.id}">
          <span style="font-size:0.75rem">${entry.assignment?.overallReach || 25}</span>
        </div>
      `
          : ""
      }
    `;

    const slider = row.querySelector('input[type="range"]');
    if (slider) {
      slider.addEventListener("input", (e) => {
        const val = parseInt(e.target.value, 10);
        entry.assignment.overallReach = val;
        row.querySelector(".reach-slider span").textContent = val;
        updateSettingsAndReevaluate();
      });
    }

    list.appendChild(row);
  });
}

function renderCanvas() {
  const canvas = document.getElementById("preview-canvas");
  const ctx = canvas.getContext("2d");

  if (!sourceImage.width) return;

  canvas.width = sourceImage.width;
  canvas.height = sourceImage.height;

  ctx.clearRect(0, 0, canvas.width, canvas.height);

  if (currentPreviewMode === "source") {
    ctx.drawImage(sourceImage, 0, 0);
  } else if (currentPreviewMode === "vector" || currentPreviewMode === "quantized") {
    // Draw vector path outputs
    scanResults.forEach((scan) => {
      ctx.fillStyle = scan.color;
      scan.pathDatas.forEach((d) => {
        const p = new Path2D(d);
        ctx.fill(p, scan.fillRule || "evenodd");
      });
    });
  } else if (currentPreviewMode === "claim") {
    ctx.drawImage(sourceImage, 0, 0);
    // Semi-transparent overlay for claims
    ctx.fillStyle = "rgba(59, 130, 246, 0.4)";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
  }
}
