use crate::datadir::AppDataLayout;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelCatalogEntry {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub base_url: String,
    pub model: String,
    pub source: String,
    #[serde(default)]
    pub checked_at: Option<String>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub capabilities: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelCatalog {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub models: Vec<ModelCatalogEntry>,
}

fn default_version() -> u32 { 1 }

impl Default for ModelCatalog {
    fn default() -> Self {
        Self { version: default_version(), models: Vec::new() }
    }
}

impl ModelCatalog {
    pub fn from_json(input: &str) -> Result<Self, String> {
        serde_json::from_str(input).map_err(|error| format!("模型目录解析失败: {}", error))
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| Self::from_json(&content).ok())
            .unwrap_or_default()
    }
}

pub fn load_model_catalog(layout: &AppDataLayout) -> ModelCatalog {
    ModelCatalog::load(&layout.config_dir().join("model-presets.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_keeps_unverified_entries_without_inventing_capabilities() {
        let catalog = ModelCatalog::from_json(
            r#"{"models":[{"id":"vendor-model","provider":"vendor","model":"vendor-model","source":"directory","verified":false,"capabilities":{}}]}"#,
        )
        .unwrap();
        assert_eq!(catalog.models[0].source, "directory");
        assert!(!catalog.models[0].verified);
        assert!(catalog.models[0].capabilities.is_empty());
    }
}
