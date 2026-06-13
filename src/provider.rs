use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Platform-specific configuration (apiUrl + apiKey)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
}

/// Provider with platform-specific configs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub level: i32,
    pub name: Option<String>,
    #[serde(rename = "apiUrl")]
    pub api_url: Option<String>,
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    pub codex: Option<PlatformConfig>,
    pub claude: Option<PlatformConfig>,
}

impl Provider {
    /// Get platform-specific config for this provider
    pub fn get_platform_config(&self, kind: &str) -> Option<PlatformConfig> {
        let platform_config = match kind {
            "codex" => self.codex.clone(),
            "claude" => self.claude.clone(),
            _ => None,
        };

        if platform_config.is_some() {
            return platform_config;
        }

        // Backward-compatible fallback to a single shared config
        match (&self.api_url, &self.api_key) {
            (Some(url), Some(key)) if !url.is_empty() && !key.is_empty() => Some(PlatformConfig {
                api_url: url.clone(),
                api_key: key.clone(),
            }),
            _ => None,
        }
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PlatformConfigList {
    Single(PlatformConfig),
    List(Vec<PlatformConfig>),
}

impl PlatformConfigList {
    fn into_vec(self) -> Vec<PlatformConfig> {
        match self {
            PlatformConfigList::Single(cfg) => vec![cfg],
            PlatformConfigList::List(list) => list,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ProviderMapConfig {
    pub codex: Option<PlatformConfigList>,
    pub claude: Option<PlatformConfigList>,
}

fn flatten_map_providers(providers: ProviderMapConfig) -> Vec<Provider> {
    let mut flattened = Vec::new();

    if let Some(codex_list) = providers.codex {
        for cfg in codex_list.into_vec() {
            flattened.push(Provider {
                enabled: default_enabled(),
                level: 0,
                name: None,
                api_url: None,
                api_key: None,
                codex: Some(cfg),
                claude: None,
            });
        }
    }

    if let Some(claude_list) = providers.claude {
        for cfg in claude_list.into_vec() {
            flattened.push(Provider {
                enabled: default_enabled(),
                level: 0,
                name: None,
                api_url: None,
                api_key: None,
                codex: None,
                claude: Some(cfg),
            });
        }
    }

    flattened
}

/// Load providers from configuration file.
/// Returns an empty list when `providers` is absent (e.g. model_mapping-only config).
pub fn load_providers() -> Result<Vec<Provider>> {
    let config_path = get_config_path()?;

    if !config_path.exists() {
        tracing::warn!("Provider config not found: {:?}", config_path);
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read provider config: {:?}", config_path))?;

    let root: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse provider config: {:?}", config_path))?;

    let Some(providers_value) = root.get("providers") else {
        tracing::info!("No providers field in config; routing via model_mapping only");
        return Ok(Vec::new());
    };

    if providers_value.is_null() {
        return Ok(Vec::new());
    }

    if providers_value.is_array() {
        let providers: Vec<Provider> = serde_json::from_value(providers_value.clone())
            .with_context(|| "Failed to parse providers list")?;
        return Ok(providers);
    }

    if providers_value.is_object() {
        let map_config: ProviderMapConfig = serde_json::from_value(providers_value.clone())
            .with_context(|| "Failed to parse providers map")?;
        return Ok(flatten_map_providers(map_config));
    }

    anyhow::bail!("Invalid providers format in provider.json");
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

/// Top-level wrapper used only to extract the optional model_mapping field.
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
    fn provider_defaults_to_enabled() {
        let provider = Provider {
            enabled: default_enabled(),
            level: 0,
            name: Some("test".to_string()),
            api_url: None,
            api_key: None,
            codex: None,
            claude: None,
        };

        assert!(provider.enabled);
    }

    #[test]
    fn get_platform_config_returns_correct_platform() {
        let provider = Provider {
            enabled: true,
            level: 1,
            name: Some("test".to_string()),
            api_url: None,
            api_key: None,
            codex: Some(PlatformConfig {
                api_url: "https://codex.api.com".to_string(),
                api_key: "codex-key".to_string(),
            }),
            claude: Some(PlatformConfig {
                api_url: "https://claude.api.com".to_string(),
                api_key: "claude-key".to_string(),
            }),
        };

        let codex_config = provider.get_platform_config("codex").unwrap();
        assert_eq!(codex_config.api_url, "https://codex.api.com");

        let claude_config = provider.get_platform_config("claude").unwrap();
        assert_eq!(claude_config.api_url, "https://claude.api.com");
    }

    #[test]
    fn map_config_parses_minimal_platforms() {
        let json = r#"
        {
            "providers": {
                "codex": { "apiUrl": "https://codex.api", "apiKey": "ckey" },
                "claude": [
                    { "apiUrl": "https://claude.api", "apiKey": "akey" },
                    { "apiUrl": "https://claude2.api", "apiKey": "akey2" }
                ]
            }
        }
        "#;

        let root: serde_json::Value = serde_json::from_str(json).unwrap();
        let map_config: ProviderMapConfig =
            serde_json::from_value(root["providers"].clone()).unwrap();
        let providers = flatten_map_providers(map_config);

        assert_eq!(providers.len(), 3);
        let provider = &providers[0];
        assert!(provider.codex.is_some());
        assert!(provider.claude.is_none());
        assert_eq!(
            providers[1].claude.as_ref().unwrap().api_url,
            "https://claude.api"
        );
        assert_eq!(
            providers[2].claude.as_ref().unwrap().api_url,
            "https://claude2.api"
        );
    }

    #[test]
    fn load_providers_returns_empty_when_only_model_mapping() {
        let json = r#"
        {
            "model_mapping": {
                "gpt-4": { "apiUrl": "https://api.example.com", "apiKey": "key" }
            }
        }
        "#;

        let root: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(root.get("providers").is_none());

        let cfg: ModelMappingConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.model_mapping.len(), 1);
    }

    #[test]
    fn load_providers_parses_empty_providers_with_model_mapping() {
        let json = r#"
        {
            "providers": {},
            "model_mapping": {
                "gpt-4": { "apiUrl": "https://api.example.com", "apiKey": "key" }
            }
        }
        "#;

        let root: serde_json::Value = serde_json::from_str(json).unwrap();
        let map_config: ProviderMapConfig =
            serde_json::from_value(root["providers"].clone()).unwrap();
        let providers = flatten_map_providers(map_config);

        assert!(providers.is_empty());
    }

    #[test]
    fn get_platform_config_falls_back_to_shared_keys() {
        let provider = Provider {
            enabled: true,
            level: 0,
            name: None,
            api_url: Some("https://shared.api.com".to_string()),
            api_key: Some("shared-key".to_string()),
            codex: None,
            claude: None,
        };

        let codex_config = provider.get_platform_config("codex").unwrap();
        assert_eq!(codex_config.api_url, "https://shared.api.com");

        let claude_config = provider.get_platform_config("claude").unwrap();
        assert_eq!(claude_config.api_url, "https://shared.api.com");
    }

    #[test]
    fn find_model_mapping_uses_case_insensitive_substring() {
        let mut mapping = HashMap::new();
        mapping.insert(
            "sonnet".to_string(),
            PlatformConfig {
                api_url: "https://sonnet.api".to_string(),
                api_key: "sonnet-key".to_string(),
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
            },
        );
        mapping.insert(
            "mimo-v2.5-pro".to_string(),
            PlatformConfig {
                api_url: "https://api.xiaomimimo.com/anthropic".to_string(),
                api_key: "sk-pro".to_string(),
            },
        );
        mapping.insert(
            "deepseek-v3".to_string(),
            PlatformConfig {
                api_url: "https://api.deepseek.com/v1".to_string(),
                api_key: "sk-ds-key".to_string(),
            },
        );

        let (key, cfg) = find_model_mapping(&mapping, "mimo-v2.5-pro").unwrap();
        assert_eq!(key, "mimo-v2.5-pro");
        assert_eq!(cfg.api_key, "sk-pro");

        let (key, _) = find_model_mapping(&mapping, "custom-mimo-v2.5-chat").unwrap();
        assert_eq!(key, "mimo-v2.5");

        let (_, cfg) = find_model_mapping(&mapping, "deepseek-v3-chat").unwrap();
        assert_eq!(cfg.api_url, "https://api.deepseek.com/v1");
    }
}
