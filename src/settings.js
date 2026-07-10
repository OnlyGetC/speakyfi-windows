/**
 * Speakyfi Windows — Settings JS
 * Handles navigation, config load/save, key capture, model downloads.
 */

const { invoke } = window.__TAURI__.core;
const { listen, emit } = window.__TAURI__.event;

// ============================================================
// Navigation
// ============================================================
document.querySelectorAll(".nav-item").forEach(item => {
  item.addEventListener("click", () => {
    document.querySelectorAll(".nav-item").forEach(i => i.classList.remove("active"));
    document.querySelectorAll(".settings-section").forEach(s => s.classList.remove("active"));
    item.classList.add("active");
    const section = item.dataset.section;
    const sec = document.getElementById("sec-" + section);
    if (sec) sec.classList.add("active");
  });
});

// ============================================================
// Config state
// ============================================================
let config = null;

async function loadConfig() {
  config = await invoke("load_config");
  applyConfigToUI(config);
}

function applyConfigToUI(cfg) {
  // Hotkeys
  setKeyLabel("ptt-key-capture", "ptt-key-label", cfg.ptt_key, cfg.ptt_modifiers);
  setKeyLabel("vad-key-capture", "vad-key-label", cfg.vad_toggle_key, cfg.vad_toggle_modifiers);
  document.getElementById("ptt-key-capture").dataset.key = cfg.ptt_key;
  document.getElementById("ptt-key-capture").dataset.mods = cfg.ptt_modifiers;
  document.getElementById("vad-key-capture").dataset.key = cfg.vad_toggle_key;
  document.getElementById("vad-key-capture").dataset.mods = cfg.vad_toggle_modifiers;

  // Model
  const modelSel = document.getElementById("model-select");
  if (modelSel) modelSel.value = cfg.model || "base";

  // Language
  const langSel = document.getElementById("lang-select");
  if (langSel) langSel.value = cfg.language || "auto";

  // Prompt
  const promptIn = document.getElementById("prompt-input");
  if (promptIn) promptIn.value = cfg.prompt || "";

  // Correction
  const corrMode = cfg.correction_mode || "off";
  const corrRadio = document.querySelector(`input[name="correction-mode"][value="${corrMode}"]`);
  if (corrRadio) corrRadio.checked = true;
  updateCorrectionVisibility(corrMode);

  const ollamaEp = document.getElementById("ollama-endpoint");
  if (ollamaEp) ollamaEp.value = cfg.correction_endpoint || "http://localhost:11434";
  const ollamaModel = document.getElementById("ollama-model");
  if (ollamaModel) ollamaModel.value = cfg.correction_model || "llama3.2:1b";

  // Provider
  const provSel = document.getElementById("provider-select");
  if (provSel) {
    provSel.value = cfg.cloud_provider || "local";
    updateProviderKeySection(cfg.cloud_provider);
  }

  // Interface lang
  const uiLang = document.getElementById("ui-lang-select");
  if (uiLang) uiLang.value = cfg.interface_lang || "en";

  // Version
  const ver = document.getElementById("current-version");
  if (ver) ver.textContent = "v" + (cfg.version || "1.6.0");
}

function collectConfigFromUI() {
  const pttCapture = document.getElementById("ptt-key-capture");
  const vadCapture = document.getElementById("vad-key-capture");
  const corrMode = document.querySelector('input[name="correction-mode"]:checked')?.value || "off";

  return {
    ptt_key: parseInt(pttCapture.dataset.key) || 0x11,
    ptt_modifiers: parseInt(pttCapture.dataset.mods) || 0,
    vad_toggle_key: parseInt(vadCapture.dataset.key) || 0,
    vad_toggle_modifiers: parseInt(vadCapture.dataset.mods) || 0,
    model: document.getElementById("model-select")?.value || "base",
    language: document.getElementById("lang-select")?.value || "auto",
    prompt: document.getElementById("prompt-input")?.value || "",
    correction_mode: corrMode,
    correction_endpoint: corrMode === "ollama"
      ? (document.getElementById("ollama-endpoint")?.value || "http://localhost:11434")
      : (document.getElementById("api-endpoint")?.value || ""),
    correction_model: corrMode === "ollama"
      ? (document.getElementById("ollama-model")?.value || "llama3.2:1b")
      : (document.getElementById("api-model")?.value || ""),
    interface_lang: document.getElementById("ui-lang-select")?.value || "en",
    cloud_provider: document.getElementById("provider-select")?.value || "local",
    version: config?.version || "1.6.0",
  };
}

