/**
 * Speakyfi Windows — Overlay JS
 * Handles PTT/VAD events, Tauri invocations, UI state transitions.
 */

const { invoke } = window.__TAURI__.core;
const { listen, emit } = window.__TAURI__.event;

// ============================================================
// Localization
// ============================================================
const L10N = {
  en: {
    ready: "READY",
    recording: "RECORDING...",
    processing: "PROCESSING",
    output: "OUTPUT: ",
    idleHint: "Hold {key} to record",
    copy: "[COPY]",
    copied: "[COPIED]",
    ptt: "PTT",
    vad: "VAD",
  },
  ru: {
    ready: "ГОТОВ",
    recording: "ЗАПИСЬ...",
    processing: "ОБРАБОТКА",
    output: "ТЕКСТ: ",
    idleHint: "Удержите {key} для записи",
    copy: "[КОПИРОВАТЬ]",
    copied: "[СКОПИРОВАНО]",
    ptt: "КТГ",
    vad: "АОД",
  },
};

function t(key) {
  const lang = localStorage.getItem("lang") || "en";
  return (L10N[lang] || L10N.en)[key] || key;
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
  if (vk >= 0x41 && vk <= 0x5A) return String.fromCharCode(vk);
  if (vk >= 0x30 && vk <= 0x39) return String.fromCharCode(vk);
  return VK_NAMES[vk] || "0x" + vk.toString(16).toUpperCase();
}

function hotkeyLabel(key, mods) {
  if (!key) return "Ctrl";
  const parts = [];
  if (mods & 0x0002) parts.push("Ctrl");
  if (mods & 0x0001) parts.push("Alt");
  if (mods & 0x0004) parts.push("Shift");
  if (mods & 0x0008) parts.push("Win");
  parts.push(vkCodeToName(key));
  return parts.join("+");
}

function idleHintText() {
  const key = hotkeyLabel(config?.ptt_key || 0x11, config?.ptt_modifiers || 0);
  return t("idleHint").replace("{key}", key);
}

// ============================================================
// State machine
// States: idle | recording | transcribing | result
// ============================================================
let currentState = "idle";
let pttAudioBuffer = null;
let resultDismissTimer = null;
let currentMode = "ptt"; // ptt | vad
let config = null;

// ============================================================
// UI References
// ============================================================
const els = {
  statusIcon:   document.getElementById("status-icon"),
  statusText:   document.getElementById("status-text"),
  statusMode:   document.getElementById("status-mode"),
  idleHint:     document.getElementById("idle-hint"),
  resultText:   document.getElementById("result-text"),
  footerMode:   document.getElementById("footer-mode"),
  btnCopy:      document.getElementById("btn-copy"),
  btnDismiss:   document.getElementById("btn-dismiss"),
  btnSettings:  document.getElementById("btn-settings"),
  btnClose:     document.getElementById("btn-close"),
  waveformBars: document.querySelectorAll(".bar"),
};

// ============================================================
// State transitions
// ============================================================
function setState(state, data) {
  // Hide all state views
  document.querySelectorAll(".state-view").forEach(v => v.classList.remove("active"));

  currentState = state;

  switch (state) {
    case "idle":
      document.getElementById("state-idle").classList.add("active");
      els.statusIcon.textContent = "○";
      els.statusIcon.className = "status-icon";
      els.statusText.textContent = t("ready");
      els.idleHint.textContent = idleHintText();
      if (resultDismissTimer) clearTimeout(resultDismissTimer);
      break;

    case "recording":
      document.getElementById("state-recording").classList.add("active");
      els.statusIcon.textContent = "●";
      els.statusIcon.className = "status-icon recording blink";
      els.statusText.textContent = t("recording");
      startWaveformAnimation();
      break;

    case "transcribing":
      document.getElementById("state-transcribing").classList.add("active");
      stopWaveformAnimation();
      els.statusIcon.textContent = "◌";
      els.statusIcon.className = "status-icon";
      els.statusText.textContent = t("processing");
      break;

    case "result":
      document.getElementById("state-result").classList.add("active");
      els.statusIcon.textContent = "○";
      els.statusIcon.className = "status-icon";
      els.statusText.textContent = t("ready");
      els.resultText.textContent = data || "";
      els.btnCopy.textContent = t("copy");

      // Auto-dismiss after 6 seconds
      resultDismissTimer = setTimeout(() => setState("idle"), 6000);
      break;
  }
}

// ============================================================
// Waveform animation
// ============================================================
let waveformInterval = null;

function startWaveformAnimation() {
  if (waveformInterval) return;
  waveformInterval = setInterval(() => {
    els.waveformBars.forEach(bar => {
      const h = Math.floor(Math.random() * 16) + 2;
      bar.style.height = h + "px";
    });
  }, 80);
}

function stopWaveformAnimation() {
  if (waveformInterval) {
    clearInterval(waveformInterval);
    waveformInterval = null;
  }
  els.waveformBars.forEach(bar => { bar.style.height = "4px"; });
}

