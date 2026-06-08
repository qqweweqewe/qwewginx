use std::path::PathBuf;

use crate::config::{PluginEntry, Plugins};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginCapabilities {
    pub request_hook: bool,
    pub master_bus: bool,
    pub http_endpoint: bool,
    pub http_endpoint_scope_master: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub sha256: String,
    pub capabilities: PluginCapabilities,
}

pub struct PluginHost {
    cache_root: PathBuf,
    plugins: Plugins,
}

impl PluginHost {
    pub fn from_config(plugins: Plugins, cache_root: impl Into<PathBuf>) -> Self {
        Self {
            cache_root: cache_root.into(),
            plugins,
        }
    }

    pub fn plugins(&self) -> &Plugins {
        &self.plugins
    }

    pub fn cache_dir_for(&self, entry: &PluginEntry) -> PathBuf {
        self.cache_root
            .join(&entry.name)
            .join(&entry.version)
    }

    pub fn wasm_path_for(&self, entry: &PluginEntry) -> PathBuf {
        self.cache_dir_for(entry).join("plugin.wasm")
    }

    pub fn manifest_path_for(&self, entry: &PluginEntry) -> PathBuf {
        self.cache_dir_for(entry).join("manifest.json")
    }

    pub fn master_init_all(&self) -> Result<(), String> {
        for entry in &self.plugins.entries {
            let wasm = self.wasm_path_for(entry);
            if !wasm.is_file() {
                return Err(format!(
                    "plugin {}@{}: missing {}",
                    entry.name,
                    entry.version,
                    wasm.display()
                ));
            }
            let _manifest = self.manifest_path_for(entry);
            // TODO(feature-16): wasmtime load, validate_config, on_master_init
            let _ = wasm.as_path();
        }
        Ok(())
    }
}
