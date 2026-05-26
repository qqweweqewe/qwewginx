use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub worker_processes: u32,
    pub events: Events,
    pub http: Http,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Events {
    pub worker_connections: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Http {
    pub servers: Vec<Server>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub listeners: Vec<Listen>,
    pub tls: Option<TlsFiles>,
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
    pub ret: ReturnDirective,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnDirective {
    pub status: u16,
    pub body: String,
}
