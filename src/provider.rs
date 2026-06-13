use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Model mapping entry (apiUrl + apiKey + optional upstream model name).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// Get configuration file path
pub fn get_config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let dir = PathBuf::from(home).join(".cc-proxy");

    let new_path = dir.join("provider.json");
    let legacy_path = dir.join("providers.json");

    if new_path.exists() || !legacy_path.exists() {
        Ok(new_path)
    } else {
        Ok(legacy_path)
    }
}

#[derive(Debug, Deserialize, Default)]
struct ModelMappingConfig {
    #[serde(default)]
    model_mapping: HashMap<String, PlatformConfig>,
}

/// Load model-to-provider mappings from the configuration file.
/// Returns an empty map when the file is absent or the field is not present.
pub fn load_model_mapping() -> Result<HashMap<String, PlatformConfig>> {
    let config_path = get_config_path()?;
    if !config_path.exists() {
        return Ok(HashMap::new());
    }
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config: {:?}", config_path))?;
    let cfg: ModelMappingConfig = serde_json::from_str(&content).unwrap_or_default();
    Ok(cfg.model_mapping)
}

/// Find the best matching model_mapping entry for a request model.
/// Case-insensitive substring match; longer keys take priority.
pub fn find_model_mapping(
    mapping: &HashMap<String, PlatformConfig>,
    model: &str,
) -> Option<(String, PlatformConfig)> {
    let model_lower = model.to_ascii_lowercase();

    mapping
        .iter()
        .filter(|(key, _)| model_lower.contains(&key.to_ascii_lowercase()))
        .max_by_key(|(key, _)| key.len())
        .map(|(key, cfg)| (key.clone(), cfg.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_mapping_parses_name_field() {
        let json = r#"
        {
            "model_mapping": {
                "deepseek-v3": {
                    "apiUrl": "https://api.deepseek.com/v1",
                    "apiKey": "sk-ds-key",
                    "name": "deepseek-v4-pro"
                }
            }
        }
        "#;

        let cfg: ModelMappingConfig = serde_json::from_str(json).unwrap();
        let entry = cfg.model_mapping.get("deepseek-v3").unwrap();
        assert_eq!(entry.api_url, "https://api.deepseek.com/v1");
        assert_eq!(entry.api_key, "sk-ds-key");
        assert_eq!(entry.name.as_deref(), Some("deepseek-v4-pro"));
    }

    #[test]
    fn find_model_mapping_uses_case_insensitive_substring() {
        let mut mapping = HashMap::new();
        mapping.insert(
            "sonnet".to_string(),
            PlatformConfig {
                api_url: "https://sonnet.api".to_string(),
                api_key: "sonnet-key".to_string(),
                name: None,
            },
        );

        let (_, cfg) = find_model_mapping(&mapping, "claude-sonnet-4-5").unwrap();
        assert_eq!(cfg.api_url, "https://sonnet.api");
    }

    #[test]
    fn find_model_mapping_prefers_longer_keys() {
        let mut mapping = HashMap::new();
        mapping.insert(
            "mimo-v2.5".to_string(),
            PlatformConfig {
                api_url: "https://api.xiaomimimo.com/anthropic".to_string(),
                api_key: "sk-base".to_string(),
                name: None,
            },
        );
        mapping.insert(
            "mimo-v2.5-pro".to_string(),
            PlatformConfig {
                api_url: "https://api.xiaomimimo.com/anthropic".to_string(),
                api_key: "sk-pro".to_string(),
                name: None,
            },
        );
        mapping.insert(
            "deepseek-v3".to_string(),
            PlatformConfig {
                api_url: "https://api.deepseek.com/v1".to_string(),
                api_key: "sk-ds-key".to_string(),
                name: Some("deepseek-v4-pro".to_string()),
            },
        );

        let (key, cfg) = find_model_mapping(&mapping, "mimo-v2.5-pro").unwrap();
        assert_eq!(key, "mimo-v2.5-pro");
        assert_eq!(cfg.api_key, "sk-pro");

        let (key, _) = find_model_mapping(&mapping, "custom-mimo-v2.5-chat").unwrap();
        assert_eq!(key, "mimo-v2.5");

        let (_, cfg) = find_model_mapping(&mapping, "deepseek-v3-chat").unwrap();
        assert_eq!(cfg.api_url, "https://api.deepseek.com/v1");
        assert_eq!(cfg.name.as_deref(), Some("deepseek-v4-pro"));
    }
}
