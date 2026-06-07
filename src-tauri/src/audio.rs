use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Global PTT buffer — filled during recording, read on stop.
static PTT_BUFFER: Mutex<Option<Vec<f32>>> = Mutex::new(None);
static PTT_RECORDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// VAD running flag.
static VAD_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
}

/// List available microphone input devices.
#[tauri::command]
pub fn list_input_devices() -> Result<Vec<DeviceInfo>, String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let default_device = host.default_input_device();
    let default_name = default_device
        .as_ref()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    let devices = host
        .input_devices()
        .map_err(|e| e.to_string())?
        .filter_map(|d| d.name().ok().map(|name| DeviceInfo {
            is_default: name == default_name,
            name,
        }))
        .collect();

    Ok(devices)
}

/// Begin buffering microphone input (PTT hold mode).
/// Audio is captured at 16kHz mono f32.
#[tauri::command]
pub fn start_ptt(_app: AppHandle) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::Ordering;

    if PTT_RECORDING.load(Ordering::SeqCst) {
        return Ok(()); // already recording
    }

    PTT_RECORDING.store(true, Ordering::SeqCst);
    {
        let mut buf = PTT_BUFFER.lock().map_err(|e| e.to_string())?;
        *buf = Some(Vec::new());
    }

    std::thread::spawn(|| {
        if let Err(e) = run_ptt_capture() {
            log::error!("PTT capture error: {}", e);
        }
    });

    Ok(())
}

fn run_ptt_capture() -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::Ordering;

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No input device available"))?;

    // Build a config for 16kHz mono f32
    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(16000),
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if PTT_RECORDING.load(Ordering::SeqCst) {
                if let Ok(mut buf) = PTT_BUFFER.lock() {
                    if let Some(ref mut v) = *buf {
                        v.extend_from_slice(data);
                    }
                }
            }
        },
        |err| log::error!("Audio stream error: {}", err),
        None,
    )?;

    stream.play()?;

    // Keep running until PTT_RECORDING is false
    while PTT_RECORDING.load(std::sync::atomic::Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    Ok(())
}

/// Stop PTT recording and return the captured audio buffer.
/// Returns Vec<f32> at 16kHz mono.
#[tauri::command]
pub fn stop_ptt() -> Result<Vec<f32>, String> {
    use std::sync::atomic::Ordering;
    PTT_RECORDING.store(false, Ordering::SeqCst);

    // Wait briefly for the capture thread to finalize
    std::thread::sleep(std::time::Duration::from_millis(50));

    let mut buf = PTT_BUFFER.lock().map_err(|e| e.to_string())?;
    Ok(buf.take().unwrap_or_default())
}

/// Start VAD (Voice Activity Detection) mode.
/// Uses simple energy threshold: RMS > 0.01 for 300ms = speech start,
/// silence for 1s = segment end. Emits "vad-segment" event with Vec<f32>.
#[tauri::command]
pub fn start_vad(app: AppHandle) -> Result<(), String> {
    use std::sync::atomic::Ordering;

    if VAD_RUNNING.load(Ordering::SeqCst) {
        return Ok(());
    }
    VAD_RUNNING.store(true, Ordering::SeqCst);

    std::thread::spawn(move || {
        if let Err(e) = run_vad_capture(app) {
            log::error!("VAD capture error: {}", e);
        }
    });

    Ok(())
}

fn run_vad_capture(app: AppHandle) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::Ordering;
    use tauri::Emitter;

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No input device available"))?;

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(16000),
        buffer_size: cpal::BufferSize::Default,
    };

    // VAD state: accumulate audio, detect speech/silence
    let samples_buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let samples_buf_clone = samples_buf.clone();

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            if VAD_RUNNING.load(Ordering::SeqCst) {
                if let Ok(mut buf) = samples_buf_clone.lock() {
                    buf.extend_from_slice(data);
                }
            }
        },
        |err| log::error!("VAD stream error: {}", err),
        None,
    )?;

    stream.play()?;

    // VAD loop: chunk-based RMS detection
    const SAMPLE_RATE: usize = 16000;
    const SPEECH_RMS: f32 = 0.01;
    const SPEECH_MIN_MS: usize = 300;   // 300ms to confirm speech
    const SILENCE_MAX_MS: usize = 1000; // 1s silence to end segment

    let speech_samples = SAMPLE_RATE * SPEECH_MIN_MS / 1000;
    let silence_samples = SAMPLE_RATE * SILENCE_MAX_MS / 1000;

    let mut speech_buffer: Vec<f32> = Vec::new();
    let mut in_speech = false;
    let mut silence_count = 0usize;
    let mut speech_count = 0usize;

    while VAD_RUNNING.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(20));

        let chunk: Vec<f32> = {
            let mut buf = samples_buf.lock().unwrap();
            let data = buf.clone();
            buf.clear();
            data
        };

        if chunk.is_empty() {
            continue;
        }

        let rms = {
            let sum_sq: f32 = chunk.iter().map(|s| s * s).sum();
            (sum_sq / chunk.len() as f32).sqrt()
        };

        let is_speech = rms > SPEECH_RMS;

        if is_speech {
            speech_buffer.extend_from_slice(&chunk);
            speech_count += chunk.len();
            silence_count = 0;
            in_speech = speech_count >= speech_samples;
        } else if in_speech {
            speech_buffer.extend_from_slice(&chunk);
            silence_count += chunk.len();

            if silence_count >= silence_samples {
                // Segment complete — emit event
                let segment = speech_buffer.clone();
                let _ = app.emit("vad-segment", segment);
                speech_buffer.clear();
                in_speech = false;
                speech_count = 0;
                silence_count = 0;
            }
        } else {
            // Pre-speech: discard
            speech_count = 0;
        }
    }

    Ok(())
}

/// Stop VAD mode.
#[tauri::command]
pub fn stop_vad() -> Result<(), String> {
    VAD_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}
