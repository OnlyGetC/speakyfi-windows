use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Manager;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub ptt_key: u32,
    pub ptt_modifiers: u32,
    pub vad_toggle_key: u32,
    pub vad_toggle_modifiers: u32,
    pub model: String,
    pub language: String,
    pub prompt: String,
    pub correction_mode: String,
    pub correction_endpoint: String,
    pub correction_model: String,
    pub interface_lang: String,
    pub cloud_provider: String,
    pub version: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ptt_key: 0x11, // VK_CONTROL
            ptt_modifiers: 0,
            vad_toggle_key: 0,
            vad_toggle_modifiers: 0,
            model: "base".to_string(),
            language: "auto".to_string(),
            prompt: String::new(),
            correction_mode: "off".to_string(),
            correction_endpoint: "http://localhost:11434".to_string(),
            correction_model: "llama3.2:1b".to_string(),
            interface_lang: "en".to_string(),
            cloud_provider: "local".to_string(),
            version: "1.6.0".to_string(),
        }
    }
}

fn config_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .expect("no app data dir")
        .join("config.json")
}

fn keys_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .expect("no app data dir")
        .join("keys.dat")
}

#[tauri::command]
pub fn load_config(app: AppHandle) -> Result<AppConfig, String> {
    let path = config_path(&app);
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let contents = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&contents).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_config(app: AppHandle, config: AppConfig) -> Result<(), String> {
    let path = config_path(&app);
    let contents = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

/// Save an API key encrypted with AES-256-GCM.
/// The key material is derived from machine-uid (Windows only).
#[tauri::command]
pub fn save_api_key(app: AppHandle, provider: String, key: String) -> Result<(), String> {
    use aes_gcm::{
        aead::{Aead, KeyInit, OsRng},
        Aes256Gcm, Key, Nonce,
    };
    use rand::RngCore;

    let machine_key = derive_machine_key();
    let cipher_key = Key::<Aes256Gcm>::from_slice(&machine_key);
    let cipher = Aes256Gcm::new(cipher_key);

    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, key.as_bytes())
        .map_err(|e| e.to_string())?;

    // Load existing keys store or create new
    let path = keys_path(&app);
    let mut store: serde_json::Value = if path.exists() {
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&raw).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Store nonce + ciphertext as base64
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    store[&provider] = serde_json::Value::String(base64_encode(&combined));

    std::fs::write(&path, store.to_string()).map_err(|e| e.to_string())
}

/// Load and decrypt an API key.
#[tauri::command]
pub fn load_api_key(app: AppHandle, provider: String) -> Result<String, String> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Key, Nonce,
    };

    let path = keys_path(&app);
    if !path.exists() {
        return Ok(String::new());
    }

    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let store: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    let encoded = store[&provider]
        .as_str()
        .ok_or("Key not found")?
        .to_string();

    let combined = base64_decode(&encoded).map_err(|e| e.to_string())?;
    if combined.len() < 12 {
        return Err("Invalid key data".to_string());
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let machine_key = derive_machine_key();
    let cipher_key = Key::<Aes256Gcm>::from_slice(&machine_key);
    let cipher = Aes256Gcm::new(cipher_key);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "Decryption failed".to_string())?;

    String::from_utf8(plaintext).map_err(|e| e.to_string())
}

/// Derive a 32-byte key from machine UID (Windows) or a fallback.
fn derive_machine_key() -> [u8; 32] {
    #[cfg(target_os = "windows")]
    {
        let uid = machine_uid::get().unwrap_or_else(|_| "speakyfi-fallback-key".to_string());
        let mut key = [0u8; 32];
        let bytes = uid.as_bytes();
        for (i, b) in bytes.iter().take(32).enumerate() {
            key[i] = *b;
        }
        // XOR-fill remaining bytes for entropy
        for i in bytes.len().min(32)..32 {
            key[i] = 0x53 ^ (i as u8);
        }
        key
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Dev/CI fallback
        let mut key = [0u8; 32];
        let fallback = b"speakyfi-dev-key-placeholder----";
        key.copy_from_slice(fallback);
        key
    }
}

fn base64_encode(data: &[u8]) -> String {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        result.push(alphabet[b0 >> 2] as char);
        result.push(alphabet[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            result.push(alphabet[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(alphabet[b2 & 0x3f] as char);
        } else {
            result.push('=');
        }
    }
    result
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
    fn default_config_uses_cloud_or_local_provider_field() {
        let config = AppConfig::default();

        assert_eq!(config.version, "1.6.0");
        assert_eq!(config.language, "auto");
        assert_eq!(config.cloud_provider, "local");
    }

    #[test]
    fn base64_helpers_roundtrip_binary_data() {
        let data = b"speakyfi\0binary\0data";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();

        assert_eq!(decoded, data);
    }

    #[test]
    fn derived_machine_key_is_32_bytes() {
        let key = derive_machine_key();

        assert_eq!(key.len(), 32);
    }
}