// ============================================================
// Transcription pipeline
// ============================================================
async function runTranscription(audioBuffer) {
  setState("transcribing");

  try {
    const cfg = config || await loadConfig();
    let text = "";

    if (cfg.cloud_provider && cfg.cloud_provider !== "local") {
      // Cloud transcription
      const audioB64 = float32ToBase64(audioBuffer);
      text = await invoke("cloud_transcribe", {
        provider: cfg.cloud_provider,
        audioB64,
        language: cfg.language || "auto",
      });
    } else {
      // Local whisper.cpp
      text = await invoke("transcribe_audio", {
        audio: Array.from(audioBuffer),
        language: cfg.language || "auto",
        model: cfg.model || "base",
      });
    }

    // Text correction
    if (cfg.correction_mode && cfg.correction_mode !== "off") {
      text = await invoke("correct_text", {
        request: {
          text,
          mode: cfg.correction_mode,
          endpoint: cfg.correction_endpoint || "http://localhost:11434",
          model: cfg.correction_model || "llama3.2:1b",
          api_key: "",
        },
      });
    }

    // Auto-insert into active window
    if (text && text.trim()) {
      await invoke("send_text", { text: text.trim() + " " });
    }

    setState("result", text.trim());
  } catch (err) {
    console.error("Transcription error:", err);
    setState("result", "[ERROR] " + (err.message || err));
  }
}

// ============================================================
// PTT event handlers (Tauri events from hotkeys.rs)
// ============================================================
async function setupEventListeners() {
  // PTT press — start recording
  await listen("ptt-press", async () => {
    if (currentState !== "idle") return;
    try {
      await invoke("start_ptt");
      setState("recording");
    } catch (err) {
      console.error("start_ptt error:", err);
    }
  });

  // PTT release — stop recording and transcribe
  await listen("ptt-release", async () => {
    if (currentState !== "recording") return;
    try {
      const audio = await invoke("stop_ptt");
      pttAudioBuffer = new Float32Array(audio);
      if (pttAudioBuffer.length < 1600) {
        // Too short — discard
        setState("idle");
        return;
      }
      await runTranscription(pttAudioBuffer);
    } catch (err) {
      console.error("stop_ptt error:", err);
      setState("idle");
    }
  });

  // VAD toggle
  await listen("vad-toggle", async () => {
    if (currentMode === "ptt") {
      currentMode = "vad";
      els.statusMode.textContent = t("vad");
      await invoke("start_vad");
    } else {
      currentMode = "ptt";
      els.statusMode.textContent = t("ptt");
      await invoke("stop_vad");
    }
  });

  // VAD segment ready
  await listen("vad-segment", async (e) => {
    if (currentState !== "idle" && currentState !== "recording") return;
    const audio = new Float32Array(e.payload);
    await runTranscription(audio);
  });

  // Model download progress
  await listen("model-download-progress", (e) => {
    const { model, progress } = e.payload;
    console.log(`Model ${model}: ${progress}%`);
  });

  // Transcription result from any source
  await listen("transcription-result", (e) => {
    setState("result", e.payload);
  });

  await listen("config-updated", async (e) => {
    config = e.payload || await loadConfig();
    await registerHotkeysFromConfig();
    if (currentState === "idle") {
      setState("idle");
    }
  });

  await listen("hotkey-error", (e) => {
    console.error("Hotkey error:", e.payload);
    setState("result", "[HOTKEY ERROR] " + e.payload);
  });
}

// ============================================================
// Button handlers
// ============================================================
els.btnCopy.addEventListener("click", () => {
  const text = els.resultText.textContent;
  if (text) {
    navigator.clipboard.writeText(text).then(() => {
      els.btnCopy.textContent = t("copied");
      setTimeout(() => { els.btnCopy.textContent = t("copy"); }, 1500);
    });
  }
});

els.btnDismiss.addEventListener("click", () => {
  setState("idle");
});

els.btnSettings.addEventListener("click", async () => {
  try {
    await invoke("tauri", { cmd: "openWindow", label: "settings" });
  } catch {
    // Tauri v2: use window API
    const { WebviewWindow } = window.__TAURI__.webviewWindow;
    const win = await WebviewWindow.getByLabel("settings");
    if (win) {
      await win.show();
      await win.setFocus();
    }
  }
});

els.btnClose.addEventListener("click", async () => {
  const { exit } = window.__TAURI__.process;
  await exit(0);
});

// ============================================================
// Config
// ============================================================
async function loadConfig() {
  config = await invoke("load_config");
  return config;
}

async function registerHotkeysFromConfig() {
  const cfg = config || await loadConfig();
  await invoke("unregister_all_hotkeys");

  const pttKey = cfg.ptt_key || 0x11;
  const pttMods = cfg.ptt_modifiers || 0;
  await invoke("register_ptt_hotkey", { key: pttKey, modifiers: pttMods });

  if (cfg.vad_toggle_key) {
    await invoke("register_vad_toggle_hotkey", {
      key: cfg.vad_toggle_key,
      modifiers: cfg.vad_toggle_modifiers || 0,
    });
  }
}

// ============================================================
// Helpers
// ============================================================
function float32ToBase64(buffer) {
  const bytes = new Uint8Array(buffer.buffer);
  let binary = "";
  for (let i = 0; i < bytes.byteLength; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}

// ============================================================
// Init
// ============================================================
async function init() {
  try {
    config = await loadConfig();
    currentMode = "ptt";
    els.statusMode.textContent = t("ptt");
    setState("idle");
    await setupEventListeners();
    await registerHotkeysFromConfig();
  } catch (err) {
    console.error("Init error:", err);
  }
}

init();
