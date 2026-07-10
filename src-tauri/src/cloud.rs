use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[derive(Debug, Serialize, Deserialize)]
pub struct CloudTranscribeRequest {
    pub provider: String,
    pub audio_b64: String,
    pub language: String,
}

/// Transcribe audio using a cloud provider (OpenAI / Groq / Deepgram).
/// audio_b64: base64-encoded raw f32 PCM at 16kHz mono, converted to WAV internally.
#[tauri::command]
pub async fn cloud_transcribe(
    app: AppHandle,
    provider: String,
    audio_b64: String,
    language: String,
) -> Result<String, String> {
    // Load API key from secure storage
    let api_key = crate::config::load_api_key(app, provider.clone())?;
    if api_key.is_empty() {
        return Err(format!("No API key configured for provider: {}", provider));
    }

    // Decode base64 audio → f32 PCM
    let raw = base64_decode(&audio_b64).map_err(|e| e.to_string())?;
    let audio: Vec<f32> = raw
        .chunks(4)
        .map(|b| {
            let bytes = [b[0], b.get(1).copied().unwrap_or(0),
                          b.get(2).copied().unwrap_or(0), b.get(3).copied().unwrap_or(0)];
            f32::from_le_bytes(bytes)
        })
        .collect();

    // Encode to WAV bytes
    let wav_bytes = pcm_to_wav(&audio, 16000).map_err(|e| e.to_string())?;

    match provider.as_str() {
        "openai" => openai_transcribe(wav_bytes, &language, &api_key).await,
        "groq"   => groq_transcribe(wav_bytes, &language, &api_key).await,
        "deepgram" => deepgram_transcribe(wav_bytes, &language, &api_key).await,
        other    => Err(format!("Unknown cloud provider: {}", other)),
    }
}

async fn openai_transcribe(wav: Vec<u8>, language: &str, key: &str) -> Result<String, String> {
    use reqwest::multipart;

    let part = multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;

    let mut form = multipart::Form::new()
        .text("model", "whisper-1")
        .part("file", part);

    if language != "auto" && !language.is_empty() {
        form = form.text("language", language.to_string());
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    json["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("OpenAI error: {}", json))
}

async fn groq_transcribe(wav: Vec<u8>, language: &str, key: &str) -> Result<String, String> {
    use reqwest::multipart;

    let part = multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;

    let mut form = multipart::Form::new()
        .text("model", "whisper-large-v3-turbo")
        .part("file", part);

    if language != "auto" && !language.is_empty() {
        form = form.text("language", language.to_string());
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .bearer_auth(key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    json["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Groq error: {}", json))
}

async fn deepgram_transcribe(wav: Vec<u8>, language: &str, key: &str) -> Result<String, String> {
    let lang_param = if language == "auto" || language.is_empty() {
        String::new()
    } else {
        format!("&language={}", language)
    };

    let url = format!(
        "https://api.deepgram.com/v1/listen?model=nova-2&smart_format=true{}",
        lang_param
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Token {}", key))
        .header("Content-Type", "audio/wav")
        .body(wav)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    json["results"]["channels"][0]["alternatives"][0]["transcript"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Deepgram error: {}", json))
}

/// Encode Vec<f32> PCM (16kHz mono) to WAV bytes (PCM 16-bit).
fn pcm_to_wav(audio: &[f32], sample_rate: u32) -> anyhow::Result<Vec<u8>> {
    use std::io::Cursor;
    let mut cursor = Cursor::new(Vec::new());
    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
        for &sample in audio {
            let s = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(s)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}

fn base64_decode(s: &str) -> Result<Vec<u8>, &'static str> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = Vec::new();
    let chars: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
    for chunk in chars.chunks(4) {
        let decode = |c: u8| alphabet.iter().position(|&x| x == c).ok_or("invalid base64");
        let b0 = decode(chunk[0])? as u8;
        let b1 = decode(chunk[1])? as u8;
        result.push((b0 << 2) | (b1 >> 4));
        if chunk.len() > 2 {
            let b2 = decode(chunk[2])? as u8;
            result.push(((b1 & 0xf) << 4) | (b2 >> 2));
        }
        if chunk.len() > 3 {
            let b2 = decode(chunk[2])? as u8;
            let b3 = decode(chunk[3])? as u8;
            result.push(((b2 & 3) << 6) | b3);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_to_wav_writes_riff_header() {
        let audio = vec![0.0_f32, 0.5, -0.5, 1.0, -1.0];
        let wav = pcm_to_wav(&audio, 16000).unwrap();

        assert!(wav.starts_with(b"RIFF"));
        assert_eq!(&wav[8..12], b"WAVE");
        assert!(wav.len() > 44);
    }

    #[test]
    fn base64_decode_handles_float32_pcm_bytes() {
        let encoded = "AAAAAAAAgD8AAIA/";
        let bytes = base64_decode(encoded).unwrap();
        let samples: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();

        assert_eq!(samples, vec![0.0, 1.0, 1.0]);
    }
}
