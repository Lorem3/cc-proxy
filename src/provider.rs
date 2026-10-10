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
    model_urls: HashMap<String, String>,
    #[serde(default)]
    model_keys: HashMap<String, String>,
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

fn resolve_mapping(cfg: ModelMappingConfig) -> HashMap<String, PlatformConfig> {
    let model_urls = cfg.model_urls;
    let model_keys = cfg.model_keys;
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

    // Resolve short apiUrl values using model_urls
    for (key, cfg) in resolved.iter_mut() {
        if !cfg.api_url.starts_with("http://") && !cfg.api_url.starts_with("https://") {
            if let Some(resolved_url) = model_urls.get(&cfg.api_url) {
                tracing::debug!(
                    "Resolved apiUrl '{}' → '{}' for key '{}' via model_urls",
                    cfg.api_url,
                    resolved_url,
                    key
                );
                cfg.api_url = resolved_url.clone();
            } else {
                tracing::warn!(
                    "apiUrl '{}' for key '{}' is not a full URL and not found in model_urls; skipping",
                    cfg.api_url,
                    key
                );
            }
        }
    }

    // Resolve $apiKey references using model_keys
    resolved.retain(|key, cfg| {
        let Some(ref_name) = cfg.api_key.strip_prefix('$') else {
            return true;
        };
        if let Some(real_key) = model_keys.get(ref_name) {
            tracing::debug!(
                "Resolved apiKey '${}' via model_keys for key '{}'",
                ref_name,
                key
            );
            cfg.api_key = real_key.clone();
            true
        } else {
            tracing::warn!(
                "apiKey '${}' for key '{}' not found in model_keys; skipping",
                ref_name,
                key
            );
            false
        }
    });

    resolved
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
    Ok(resolve_mapping(cfg))
}

/// Which rule produced a model_mapping hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Learned,
    Config,
    Wildcard,
}

/// Find the best matching model_mapping entry for a request model.
///
/// Order:
/// 1. In-memory exact records (case-insensitive full key equality).
/// 2. Config keys without `*`: case-insensitive substring, longer keys win.
/// 3. Config keys with `*`: case-insensitive glob. More literal characters win.
pub fn find_model_mapping(
    mapping: &HashMap<String, PlatformConfig>,
    learned: &HashMap<String, PlatformConfig>,
    model: &str,
) -> Option<(String, PlatformConfig, MatchKind)> {
    if let Some((key, cfg)) = learned
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(model))
    {
        return Some((key.clone(), cfg.clone(), MatchKind::Learned));
    }

    let model_lower = model.to_ascii_lowercase();

    if let Some((key, cfg)) = mapping
        .iter()
        .filter(|(key, _)| !key.contains('*') && model_lower.contains(&key.to_ascii_lowercase()))
        .max_by_key(|(key, _)| key.len())
    {
        return Some((key.clone(), cfg.clone(), MatchKind::Config));
    }

    mapping
        .iter()
        .filter(|(key, _)| key.contains('*') && glob_match(key, model))
        .max_by_key(|(key, _)| glob_rank(key))
        .map(|(key, cfg)| (key.clone(), cfg.clone(), MatchKind::Wildcard))
}

/// Build the in-memory exact entry for a wildcard hit.
/// A missing or empty `name` becomes the requested model so the next lookup
/// keeps passing that name through.
pub fn promote_wildcard_match(mut cfg: PlatformConfig, model: &str) -> PlatformConfig {
    if cfg.name.as_ref().map(|name| name.is_empty()).unwrap_or(true) {
        cfg.name = Some(model.to_string());
    }
    cfg
}

/// `Some(name)` when the forwarded body should replace `model`.
/// Equal names stay on the original body.
pub fn rename_model<'a>(name: Option<&'a str>, model: &str) -> Option<&'a str> {
    match name {
        Some(name) if !name.is_empty() && name != model => Some(name),
        _ => None,
    }
}

