use std::fs;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::host::{PluginCapabilities, PluginManifest};
use crate::config::{PluginEntry, PluginSource};

#[derive(Debug, Deserialize)]
struct ManifestFile {
    name: String,
    version: String,
    api_version: u32,
    sha256: String,
    #[serde(default)]
    #[allow(dead_code)]
    min_qwewginx: String,
    capabilities: Vec<String>,
    http_endpoint: Option<HttpEndpointFile>,
}

#[derive(Debug, Deserialize)]
struct HttpEndpointFile {
    scope: String,
}

pub fn read_manifest(path: &Path) -> Result<PluginManifest, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    parse_manifest_str(&raw)
}

pub fn parse_manifest_str(raw: &str) -> Result<PluginManifest, String> {
    let file: ManifestFile =
        serde_json::from_str(raw).map_err(|e| format!("manifest json: {e}"))?;
    if file.api_version != 1 {
        return Err(format!(
            "unsupported plugin api_version {} (want 1)",
            file.api_version
        ));
    }
    Ok(PluginManifest {
        name: file.name,
        version: file.version,
        api_version: file.api_version,
        sha256: file.sha256,
        capabilities: capabilities_from_list(&file.capabilities, file.http_endpoint.as_ref())?,
    })
}

fn capabilities_from_list(
    caps: &[String],
    http: Option<&HttpEndpointFile>,
) -> Result<PluginCapabilities, String> {
    let mut out = PluginCapabilities::default();
    for cap in caps {
        match cap.as_str() {
            "request_hook" => out.request_hook = true,
            "master_bus" => out.master_bus = true,
            "http_endpoint" => out.http_endpoint = true,
            other => return Err(format!("unknown capability: {other}")),
        }
    }
    if out.http_endpoint {
        let scope = http
            .ok_or_else(|| "http_endpoint capability requires http_endpoint object".to_string())?
            .scope
            .as_str();
        match scope {
            "master" => out.http_endpoint_scope_master = true,
            "worker" => {}
            other => return Err(format!("unknown http_endpoint.scope: {other}")),
        }
    }
    Ok(out)
}

pub fn verify_wasm_sha256(path: &Path, expected_hex: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    let got = hex::encode(digest);
    if got != expected_hex.trim().to_lowercase() {
        return Err(format!(
            "sha256 mismatch for {}: got {got}, want {expected_hex}",
            path.display()
        ));
    }
    Ok(())
}

pub fn load_and_verify(
    entry: &PluginEntry,
    manifest_path: &Path,
    wasm_path: &Path,
) -> Result<PluginManifest, String> {
    let manifest = read_manifest(manifest_path)?;
    if manifest.name != entry.name {
        return Err(format!(
            "manifest name {} does not match config {}",
            manifest.name, entry.name
        ));
    }
    if manifest.version != entry.version {
        return Err(format!(
            "manifest version {} does not match config {}",
            manifest.version, entry.version
        ));
    }
    if entry.source == PluginSource::Curated {
        verify_wasm_sha256(wasm_path, &manifest.sha256)?;
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_manifest_capabilities() {
        let raw = r#"{
            "name": "hello",
            "version": "0.1.0",
            "api_version": 1,
            "sha256": "abc",
            "capabilities": ["request_hook", "master_bus", "http_endpoint"],
            "http_endpoint": { "scope": "master" }
        }"#;
        let m = parse_manifest_str(raw).unwrap();
        assert!(m.capabilities.request_hook);
        assert!(m.capabilities.master_bus);
        assert!(m.capabilities.http_endpoint);
        assert!(m.capabilities.http_endpoint_scope_master);
    }
}