// ============================================================
// Save / Close
// ============================================================
document.getElementById("btn-save").addEventListener("click", async () => {
  try {
    const cfg = collectConfigFromUI();
    await invoke("save_config", { config: cfg });
    config = cfg;
    await emit("config-updated", cfg);
    const btn = document.getElementById("btn-save");
    btn.textContent = "[ SAVED ]";
    setTimeout(() => { btn.textContent = "[ SAVE ]"; }, 1500);
  } catch (err) {
    alert("Save failed: " + err);
  }
});

document.getElementById("btn-close").addEventListener("click", async () => {
  const { WebviewWindow } = window.__TAURI__.webviewWindow;
  const win = await WebviewWindow.getByLabel("settings");
  if (win) await win.hide();
});

// ============================================================
// Key capture
// ============================================================
let capturingElement = null;

function setKeyLabel(btnId, labelId, key, mods) {
  const label = document.getElementById(labelId);
  if (!label) return;
  if (!key || key === 0) {
    label.textContent = "Not set";
    return;
  }
  const parts = [];
  if (mods & 0x0002) parts.push("Ctrl");
  if (mods & 0x0001) parts.push("Alt");
  if (mods & 0x0004) parts.push("Shift");
  if (mods & 0x0008) parts.push("Win");
  parts.push(vkCodeToName(key));
  label.textContent = parts.join("+");
}

function vkCodeToName(vk) {
  const VK_NAMES = {
    0x08: "Backspace", 0x09: "Tab", 0x0D: "Enter", 0x10: "Shift",
    0x11: "Ctrl", 0x12: "Alt", 0x13: "Pause", 0x14: "CapsLock",
    0x1B: "Esc", 0x20: "Space", 0x21: "PgUp", 0x22: "PgDn",
    0x23: "End", 0x24: "Home", 0x25: "Left", 0x26: "Up",
    0x27: "Right", 0x28: "Down", 0x2C: "PrtSc", 0x2D: "Ins",
    0x2E: "Del", 0x5B: "LWin", 0x5C: "RWin",
    0x70: "F1", 0x71: "F2", 0x72: "F3", 0x73: "F4",
    0x74: "F5", 0x75: "F6", 0x76: "F7", 0x77: "F8",
    0x78: "F9", 0x79: "F10", 0x7A: "F11", 0x7B: "F12",
    0x90: "NumLock", 0x91: "ScrollLock",
    0xA0: "LShift", 0xA1: "RShift", 0xA2: "LCtrl", 0xA3: "RCtrl",
    0xA4: "LAlt", 0xA5: "RAlt",
  };
  // A-Z
  if (vk >= 0x41 && vk <= 0x5A) return String.fromCharCode(vk);
  // 0-9
  if (vk >= 0x30 && vk <= 0x39) return String.fromCharCode(vk);
  return VK_NAMES[vk] || "0x" + vk.toString(16).toUpperCase();
}

document.querySelectorAll(".key-capture").forEach(btn => {
  btn.addEventListener("click", () => {
    if (capturingElement === btn) {
      // Cancel
      btn.classList.remove("capturing");
      capturingElement = null;
      return;
    }
    if (capturingElement) {
      capturingElement.classList.remove("capturing");
    }
    capturingElement = btn;
    btn.classList.add("capturing");
    const labelId = btn.id.replace("capture", "label");
    const label = document.getElementById(labelId);
    if (label) label.textContent = "Press key...";
  });
});

document.addEventListener("keydown", (e) => {
  if (!capturingElement) return;
  e.preventDefault();
  e.stopPropagation();

  const key = e.keyCode;
  let mods = 0;
  if (e.ctrlKey)  mods |= 0x0002;
  if (e.altKey)   mods |= 0x0001;
  if (e.shiftKey) mods |= 0x0004;

  // Allow pure modifier presses (Ctrl alone, Alt alone, Shift alone, Win alone)
  // VK: Shift=0x10, Ctrl=0x11, Alt=0x12, LWin=0x5B, RWin=0x5C
  const modifierVKs = [0x10, 0x11, 0x12, 0x5B, 0x5C];
  if (modifierVKs.includes(key)) {
    // Capture the modifier key itself — mods will be 0, key is the VK code
    capturingElement.dataset.key = key;
    capturingElement.dataset.mods = 0;
  } else {
    capturingElement.dataset.key = key;
    capturingElement.dataset.mods = mods;
  }

  const labelId = capturingElement.id.replace("capture", "label");
  setKeyLabel(capturingElement.id, labelId,
    parseInt(capturingElement.dataset.key),
    parseInt(capturingElement.dataset.mods));

  capturingElement.classList.remove("capturing");
  capturingElement = null;
}, true);

