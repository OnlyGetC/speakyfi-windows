use serde::{Deserialize, Serialize};
use std::time::Duration;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaModelStatus {
    pub endpoint: String,
    pub requested_model: String,
    pub installed_models: Vec<String>,
    pub available: bool,
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

#[tauri::command]
pub async fn check_ollama_model(endpoint: String, model: String) -> Result<OllamaModelStatus, String> {
    let installed_models = fetch_ollama_models(&endpoint).await?;
    let available = installed_models.iter().any(|name| name == &model);

    Ok(OllamaModelStatus {
        endpoint,
        requested_model: model,
        installed_models,
        available,
    })
}

async fn correct_via_ollama(req: CorrectionRequest) -> Result<String, String> {
    let installed_models = fetch_ollama_models(&req.endpoint).await?;
    if !installed_models.iter().any(|name| name == &req.model) {
        return Err(ollama_model_missing_message(&req.model, &installed_models));
    }

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
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {}. Is Ollama running?", e))?;

    let status = resp.status();
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Ollama returned HTTP {}: {}", status, json));
    }

    json["response"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| format!("Ollama error: {}", json))
}

async fn fetch_ollama_models(endpoint: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Ollama is not reachable at {}: {}. Is Ollama running?", endpoint, e))?;

    let status = resp.status();
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Ollama model list failed with HTTP {}: {}", status, json));
    }

    Ok(parse_ollama_model_names(&json))
}

fn parse_ollama_model_names(json: &serde_json::Value) -> Vec<String> {
    json["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model["name"].as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn ollama_model_missing_message(model: &str, installed_models: &[String]) -> String {
    if installed_models.is_empty() {
        return format!(
            "Ollama model '{}' is not installed. Run `ollama pull {}` or choose a model from `ollama list`.",
            model, model,
        );
    }

    format!(
        "Ollama model '{}' is not installed. Installed models: {}. Run `ollama pull {}` or choose an installed model.",
        model,
        installed_models.join(", "),
        model,
    )
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
        .timeout(Duration::from_secs(30));

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ollama_model_names_reads_names() {
        let json = serde_json::json!({
            "models": [
                { "name": "llama3.2:1b" },
                { "name": "qwen3:8b" }
            ]
        });

        assert_eq!(
            parse_ollama_model_names(&json),
            vec!["llama3.2:1b".to_string(), "qwen3:8b".to_string()],
        );
    }

    #[test]
    fn missing_model_message_contains_pull_command() {
        let msg = ollama_model_missing_message("qwen3:8b", &["llama3.2:1b".to_string()]);

        assert!(msg.contains("qwen3:8b"));
        assert!(msg.contains("ollama pull qwen3:8b"));
        assert!(msg.contains("llama3.2:1b"));
    }
}
