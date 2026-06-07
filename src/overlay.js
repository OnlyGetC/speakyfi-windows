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
    idleHint: "Hold Ctrl to record",
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
    idleHint: "Удержите Ctrl для записи",
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
      els.idleHint.textContent = t("idleHint");
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
        audiob64: audioB64,
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

    // Register PTT hotkey (default: VK_CONTROL = 0x11)
    const pttKey = config.ptt_key || 0x11;
    const pttMods = config.ptt_modifiers || 0;
    await invoke("register_ptt_hotkey", { key: pttKey, modifiers: pttMods });

    // Register VAD hotkey if configured
    if (config.vad_toggle_key) {
      await invoke("register_vad_toggle_hotkey", {
        key: config.vad_toggle_key,
        modifiers: config.vad_toggle_modifiers || 0,
      });
    }
  } catch (err) {
    console.error("Init error:", err);
  }
}

init();