// ============================================================
// Correction mode toggle
// ============================================================
function updateCorrectionVisibility(mode) {
  document.getElementById("correction-config-ollama").style.display =
    mode === "ollama" ? "block" : "none";
  document.getElementById("correction-config-api").style.display =
    mode === "api" ? "block" : "none";
}

document.querySelectorAll('input[name="correction-mode"]').forEach(radio => {
  radio.addEventListener("change", () => updateCorrectionVisibility(radio.value));
});

// ============================================================
// Provider API key
// ============================================================
function updateProviderKeySection(provider) {
  const section = document.getElementById("provider-key-section");
  if (section) {
    section.style.display = (provider && provider !== "local") ? "block" : "none";
  }
}

document.getElementById("provider-select")?.addEventListener("change", (e) => {
  updateProviderKeySection(e.target.value);
});

document.getElementById("btn-save-provider-key")?.addEventListener("click", async () => {
  const provider = document.getElementById("provider-select")?.value;
  const key = document.getElementById("provider-api-key")?.value || "";
  if (!provider || !key) return;
  try {
    await invoke("save_api_key", { provider, key });
    const btn = document.getElementById("btn-save-provider-key");
    btn.textContent = "[ SAVED ]";
    setTimeout(() => { btn.textContent = "[ SAVE KEY ]"; }, 1500);
    document.getElementById("provider-api-key").value = "";
  } catch (err) {
    alert("Failed to save key: " + err);
  }
});

// ============================================================
// Model management
// ============================================================
async function refreshModelList() {
  try {
    const models = await invoke("get_model_status");
    const list = document.getElementById("model-list");
    if (!list) return;
    list.innerHTML = "";
    models.forEach(m => {
      const row = document.createElement("div");
      row.className = "model-row";
      row.innerHTML = `
        <span class="model-name">${m.model}</span>
        <span class="model-size">${m.downloaded ? m.size_mb.toFixed(1) + " MB" : "—"}</span>
        <span class="${m.downloaded ? "model-status-ok" : "model-status-no"}">
          ${m.downloaded ? "[OK]" : "[NOT DOWNLOADED]"}
        </span>
      `;
      list.appendChild(row);
    });
  } catch (err) {
    console.error("Failed to get model status:", err);
  }
}

document.getElementById("btn-download-model")?.addEventListener("click", async () => {
  const model = document.getElementById("model-select")?.value;
  if (!model) return;
  const progressWrap = document.getElementById("download-progress-wrap");
  const progressBar = document.getElementById("download-progress-bar");
  const progressLabel = document.getElementById("download-progress-label");
  if (progressWrap) progressWrap.style.display = "block";
  try {
    await invoke("download_model", { model });
    await refreshModelList();
  } catch (err) {
    alert("Download failed: " + err);
  }
  if (progressWrap) progressWrap.style.display = "none";
});

// ============================================================
// Prompt reset
// ============================================================
document.getElementById("btn-reset-prompt")?.addEventListener("click", () => {
  const promptIn = document.getElementById("prompt-input");
  if (promptIn) promptIn.value = "";
});

// ============================================================
// Update check (stub — checks GitHub releases)
// ============================================================
document.getElementById("btn-check-updates")?.addEventListener("click", async () => {
  const resultBox = document.getElementById("update-result");
  const msg = document.getElementById("update-message");
  if (resultBox) resultBox.style.display = "block";
  if (msg) msg.textContent = "Checking...";
  try {
    const resp = await fetch(
      "https://api.github.com/repos/OnlyGetC/speakyfi-windows/releases/latest"
    );
    const data = await resp.json();
    const latest = data.tag_name || "unknown";
    const current = "v" + (config?.version || "1.6.0");
    if (msg) {
      msg.textContent = latest === current
        ? "You are up to date (" + current + ")"
        : "New version available: " + latest + " (current: " + current + ")";
    }
  } catch (err) {
    if (msg) msg.textContent = "Check failed: " + err.message;
  }
});

// ============================================================
// Download progress events
// ============================================================
listen("model-download-progress", (e) => {
  const { model, progress } = e.payload;
  const bar = document.getElementById("download-progress-bar");
  const label = document.getElementById("download-progress-label");
  if (bar) bar.style.width = progress + "%";
  if (label) label.textContent = progress + "%";
});

listen("model-download-complete", async (e) => {
  await refreshModelList();
});

// ============================================================
// Init
// ============================================================
async function init() {
  await loadConfig();
  await refreshModelList();
}

init();
