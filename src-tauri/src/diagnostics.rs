use serde::Serialize;
use tauri::{AppHandle, Manager};

#[derive(Debug, Serialize)]
pub struct DiagnosticCheck {
    pub id: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct DiagnosticsReport {
    pub app_version: String,
    pub package_version: String,
    pub build_mode: String,
    pub build_commit: String,
    pub build_run_id: String,
    pub data_dir: String,
    pub config_path: String,
    pub keys_path: String,
    pub logs_path: String,
    pub cloud_provider: String,
    pub selected_model: String,
    pub correction_mode: String,
    pub correction_model: String,
    pub local_whisper_enabled: bool,
    pub api_keys: Vec<ProviderKeyStatus>,
    pub input_devices: Vec<String>,
    pub default_input_device: Option<String>,
    pub checks: Vec<DiagnosticCheck>,
}

#[derive(Debug, Serialize)]
pub struct ProviderKeyStatus {
    pub provider: String,
    pub configured: bool,
}

#[tauri::command]
pub fn collect_diagnostics(app: AppHandle) -> Result<DiagnosticsReport, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let config_path = data_dir.join("config.json");
    let keys_path = data_dir.join("keys.dat");
    let logs_path = data_dir.join("logs");

    let config = crate::config::load_config(app.clone()).unwrap_or_default();
    let input_devices = list_input_device_names();
    let default_input_device = default_input_device_name();

    let api_keys = ["openai", "groq", "deepgram"]
        .iter()
        .map(|provider| ProviderKeyStatus {
            provider: provider.to_string(),
            configured: crate::config::load_api_key(app.clone(), provider.to_string())
                .map(|key| !key.is_empty())
                .unwrap_or(false),
        })
        .collect::<Vec<_>>();

    let mut checks = Vec::new();
    checks.push(DiagnosticCheck {
        id: "diagnostics.config_path".to_string(),
        status: "ok".to_string(),
        detail: config_path.to_string_lossy().to_string(),
    });
    checks.push(DiagnosticCheck {
        id: "diagnostics.cloud_provider".to_string(),
        status: if config.cloud_provider == "local" && !cfg!(feature = "local-whisper") {
            "warning".to_string()
        } else {
            "ok".to_string()
        },
        detail: config.cloud_provider.clone(),
    });
    checks.push(DiagnosticCheck {
        id: "diagnostics.local_whisper".to_string(),
        status: if cfg!(feature = "local-whisper") {
            "ok".to_string()
        } else {
            "warning".to_string()
        },
        detail: if cfg!(feature = "local-whisper") {
            "enabled".to_string()
        } else {
            "disabled".to_string()
        },
    });
    checks.push(DiagnosticCheck {
        id: "diagnostics.audio_input_devices".to_string(),
        status: if input_devices.is_empty() {
            "warning".to_string()
        } else {
            "ok".to_string()
        },
        detail: format!("{} device(s)", input_devices.len()),
    });

    Ok(DiagnosticsReport {
        app_version: config.version,
        package_version: env!("CARGO_PKG_VERSION").to_string(),
        build_mode: if cfg!(feature = "local-whisper") {
            "local-whisper".to_string()
        } else {
            "cloud-only".to_string()
        },
        build_commit: short_env("GITHUB_SHA"),
        build_run_id: option_env!("GITHUB_RUN_ID").unwrap_or("local").to_string(),
        data_dir: data_dir.to_string_lossy().to_string(),
        config_path: config_path.to_string_lossy().to_string(),
        keys_path: keys_path.to_string_lossy().to_string(),
        logs_path: logs_path.to_string_lossy().to_string(),
        cloud_provider: config.cloud_provider,
        selected_model: config.model,
        correction_mode: config.correction_mode,
        correction_model: config.correction_model,
        local_whisper_enabled: cfg!(feature = "local-whisper"),
        api_keys,
        input_devices,
        default_input_device,
        checks,
    })
}

fn short_env(name: &str) -> String {
    let value = match name {
        "GITHUB_SHA" => option_env!("GITHUB_SHA"),
        _ => None,
    }
    .unwrap_or("local");

    value.chars().take(12).collect()
}

fn list_input_device_names() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };

    devices.filter_map(|device| device.name().ok()).collect()
}

fn default_input_device_name() -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    cpal::default_host()
        .default_input_device()
        .and_then(|device| device.name().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_key_status_serializes_without_secret_material() {
        let status = ProviderKeyStatus {
            provider: "openai".to_string(),
            configured: true,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("openai"));
        assert!(json.contains("configured"));
        assert!(!json.contains("sk-"));
    }

    #[test]
    fn diagnostic_check_has_stable_fields() {
        let check = DiagnosticCheck {
            id: "diagnostics.config_path".to_string(),
            status: "ok".to_string(),
            detail: "config.json".to_string(),
        };

        let value = serde_json::to_value(check).unwrap();
        assert_eq!(value["id"], "diagnostics.config_path");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["detail"], "config.json");
    }

    #[test]
    fn short_env_returns_stable_value() {
        assert!(!short_env("GITHUB_SHA").is_empty());
    }
}
