use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    let dir = PathBuf::from(home).join(".cc-mapping");

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
    model_mapping: HashMap<String, MappingEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MappingEntry {
    Direct(PlatformConfig),
    Alias(String),
}

fn resolve_mapping_alias(
    model_mapping: &HashMap<String, MappingEntry>,
    alias_key: &str,
    visited: &mut HashSet<String>,
) -> Option<PlatformConfig> {
    if !visited.insert(alias_key.to_string()) {
        tracing::warn!("model_mapping alias cycle detected at '{}'", alias_key);
        return None;
    }

    match model_mapping.get(alias_key)? {
        MappingEntry::Direct(platform_cfg) => Some(platform_cfg.clone()),
        MappingEntry::Alias(next_key) => resolve_mapping_alias(model_mapping, next_key, visited),
    }
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
    let model_mapping = cfg.model_mapping;
    let mut resolved = HashMap::new();

    for (model_key, entry) in &model_mapping {
        match entry {
            MappingEntry::Direct(platform_cfg) => {
                resolved.insert(model_key.clone(), platform_cfg.clone());
            }
            MappingEntry::Alias(alias_key) => {
                if let Some(platform_cfg) =
                    resolve_mapping_alias(&model_mapping, alias_key, &mut HashSet::new())
                {
                    resolved.insert(model_key.clone(), platform_cfg);
                } else {
                    tracing::warn!(
                        "model_mapping alias '{}' for key '{}' not found in model_mapping; skipping",
                        alias_key,
                        model_key
                    );
                }
            }
        }
    }

    Ok(resolved)
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
        let MappingEntry::Direct(entry) = cfg.model_mapping.get("deepseek-v3").unwrap() else {
            panic!("expected direct mapping entry");
        };
        assert_eq!(entry.api_url, "https://api.deepseek.com/v1");
        assert_eq!(entry.api_key, "sk-ds-key");
        assert_eq!(entry.name.as_deref(), Some("deepseek-v4-pro"));
    }

    #[test]
    fn model_mapping_parses_alias_and_resolves_via_model_mapping() {
        let json = r#"
        {
            "model_mapping": {
                "AAA": {
                    "apiUrl": "https://api.alias.com/v1",
                    "apiKey": "sk-alias-key",
                    "name": "alias-model"
                },
                "deepseek-v3": "AAA"
            }
        }
        "#;

        let cfg: ModelMappingConfig = serde_json::from_str(json).unwrap();
        assert!(matches!(
            cfg.model_mapping.get("deepseek-v3"),
            Some(MappingEntry::Alias(v)) if v == "AAA"
        ));
        let MappingEntry::Direct(alias_cfg) = cfg.model_mapping.get("AAA").unwrap() else {
            panic!("expected direct mapping entry for alias target");
        };
        assert_eq!(alias_cfg.api_url, "https://api.alias.com/v1");
        assert_eq!(alias_cfg.api_key, "sk-alias-key");
        assert_eq!(alias_cfg.name.as_deref(), Some("alias-model"));

        let resolved =
            resolve_mapping_alias(&cfg.model_mapping, "AAA", &mut HashSet::new()).unwrap();
        assert_eq!(resolved.api_url, "https://api.alias.com/v1");
    }

    #[test]
    fn find_model_mapping_skips_missing_alias_by_not_loading_it() {
        let json = r#"
        {
            "model_mapping": {
                "mimo-v2.5-pro": "MISSING_ALIAS",
                "mimo-v2.5": {
                    "apiUrl": "https://api.xiaomimimo.com/anthropic",
                    "apiKey": "sk-fallback"
                }
            }
        }
        "#;

        let cfg: ModelMappingConfig = serde_json::from_str(json).unwrap();
        let mut resolved = HashMap::new();
        for (model_key, entry) in &cfg.model_mapping {
            match entry {
                MappingEntry::Direct(platform_cfg) => {
                    resolved.insert(model_key.clone(), platform_cfg.clone());
                }
                MappingEntry::Alias(alias_key) => {
                    if let Some(platform_cfg) =
                        resolve_mapping_alias(&cfg.model_mapping, alias_key, &mut HashSet::new())
                    {
                        resolved.insert(model_key.clone(), platform_cfg);
                    }
                }
            }
        }

        let (key, platform_cfg) = find_model_mapping(&resolved, "mimo-v2.5-pro-chat").unwrap();
        assert_eq!(key, "mimo-v2.5");
        assert_eq!(platform_cfg.api_key, "sk-fallback");
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
