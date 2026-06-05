use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub worker_processes: u32,
    pub events: Events,
    pub stream: Stream,
    pub http: Http,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stream {
    pub servers: Vec<StreamServer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamServer {
    pub listen: SocketAddr,
    pub proxy_pass: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Events {
    pub worker_connections: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Http {
    pub access_log: Option<AccessLogSetting>,
    pub upstreams: Vec<Upstream>,
    pub servers: Vec<Server>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessLogSetting {
    Off,
    Path(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    pub name: String,
    pub servers: Vec<SocketAddr>,
    pub health_check: Option<HealthCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthCheck {
    pub interval_secs: u32,
    pub uri: String,
}

impl Default for HealthCheck {
    fn default() -> Self {
        Self {
            interval_secs: 5,
            uri: "/".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub listeners: Vec<Listen>,
    pub tls: Option<TlsFiles>,
    /// `None` inherits `http.access_log`.
    pub access_log: Option<AccessLogSetting>,
    pub forward_proxy: bool,
    pub locations: Vec<Location>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listen {
    pub addr: SocketAddr,
    pub ssl: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: String,
    pub action: LocationAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationAction {
    Return(ReturnDirective),
    ProxyPass(ProxyPass),
    Static(StaticFiles),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticFiles {
    pub root: PathBuf,
    pub index: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnDirective {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyScheme {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyPass {
    pub scheme: ProxyScheme,
    pub ssl_verify: bool,
    pub target: ProxyTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyTarget {
    Upstream(String),
    Direct(SocketAddr),
}