fn glob_rank(pattern: &str) -> (usize, std::cmp::Reverse<usize>, usize) {
    let stars = pattern.bytes().filter(|byte| *byte == b'*').count();
    let literal = pattern.len().saturating_sub(stars);
    (literal, std::cmp::Reverse(stars), pattern.len())
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let text = text.to_ascii_lowercase();
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();

    let mut pattern_index = 0;
    let mut text_index = 0;
    let mut star_pattern: Option<usize> = None;
    let mut star_text = 0;

    while text_index < text.len() {
        if pattern_index < pattern.len() && pattern[pattern_index] == text[text_index] {
            pattern_index += 1;
            text_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_pattern = Some(pattern_index);
            star_text = text_index;
            pattern_index += 1;
        } else if let Some(saved) = star_pattern {
            pattern_index = saved + 1;
            star_text += 1;
            text_index = star_text;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
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
        let resolved = resolve_mapping(cfg);

        let (key, platform_cfg, _) =
            find_model_mapping(&resolved, &HashMap::new(), "mimo-v2.5-pro-chat").unwrap();
        assert_eq!(key, "mimo-v2.5");
        assert_eq!(platform_cfg.api_key, "sk-fallback");
        assert!(!resolved.contains_key("mimo-v2.5-pro"));
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

        let (_, cfg, _) =
            find_model_mapping(&mapping, &HashMap::new(), "claude-sonnet-4-5").unwrap();
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

        let (key, cfg, _) = find_model_mapping(&mapping, &HashMap::new(), "mimo-v2.5-pro").unwrap();
        assert_eq!(key, "mimo-v2.5-pro");
        assert_eq!(cfg.api_key, "sk-pro");

        let (key, _, _) =
            find_model_mapping(&mapping, &HashMap::new(), "custom-mimo-v2.5-chat").unwrap();
        assert_eq!(key, "mimo-v2.5");

        let (_, cfg, _) =
            find_model_mapping(&mapping, &HashMap::new(), "deepseek-v3-chat").unwrap();
        assert_eq!(cfg.api_url, "https://api.deepseek.com/v1");
        assert_eq!(cfg.name.as_deref(), Some("deepseek-v4-pro"));
    }

    #[test]
    fn model_urls_resolves_short_api_url() {
        let json = r#"
        {
            "model_urls": {
                "mimo": "https://api.xiaomimimo.com/anthropic",
                "dashscope": "https://dashscope.aliyuncs.com/apps/anthropic"
            },
            "model_mapping": {
                "mimo_A": {
                    "apiUrl": "mimo",
                    "apiKey": "sk-key",
                    "name": "mimo-v2.5"
                },
                "mimo_B": {
                    "apiUrl": "https://api.deepseek.com/v1",
                    "apiKey": "sk-ds"
                }
            }
        }
        "#;

        let cfg: ModelMappingConfig = serde_json::from_str(json).unwrap();
        let resolved = resolve_mapping(cfg);

        assert_eq!(
            resolved.get("mimo_A").unwrap().api_url,
            "https://api.xiaomimimo.com/anthropic"
        );
        assert_eq!(
            resolved.get("mimo_B").unwrap().api_url,
            "https://api.deepseek.com/v1"
        );
    }

    #[test]
    fn model_keys_resolves_dollar_prefixed_api_key() {
        let json = r#"
        {
            "model_keys": {
                "AAA": "sk-real-key-1",
                "BBB": "sk-real-key-2"
            },
            "model_mapping": {
                "sonnet": {
                    "apiUrl": "https://api.anthropic.com",
                    "apiKey": "$AAA"
                },
                "deepseek": {
                    "apiUrl": "https://api.deepseek.com/v1",
                    "apiKey": "sk-literal-key"
                }
            }
        }
        "#;

        let cfg: ModelMappingConfig = serde_json::from_str(json).unwrap();
        let resolved = resolve_mapping(cfg);

        assert_eq!(resolved.get("sonnet").unwrap().api_key, "sk-real-key-1");
        assert_eq!(resolved.get("deepseek").unwrap().api_key, "sk-literal-key");
    }

    #[test]
    fn model_keys_skips_missing_reference() {
        let json = r#"
        {
            "model_keys": {
                "AAA": "sk-real-key-1"
            },
            "model_mapping": {
                "sonnet": {
                    "apiUrl": "https://api.anthropic.com",
                    "apiKey": "$MISSING"
                },
                "haiku": {
                    "apiUrl": "https://api.anthropic.com",
                    "apiKey": "$AAA"
                }
            }
        }
        "#;

        let cfg: ModelMappingConfig = serde_json::from_str(json).unwrap();
        let resolved = resolve_mapping(cfg);

        assert!(!resolved.contains_key("sonnet"));
        assert_eq!(resolved.get("haiku").unwrap().api_key, "sk-real-key-1");
    }

    #[test]
    fn model_keys_resolves_after_alias_expansion() {
        let json = r#"
        {
            "model_keys": {
                "AAA": "sk-shared-key"
            },
            "model_mapping": {
                "provider_shared": {
                    "apiUrl": "https://api.shared.com/v1",
                    "apiKey": "$AAA",
                    "name": "shared-model"
                },
                "deepseek-v3": "provider_shared"
            }
        }
        "#;

        let cfg: ModelMappingConfig = serde_json::from_str(json).unwrap();
        let resolved = resolve_mapping(cfg);

        assert_eq!(
            resolved.get("provider_shared").unwrap().api_key,
            "sk-shared-key"
        );
        assert_eq!(
            resolved.get("deepseek-v3").unwrap().api_key,
            "sk-shared-key"
        );
        assert_eq!(
            resolved.get("deepseek-v3").unwrap().name.as_deref(),
            Some("shared-model")
        );
    }

    fn sample_cfg(api_key: &str, name: Option<&str>) -> PlatformConfig {
        PlatformConfig {
            api_url: "https://upstream.example/v1".to_string(),
            api_key: api_key.to_string(),
            name: name.map(str::to_string),
        }
    }

    #[test]
    fn substring_match_beats_wildcard() {
        let mut mapping = HashMap::new();
        mapping.insert("gpt".to_string(), sample_cfg("substring-key", None));
        mapping.insert("gpt-*".to_string(), sample_cfg("wildcard-key", None));

        let (key, cfg, kind) = find_model_mapping(&mapping, &HashMap::new(), "gpt-aaa").unwrap();
        assert_eq!(key, "gpt");
        assert_eq!(cfg.api_key, "substring-key");
        assert_eq!(kind, MatchKind::Config);
    }

    #[test]
    fn wildcard_matches_when_no_plain_key_hits() {
        let mut mapping = HashMap::new();
        mapping.insert("sonnet".to_string(), sample_cfg("sonnet-key", None));
        mapping.insert(
            "gpt-*".to_string(),
            sample_cfg("wildcard-key", Some("gpt-6.1-sol")),
        );

        let (key, cfg, kind) = find_model_mapping(&mapping, &HashMap::new(), "gpt-aaa").unwrap();
        assert_eq!(key, "gpt-*");
        assert_eq!(kind, MatchKind::Wildcard);
        assert_eq!(cfg.api_key, "wildcard-key");
        assert_eq!(cfg.name.as_deref(), Some("gpt-6.1-sol"));

        assert!(find_model_mapping(&mapping, &HashMap::new(), "claude-opus").is_none());
        assert!(super::glob_match("gpt-*", "GPT-AAA"));
        assert!(!super::glob_match("gpt-*", "xg-gpt-aaa"));
        assert!(super::glob_match("*-codex", "gpt-5-codex"));
        assert!(super::glob_match("gpt-*-mini", "gpt-5-mini"));
        assert!(!super::glob_match("gpt-*-mini", "gpt-mini"));
    }

    #[test]
    fn wildcard_prefers_more_specific_pattern() {
        let mut mapping = HashMap::new();
        mapping.insert("gpt-*".to_string(), sample_cfg("broad", None));
        mapping.insert("gpt-5-*".to_string(), sample_cfg("specific", None));
        mapping.insert("*".to_string(), sample_cfg("any", None));

        let (key, cfg, kind) =
            find_model_mapping(&mapping, &HashMap::new(), "gpt-5-codex").unwrap();
        assert_eq!(key, "gpt-5-*");
        assert_eq!(kind, MatchKind::Wildcard);
        assert_eq!(cfg.api_key, "specific");

        let (key, cfg, _) = find_model_mapping(&mapping, &HashMap::new(), "gpt-aaa").unwrap();
        assert_eq!(key, "gpt-*");
        assert_eq!(cfg.api_key, "broad");
    }

    #[test]
    fn promote_wildcard_uses_request_model_when_name_missing() {
        let promoted = promote_wildcard_match(sample_cfg("sk", None), "gpt-aaa");
        assert_eq!(promoted.name.as_deref(), Some("gpt-aaa"));
        assert_eq!(promoted.api_key, "sk");
        assert!(rename_model(promoted.name.as_deref(), "gpt-aaa").is_none());

        let empty = promote_wildcard_match(sample_cfg("sk", Some("")), "gpt-aaa");
        assert_eq!(empty.name.as_deref(), Some("gpt-aaa"));
    }

    #[test]
    fn promote_wildcard_keeps_configured_name() {
        let promoted = promote_wildcard_match(sample_cfg("sk", Some("gpt-6.1-sol")), "gpt-aaa");
        assert_eq!(promoted.name.as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(
            rename_model(promoted.name.as_deref(), "gpt-aaa"),
            Some("gpt-6.1-sol")
        );
    }

    #[test]
    fn learned_exact_match_beats_wildcard_and_does_not_substring() {
        let mut mapping = HashMap::new();
        mapping.insert("gpt-*".to_string(), sample_cfg("wildcard-key", None));

        let mut learned = HashMap::new();
        learned.insert(
            "gpt-aaa".to_string(),
            promote_wildcard_match(sample_cfg("cached-key", None), "gpt-aaa"),
        );

        let (key, cfg, kind) = find_model_mapping(&mapping, &learned, "GPT-AAA").unwrap();
        assert_eq!(key, "gpt-aaa");
        assert_eq!(kind, MatchKind::Learned);
        assert_eq!(cfg.api_key, "cached-key");
        assert_eq!(cfg.name.as_deref(), Some("gpt-aaa"));

        let (key, cfg, kind) = find_model_mapping(&mapping, &learned, "gpt-aaa-extra").unwrap();
        assert_eq!(key, "gpt-*");
        assert_eq!(kind, MatchKind::Wildcard);
        assert_eq!(cfg.api_key, "wildcard-key");
    }
}
