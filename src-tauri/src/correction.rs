use serde::{Deserialize, Serialize};

const CORRECTION_PROMPT: &str =
    "You are a transcription corrector. Fix grammar, punctuation, and spelling errors \
     in the following transcribed text. Return only the corrected text, no explanations. \
     Preserve the original language. Text: ";

#[derive(Debug, Serialize, Deserialize)]
pub struct CorrectionRequest {
    pub text: String,
    pub mode: String,       // "off" | "ollama" | "api"
    pub endpoint: String,   // for ollama: http://localhost:11434, for api: custom OpenAI-compat URL
    pub model: String,      // e.g. "llama3.2:1b" or "mixtral-8x7b-32768"
    pub api_key: String,    // for API mode
}

/// Correct transcribed text using Ollama or an OpenAI-compatible API.
#[tauri::command]
pub async fn correct_text(request: CorrectionRequest) -> Result<String, String> {
    match request.mode.as_str() {
        "off" => Ok(request.text),
        "ollama" => correct_via_ollama(request).await,
        "api" => correct_via_api(request).await,
        other => Err(format!("Unknown correction mode: {}", other)),
    }
}

async fn correct_via_ollama(req: CorrectionRequest) -> Result<String, String> {
    let url = format!("{}/api/generate", req.endpoint.trim_end_matches('/'));
    let prompt = format!("{}{}", CORRECTION_PROMPT, req.text);

    let body = serde_json::json!({
        "model": req.model,
        "prompt": prompt,
        "stream": false,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {}. Is Ollama running?", e))?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    json["response"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("Ollama error: {}", json))
}

async fn correct_via_api(req: CorrectionRequest) -> Result<String, String> {
    let url = format!("{}/chat/completions", req.endpoint.trim_end_matches('/'));
    let prompt = format!("{}{}", CORRECTION_PROMPT, req.text);

    let body = serde_json::json!({
        "model": req.model,
        "messages": [
            { "role": "user", "content": prompt }
        ],
        "temperature": 0.1,
        "max_tokens": 1024,
    });

    let mut request_builder = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(30));

    if !req.api_key.is_empty() {
        request_builder = request_builder.bearer_auth(&req.api_key);
    }

    let resp = request_builder
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("API error: {}", json))
}
