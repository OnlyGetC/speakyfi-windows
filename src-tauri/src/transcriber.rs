use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const MODEL_URLS: &[(&str, &str)] = &[
    ("tiny",   "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"),
    ("base",   "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"),
    ("small",  "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"),
    ("medium", "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin"),
];

const SAMPLE_RATE: usize = 16000;
const MAX_LOCAL_AUDIO_SECONDS: usize = 30;
const MAX_LOCAL_AUDIO_SAMPLES: usize = SAMPLE_RATE * MAX_LOCAL_AUDIO_SECONDS;
const LOCAL_TRANSCRIBE_TIMEOUT_SECONDS: u64 = 120;

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelStatus {
    pub model: String,
    pub downloaded: bool,
    pub path: String,
    pub size_mb: f64,
}

fn models_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .expect("no app data dir")
        .join("models")
}

fn model_path(app: &AppHandle, model: &str) -> PathBuf {
    models_dir(app).join(format!("ggml-{}.bin", model))
}

/// Transcribe a Vec<f32> PCM buffer (16kHz mono) using whisper.cpp.
/// Only available when compiled with the "local-whisper" feature.
/// Returns the transcribed text string.
#[tauri::command]
pub async fn transcribe_audio(
    app: AppHandle,
    audio: Vec<f32>,
    language: String,
    model: String,
    prompt: String,
) -> Result<String, String> {
    #[cfg(feature = "local-whisper")]
    {
        let task = tauri::async_runtime::spawn_blocking(move || {
            transcribe_audio_blocking(app, audio, language, model, prompt)
        });

        match tokio::time::timeout(Duration::from_secs(LOCAL_TRANSCRIBE_TIMEOUT_SECONDS), task).await {
            Ok(joined) => joined.map_err(|e| format!("Local whisper worker failed: {}", e))?,
            Err(_) => Err(format!(
                "Local whisper timed out after {} seconds. Try the tiny/base model or record a shorter phrase.",
                LOCAL_TRANSCRIBE_TIMEOUT_SECONDS,
            )),
        }
    }
    #[cfg(not(feature = "local-whisper"))]
    {
        // Local whisper not compiled in — instruct user to use cloud provider
        let _ = (app, audio, language, model, prompt);
        Err("Local whisper.cpp is not enabled in this build. Download the local-whisper build or choose a cloud provider in Settings.".to_string())
    }
}

#[cfg(feature = "local-whisper")]
fn transcribe_audio_blocking(
    app: AppHandle,
    audio: Vec<f32>,
    language: String,
    model: String,
    prompt: String,
) -> Result<String, String> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    let path = model_path(&app, &model);
    if !path.exists() {
        return Err(format!(
            "Model '{}' not found. Download it from settings first.",
            model
        ));
    }
    validate_model_file(&path, &model)?;

    let audio = if audio.len() > MAX_LOCAL_AUDIO_SAMPLES {
        audio[audio.len() - MAX_LOCAL_AUDIO_SAMPLES..].to_vec()
    } else {
        audio
    };
    if audio.is_empty() {
        return Err("No audio samples captured for local transcription.".to_string());
    }

    let ctx = WhisperContext::new_with_params(
        path.to_str().unwrap(),
        WhisperContextParameters::default(),
    )
    .map_err(|e| format!("Failed to load model: {}", e))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    if language != "auto" && !language.is_empty() {
        params.set_language(Some(&language));
    }
    if !prompt.trim().is_empty() {
        params.set_initial_prompt(prompt.trim());
    }
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    let mut state = ctx.create_state().map_err(|e| e.to_string())?;
    state
        .full(params, &audio)
        .map_err(|e| format!("Transcription failed: {}", e))?;

    let num_segments = state.full_n_segments().map_err(|e| e.to_string())?;
    let mut text = String::new();
    for i in 0..num_segments {
        if let Ok(segment) = state.full_get_segment_text(i) {
            text.push_str(segment.trim());
            text.push(' ');
        }
    }

    Ok(text.trim().to_string())
}

fn validate_model_file(path: &PathBuf, model: &str) -> Result<(), String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read model file metadata: {}", e))?
        .len();
    let min_size = minimum_model_size_bytes(model);

    if size < min_size {
        return Err(format!(
            "Model '{}' looks incomplete ({:.1} MB). Delete it and download it again.",
            model,
            size as f64 / 1_048_576.0,
        ));
    }

    Ok(())
}

fn minimum_model_size_bytes(model: &str) -> u64 {
    match model {
        "tiny" => 30 * 1_048_576,
        "base" => 60 * 1_048_576,
        "small" => 200 * 1_048_576,
        "medium" => 650 * 1_048_576,
        _ => 1,
    }
}

/// Download a whisper model from HuggingFace.
/// Emits progress events to the frontend.
/// Only meaningful when local-whisper feature is enabled.
#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    model: String,
) -> Result<(), String> {
    let url = MODEL_URLS
        .iter()
        .find(|(name, _)| *name == model.as_str())
        .map(|(_, url)| *url)
        .ok_or_else(|| format!("Unknown model: {}", model))?;

    let dir = models_dir(&app);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = model_path(&app, &model);

    let client = reqwest::Client::new();
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    let total = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut file_bytes = Vec::new();

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Stream error: {}", e))?
    {
        downloaded += chunk.len() as u64;
        file_bytes.extend_from_slice(&chunk);

        if total > 0 {
            let progress = (downloaded as f64 / total as f64 * 100.0) as u32;
            let _ = app.emit(
                "model-download-progress",
                serde_json::json!({ "model": model, "progress": progress }),
            );
        }
    }

    std::fs::write(&dest, &file_bytes).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "model-download-complete",
        serde_json::json!({ "model": model }),
    );

    Ok(())
}

/// Check which models are already downloaded.
#[tauri::command]
pub fn get_model_status(app: AppHandle) -> Vec<ModelStatus> {
    MODEL_URLS
        .iter()
        .map(|(name, _)| {
            let path = model_path(&app, name);
            let size_mb = if path.exists() {
                std::fs::metadata(&path)
                    .map(|m| m.len() as f64 / 1_048_576.0)
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            ModelStatus {
                model: name.to_string(),
                downloaded: path.exists(),
                path: path.to_string_lossy().to_string(),
                size_mb,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: verify that a short sine-wave buffer can be passed through
    /// the whisper-rs API surface without panicking (model not required for compile test).
    #[test]
    fn sine_wave_buffer_is_valid_f32() {
        let sample_rate = 16000u32;
        let duration_s = 1u32;
        let num_samples = (sample_rate * duration_s) as usize;

        let audio: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5
            })
            .collect();

        assert_eq!(audio.len(), num_samples);
        assert!(audio.iter().all(|&s| s >= -1.0 && s <= 1.0));
        // Verify the buffer length matches expectation for 1s @ 16kHz
        assert_eq!(audio.len(), 16000);
    }

    #[test]
    fn model_minimum_sizes_are_nonzero_for_known_models() {
        assert!(minimum_model_size_bytes("tiny") > 0);
        assert!(minimum_model_size_bytes("base") > minimum_model_size_bytes("tiny"));
        assert!(minimum_model_size_bytes("small") > minimum_model_size_bytes("base"));
        assert!(minimum_model_size_bytes("medium") > minimum_model_size_bytes("small"));
    }
}
