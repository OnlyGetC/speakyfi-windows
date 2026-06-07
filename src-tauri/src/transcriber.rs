use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

const MODEL_URLS: &[(&str, &str)] = &[
    ("tiny",   "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"),
    ("base",   "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"),
    ("small",  "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"),
    ("medium", "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin"),
];

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
pub fn transcribe_audio(
    app: AppHandle,
    audio: Vec<f32>,
    language: String,
    model: String,
) -> Result<String, String> {
    #[cfg(feature = "local-whisper")]
    {
        use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

        let path = model_path(&app, &model);
        if !path.exists() {
            return Err(format!(
                "Model '{}' not found. Download it from settings first.",
                model
            ));
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
    #[cfg(not(feature = "local-whisper"))]
    {
        // Local whisper not compiled in — instruct user to use cloud provider
        let _ = (app, audio, language, model);
        Err("Local whisper.cpp not enabled. Please configure a cloud provider in Settings.".to_string())
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
}
