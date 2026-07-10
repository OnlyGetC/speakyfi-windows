use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

/// Global PTT buffer — filled during recording, read on stop.
static PTT_BUFFER: Mutex<Option<Vec<f32>>> = Mutex::new(None);
static PTT_CAPTURE_ERROR: Mutex<Option<String>> = Mutex::new(None);
static PTT_RECORDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static PTT_SAMPLE_RATE: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(16000);

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
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::time::Duration;

    if PTT_RECORDING.load(Ordering::SeqCst) {
        return Ok(()); // already recording
    }

    PTT_RECORDING.store(true, Ordering::SeqCst);
    {
        let mut buf = PTT_BUFFER.lock().map_err(|e| e.to_string())?;
        *buf = Some(Vec::new());
    }
    {
        let mut err = PTT_CAPTURE_ERROR.lock().map_err(|e| e.to_string())?;
        *err = None;
    }

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        if let Err(e) = run_ptt_capture(tx.clone()) {
            if let Ok(mut err) = PTT_CAPTURE_ERROR.lock() {
                *err = Some(e.to_string());
            }
            PTT_RECORDING.store(false, Ordering::SeqCst);
            log::error!("PTT capture error: {}", e);
            let _ = tx.send(Err(e.to_string()));
        }
    });

    match rx.recv_timeout(Duration::from_millis(1500)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Audio capture did not start within 1500ms".to_string()),
    }
}

fn run_ptt_capture(started_tx: std::sync::mpsc::Sender<Result<(), String>>) -> anyhow::Result<()> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::Ordering;

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No input device available"))?;

    let supported_config = device.default_input_config()?;
    let sample_format = supported_config.sample_format();
    let config = supported_config.config();
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate.0;
    PTT_SAMPLE_RATE.store(sample_rate, Ordering::SeqCst);

    let stream = match sample_format {
        cpal::SampleFormat::F32 => build_ptt_stream::<f32>(&device, &config, channels)?,
        cpal::SampleFormat::I16 => build_ptt_stream::<i16>(&device, &config, channels)?,
        cpal::SampleFormat::U16 => build_ptt_stream::<u16>(&device, &config, channels)?,
        other => anyhow::bail!("Unsupported input sample format: {:?}", other),
    };

    stream.play()?;
    let _ = started_tx.send(Ok(()));

    // Keep running until PTT_RECORDING is false
    while PTT_RECORDING.load(std::sync::atomic::Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    Ok(())
}

fn build_ptt_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
) -> anyhow::Result<cpal::Stream>
where
    T: cpal::Sample + cpal::SizedSample + Send + 'static,
    f32: FromSampleCompat<T>,
{
    use cpal::traits::DeviceTrait;
    use std::sync::atomic::Ordering;

    Ok(device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if !PTT_RECORDING.load(Ordering::SeqCst) {
                return;
            }
            if let Ok(mut buf) = PTT_BUFFER.lock() {
                if let Some(ref mut v) = *buf {
                    append_mono_samples(v, data, channels);
                }
            }
        },
        |err| {
            if let Ok(mut capture_error) = PTT_CAPTURE_ERROR.lock() {
                *capture_error = Some(format!("Audio stream error: {}", err));
            }
            log::error!("Audio stream error: {}", err);
        },
        None,
    )?)
}

trait FromSampleCompat<T> {
    fn from_sample_compat(sample: T) -> f32;
}

impl FromSampleCompat<f32> for f32 {
    fn from_sample_compat(sample: f32) -> f32 {
        sample
    }
}

impl FromSampleCompat<i16> for f32 {
    fn from_sample_compat(sample: i16) -> f32 {
        sample as f32 / i16::MAX as f32
    }
}

impl FromSampleCompat<u16> for f32 {
    fn from_sample_compat(sample: u16) -> f32 {
        (sample as f32 - 32768.0) / 32768.0
    }
}

fn append_mono_samples<T>(out: &mut Vec<f32>, data: &[T], channels: usize)
where
    T: Copy,
    f32: FromSampleCompat<T>,
{
    if channels <= 1 {
        out.extend(data.iter().copied().map(f32::from_sample_compat));
        return;
    }

    for frame in data.chunks(channels) {
        if frame.is_empty() {
            continue;
        }
        let sum: f32 = frame
            .iter()
            .copied()
            .map(f32::from_sample_compat)
            .sum();
        out.push(sum / frame.len() as f32);
    }
}

/// Stop PTT recording and return the captured audio buffer.
/// Returns Vec<f32> at 16kHz mono.
#[tauri::command]
pub fn stop_ptt() -> Result<Vec<f32>, String> {
    use std::sync::atomic::Ordering;
    PTT_RECORDING.store(false, Ordering::SeqCst);

    // Wait briefly for the capture thread to finalize
    std::thread::sleep(std::time::Duration::from_millis(50));

    if let Some(err) = PTT_CAPTURE_ERROR.lock().map_err(|e| e.to_string())?.take() {
        return Err(err);
    }

    let mut buf = PTT_BUFFER.lock().map_err(|e| e.to_string())?;
    let audio = buf.take().unwrap_or_default();
    let sample_rate = PTT_SAMPLE_RATE.load(Ordering::SeqCst);
    Ok(resample_to_16khz(&audio, sample_rate))
}

fn resample_to_16khz(audio: &[f32], source_rate: u32) -> Vec<f32> {
    const TARGET_RATE: u32 = 16000;

    if audio.is_empty() || source_rate == 0 {
        return Vec::new();
    }
    if source_rate == TARGET_RATE {
        return audio.to_vec();
    }

    let output_len = (audio.len() as u64 * TARGET_RATE as u64 / source_rate as u64) as usize;
    if output_len == 0 {
        return Vec::new();
    }

    let ratio = source_rate as f32 / TARGET_RATE as f32;
    (0..output_len)
        .map(|i| {
            let pos = i as f32 * ratio;
            let idx = pos.floor() as usize;
            let frac = pos - idx as f32;
            let a = audio.get(idx).copied().unwrap_or(0.0);
            let b = audio.get(idx + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_mono_samples_keeps_mono_f32() {
        let mut out = Vec::new();
        append_mono_samples(&mut out, &[0.0_f32, 0.25, -0.5], 1);

        assert_eq!(out, vec![0.0, 0.25, -0.5]);
    }

    #[test]
    fn append_mono_samples_averages_stereo_i16() {
        let mut out = Vec::new();
        append_mono_samples(&mut out, &[i16::MAX, i16::MAX, i16::MAX, -i16::MAX], 2);

        assert_eq!(out.len(), 2);
        assert!((out[0] - 1.0).abs() < 0.0001);
        assert!(out[1].abs() < 0.0001);
    }

    #[test]
    fn resample_to_16khz_downsamples_48khz_duration() {
        let audio = vec![0.5_f32; 48_000];
        let resampled = resample_to_16khz(&audio, 48_000);

        assert_eq!(resampled.len(), 16_000);
        assert!(resampled.iter().all(|sample| (*sample - 0.5).abs() < 0.0001));
    }

    #[test]
    fn resample_to_16khz_keeps_16khz_input() {
        let audio = vec![0.0_f32, 0.25, 0.5];
        let resampled = resample_to_16khz(&audio, 16_000);

        assert_eq!(resampled, audio);
    }
}
