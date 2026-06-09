use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing::{info, warn};
use wasmtime::component::{bindgen, Component, Linker};
use wasmtime::{Config as EngineConfig, Engine, Store};

use super::config_json::{directive_arg, entry_config_json};
use super::host::PluginManifest;
use super::manifest::load_and_verify;
use crate::config::{PluginEntry, Plugins};

bindgen!({
    path: "wit/qwewginx-plugin.wit",
    world: "plugin",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginRole {
    Master,
    Worker,
}

pub struct HostState {
    pub plugin_name: String,
    pub config_json: String,
    pub role: PluginRole,
    pub bus_events: Vec<String>,
}

impl qwewginx::plugin::host::Host for HostState {
    fn log_info(&mut self, msg: String) {
        info!(plugin = %self.plugin_name, "{msg}");
    }

    fn log_warn(&mut self, msg: String) {
        warn!(plugin = %self.plugin_name, "{msg}");
    }

    fn bus_emit(&mut self, event: String) {
        if self.role == PluginRole::Master {
            tracing::debug!(plugin = %self.plugin_name, bus = %event, "master bus (local)");
        }
        self.bus_events.push(event);
    }

    fn get_config_json(&mut self) -> String {
        self.config_json.clone()
    }
}

struct PluginInstance {
    manifest: PluginManifest,
    entry: PluginEntry,
    store: Store<HostState>,
    bindings: Plugin,
}

impl PluginInstance {
    fn load(
        engine: &Engine,
        linker: &Linker<HostState>,
        entry: &PluginEntry,
        manifest: PluginManifest,
        wasm_path: &Path,
        role: PluginRole,
    ) -> Result<Self, String> {
        let component = Component::from_file(engine, wasm_path)
            .map_err(|e| format!("load wasm {}: {e}", wasm_path.display()))?;
        let mut store = Store::new(
            engine,
            HostState {
                plugin_name: entry.name.clone(),
                config_json: entry_config_json(entry),
                role,
                bus_events: Vec::new(),
            },
        );
        let bindings = Plugin::instantiate(&mut store, &component, linker)
            .map_err(|e| format!("instantiate {}@{}: {e}", entry.name, entry.version))?;
        Ok(Self {
            manifest,
            entry: entry.clone(),
            store,
            bindings,
        })
    }

    fn validate_config(&mut self) -> Result<(), String> {
        self.bindings
            .call_validate_config(&mut self.store)
            .map_err(|e| format!("validate_config {}: {e}", self.entry.name))?
            .map_err(|msg| format!("validate_config {}: {msg}", self.entry.name))
    }

    fn on_master_init(&mut self) -> Result<(), String> {
        self.bindings
            .call_on_master_init(&mut self.store)
            .map_err(|e| format!("on_master_init {}: {e}", self.entry.name))
    }

    fn on_worker_init(&mut self) -> Result<(), String> {
        self.bindings
            .call_on_worker_init(&mut self.store)
            .map_err(|e| format!("on_worker_init {}: {e}", self.entry.name))
    }

    fn on_request_complete(&mut self, payload_json: &str) -> Result<(), String> {
        if !self.manifest.capabilities.request_hook {
            return Ok(());
        }
        self.bindings
            .call_on_request_complete(&mut self.store, payload_json)
            .map_err(|e| format!("on_request_complete {}: {e}", self.entry.name))
    }

    fn handle_http(&mut self, method: &str, path: &str, body: &str) -> Result<(u16, String), String> {
        self.bindings
            .call_handle_http(&mut self.store, method, path, body)
            .map_err(|e| format!("handle_http {}: {e}", self.entry.name))
    }

    fn http_listen(&self) -> Option<SocketAddr> {
        if !self.manifest.capabilities.http_endpoint {
            return None;
        }
        let listen = directive_arg(&self.entry, "listen")?;
        parse_listen_addr(listen).ok()
    }

    fn http_path(&self) -> Option<String> {
        if !self.manifest.capabilities.http_endpoint {
            return None;
        }
        directive_arg(&self.entry, "path").map(|s| s.to_string())
    }

    fn http_scope_master(&self) -> bool {
        self.manifest.capabilities.http_endpoint_scope_master
    }
}

fn parse_listen_addr(s: &str) -> Result<SocketAddr, String> {
    let addr = if s.starts_with(':') {
        format!("127.0.0.1{s}")
    } else {
        s.to_string()
    };
    addr.parse().map_err(|_| format!("bad listen address: {s}"))
}

fn build_engine() -> Result<Engine, String> {
    let mut cfg = EngineConfig::new();
    cfg.wasm_component_model(true);
    Engine::new(&cfg).map_err(|e| format!("wasm engine: {e}"))
}

fn build_linker(engine: &Engine) -> Result<Linker<HostState>, String> {
    let mut linker = Linker::new(engine);
    Plugin::add_to_linker(&mut linker, |s| s)
        .map_err(|e| format!("plugin linker: {e}"))?;
    Ok(linker)
}

fn load_instances(
    plugins: &Plugins,
    cache_root: &Path,
    role: PluginRole,
) -> Result<Vec<PluginInstance>, String> {
    if plugins.entries.is_empty() {
        return Ok(Vec::new());
    }
    let engine = build_engine()?;
    let linker = build_linker(&engine)?;
    let mut instances = Vec::new();
    for entry in &plugins.entries {
        let dir = cache_root.join(&entry.name).join(&entry.version);
        let manifest_path = dir.join("manifest.json");
        let wasm_path = dir.join("plugin.wasm");
        let manifest = load_and_verify(entry, &manifest_path, &wasm_path)?;
        instances.push(PluginInstance::load(
            &engine, &linker, entry, manifest, &wasm_path, role,
        )?);
    }
    Ok(instances)
}

#[derive(Debug, Clone)]
pub struct MasterHttpRoute {
    pub listen: SocketAddr,
    pub path: String,
    pub plugin_name: String,
    pub plugin_index: usize,
}

pub struct PluginMaster {
    instances: Vec<PluginInstance>,
    routes: Vec<MasterHttpRoute>,
}

impl PluginMaster {
    pub fn load(plugins: &Plugins, cache_root: impl AsRef<Path>) -> Result<Self, String> {
        Ok(Self {
            instances: load_instances(plugins, cache_root.as_ref(), PluginRole::Master)?,
            routes: Vec::new(),
        })
    }

    pub fn validate_and_init(&mut self) -> Result<(), String> {
        for inst in &mut self.instances {
            inst.validate_config()?;
        }
        for inst in &mut self.instances {
            inst.on_master_init()?;
        }
        self.routes.clear();
        for (idx, inst) in self.instances.iter().enumerate() {
            if !inst.manifest.capabilities.http_endpoint || !inst.http_scope_master() {
                continue;
            }
            let listen = inst
                .http_listen()
                .ok_or_else(|| format!("plugin {} needs listen", inst.entry.name))?;
            let path = inst
                .http_path()
                .ok_or_else(|| format!("plugin {} needs path", inst.entry.name))?;
            self.routes.push(MasterHttpRoute {
                listen,
                path,
                plugin_name: inst.entry.name.clone(),
                plugin_index: idx,
            });
        }
        Ok(())
    }

    pub fn master_http_routes(&self) -> &[MasterHttpRoute] {
        &self.routes
    }

    pub fn handle_master_http(
        &mut self,
        plugin_index: usize,
        method: &str,
        path: &str,
        body: &str,
    ) -> Result<(u16, String), String> {
        let inst = self
            .instances
            .get_mut(plugin_index)
            .ok_or_else(|| format!("plugin index {plugin_index} out of range"))?;
        inst.handle_http(method, path, body)
    }
}

pub struct PluginWorkerRuntime {
    inner: Mutex<Vec<PluginInstance>>,
}

impl PluginWorkerRuntime {
    pub fn load(plugins: &Plugins, cache_root: impl AsRef<Path>) -> Result<Self, String> {
        Ok(Self {
            inner: Mutex::new(load_instances(
                plugins,
                cache_root.as_ref(),
                PluginRole::Worker,
            )?),
        })
    }

    pub fn worker_init(&self) -> Result<(), String> {
        let mut plugins = self.inner.lock().map_err(|e| e.to_string())?;
        for inst in plugins.iter_mut() {
            inst.on_worker_init()?;
        }
        Ok(())
    }

    pub fn on_request_complete(&self, payload_json: &str) {
        let Ok(mut plugins) = self.inner.lock() else {
            return;
        };
        for inst in plugins.iter_mut() {
            if let Err(e) = inst.on_request_complete(payload_json) {
                warn!(plugin = %inst.manifest.name, "{e}");
            }
        }
    }
}

pub fn plugin_cache_root() -> PathBuf {
    std::env::var("QWEWNGINX_PLUGIN_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/var/lib/qwewginx/plugins"))
}

pub type SharedPluginMaster = Arc<Mutex<PluginMaster>>;
pub type SharedPluginWorker = Arc<PluginWorkerRuntime>;
