use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, HOST};
use hyper::http::uri::Uri;
use hyper::{Method, Request, Response, StatusCode, Version};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

use crate::config::Http;

pub type HttpClient = Client<HttpConnector, Full<Bytes>>;
pub type ResponseBody = BoxBody<Bytes, hyper::Error>;

/// how long a peer stays out of rotation after a passive failure
pub const FAIL_TIMEOUT: Duration = Duration::from_secs(10);

struct Peer {
    addr: SocketAddr,
    /// epoch ms when peer is healthy again; 0 = up
    down_until_ms: AtomicU64,
}

impl Peer {
    fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            down_until_ms: AtomicU64::new(0),
        }
    }

    fn is_healthy(&self) -> bool {
        let until = self.down_until_ms.load(Ordering::Relaxed);
        until == 0 || now_ms() >= until
    }

    fn mark_down(&self) {
        let until = now_ms().saturating_add(ms_from_duration(FAIL_TIMEOUT));
        self.down_until_ms.store(until, Ordering::Relaxed);
        tracing::debug!(addr = %self.addr, "upstream peer marked down");
    }

    fn mark_up(&self) {
        self.down_until_ms.store(0, Ordering::Relaxed);
    }
}

pub struct UpstreamPool {
    peers: Vec<Peer>,
    next: AtomicUsize,
}

impl UpstreamPool {
    pub fn new(servers: Vec<SocketAddr>) -> Self {
        Self {
            peers: servers.into_iter().map(Peer::new).collect(),
            next: AtomicUsize::new(0),
        }
    }

    /// round-robin among healthy peers only
    pub fn pick(&self) -> Option<SocketAddr> {
        let n = self.peers.len();
        if n == 0 {
            return None;
        }
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        for off in 0..n {
            let peer = &self.peers[(start + off) % n];
            if peer.is_healthy() {
                return Some(peer.addr);
            }
        }
        None
    }

    fn mark_down(&self, addr: SocketAddr) {
        if let Some(peer) = self.peers.iter().find(|p| p.addr == addr) {
            peer.mark_down();
        }
    }

    fn mark_up(&self, addr: SocketAddr) {
        if let Some(peer) = self.peers.iter().find(|p| p.addr == addr) {
            peer.mark_up();
        }
    }

    fn len(&self) -> usize {
        self.peers.len()
    }
}

pub struct WorkerHttp {
    pub upstreams: HashMap<String, UpstreamPool>,
    pub client: HttpClient,
}

impl WorkerHttp {
    pub fn new(http: &Http) -> Self {
        let mut upstreams = HashMap::new();
        for u in &http.upstreams {
            if !u.servers.is_empty() {
                upstreams.insert(u.name.clone(), UpstreamPool::new(u.servers.clone()));
            }
        }
        let connector = HttpConnector::new();
        let client = Client::builder(TokioExecutor::new()).build(connector);
        Self { upstreams, client }
    }
}

#[cfg(test)]
fn resolve_target(
    target: &ProxyTarget,
    upstreams: &HashMap<String, UpstreamPool>,
) -> Option<SocketAddr> {
    match target {
        ProxyTarget::Direct(addr) => Some(*addr),
        ProxyTarget::Upstream(name) => upstreams.get(name).and_then(UpstreamPool::pick),
    }
}

pub fn build_upstream_uri(addr: SocketAddr, req_uri: &Uri) -> Option<Uri> {
    let pq = req_uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("/");
    let authority = if addr.port() == 80 {
        addr.ip().to_string()
    } else {
        addr.to_string()
    };
    format!("http://{authority}{pq}").parse().ok()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn ms_from_duration(d: Duration) -> u64 {
    d.as_millis() as u64
}

fn upstream_host(addr: SocketAddr) -> String {
    if addr.port() == 80 {
        addr.ip().to_string()
    } else {
        addr.to_string()
    }
}

fn hop_by_hop(name: &hyper::header::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection" | "keep-alive" | "proxy-connection" | "te" | "trailer"
            | "transfer-encoding" | "upgrade"
    ) || name.as_str().starts_with(':')
}

fn peer_failure_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::BAD_GATEWAY | StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT
    )
}

struct BufferedRequest {
    method: Method,
    uri: Uri,
    headers: hyper::HeaderMap,
    body: Bytes,
}

enum ForwardOutcome {
    Ok(Response<ResponseBody>),
    PeerFailed,
}

pub async fn proxy_upstream(
    client: &HttpClient,
    pool: &UpstreamPool,
    req: Request<Incoming>,
) -> Response<ResponseBody> {
    let buffered = match buffer_request(req).await {
        Some(b) => b,
        None => return bad_gateway(),
    };

    let attempts = pool.len().max(1);
    for attempt in 0..attempts {
        let Some(upstream) = pool.pick() else {
            tracing::debug!("no healthy upstream peers");
            return bad_gateway();
        };
        match forward_buffered(client, &buffered, upstream).await {
            ForwardOutcome::Ok(resp) => {
                pool.mark_up(upstream);
                return resp;
            }
            ForwardOutcome::PeerFailed => {
                pool.mark_down(upstream);
                tracing::debug!(%upstream, attempt, "retrying next upstream peer");
            }
        }
    }
    tracing::debug!("upstream pool exhausted retries");
    bad_gateway()
}

pub async fn forward(
    client: &HttpClient,
    req: Request<Incoming>,
    upstream: SocketAddr,
) -> Response<ResponseBody> {
    let buffered = match buffer_request(req).await {
        Some(b) => b,
        None => return bad_gateway(),
    };
    match forward_buffered(client, &buffered, upstream).await {
        ForwardOutcome::Ok(resp) => resp,
        ForwardOutcome::PeerFailed => bad_gateway(),
    }
}

async fn buffer_request(req: Request<Incoming>) -> Option<BufferedRequest> {
    let (parts, body) = req.into_parts();
    let body = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            tracing::debug!("proxy body read failed: {e}");
            return None;
        }
    };
    Some(BufferedRequest {
        method: parts.method,
        uri: parts.uri,
        headers: parts.headers,
        body,
    })
}

async fn forward_buffered(
    client: &HttpClient,
    buffered: &BufferedRequest,
    upstream: SocketAddr,
) -> ForwardOutcome {
    let uri = match build_upstream_uri(upstream, &buffered.uri) {
        Some(u) => u,
        None => return ForwardOutcome::PeerFailed,
    };
    let host: HeaderValue = upstream_host(upstream)
        .parse()
        .expect("host is valid header value");
    let mut builder = Request::builder()
        .method(&buffered.method)
        .uri(uri)
        .version(Version::HTTP_11)
        .header(HOST, host);
    for (name, value) in buffered.headers.iter() {
        if hop_by_hop(name) || *name == HOST {
            continue;
        }
        builder = builder.header(name, value);
    }
    let mut upstream_req = match builder.body(Full::new(buffered.body.clone())) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("proxy request build failed: {e}");
            return ForwardOutcome::PeerFailed;
        }
    };
    *upstream_req.version_mut() = Version::HTTP_11;
    upstream_req.extensions_mut().clear();
    match client.request(upstream_req).await {
        Ok(resp) => {
            if peer_failure_status(resp.status()) {
                ForwardOutcome::PeerFailed
            } else {
                ForwardOutcome::Ok(resp.map(|b| b.boxed()))
            }
        }
        Err(e) => {
            tracing::debug!("proxy upstream failed: {e}");
            ForwardOutcome::PeerFailed
        }
    }
}

fn bad_gateway() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .header("content-type", "text/plain")
        .body(
            Full::new(Bytes::from("bad gateway\n"))
                .map_err(|e: Infallible| match e {})
                .boxed(),
        )
        .unwrap()
}

pub fn worker_http_arc(http: &Http) -> Arc<WorkerHttp> {
    Arc::new(WorkerHttp::new(http))
}

pub fn bad_gateway_response() -> Response<ResponseBody> {
    bad_gateway()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProxyTarget;

    impl UpstreamPool {
        fn force_down(&self, addr: SocketAddr, timeout: Duration) {
            let peer = self.peers.iter().find(|p| p.addr == addr).expect("peer");
            let until = now_ms().saturating_add(ms_from_duration(timeout));
            peer.down_until_ms.store(until, Ordering::Relaxed);
        }

        fn force_recovered(&self, addr: SocketAddr) {
            let peer = self.peers.iter().find(|p| p.addr == addr).expect("peer");
            peer.down_until_ms
                .store(now_ms().saturating_sub(1), Ordering::Relaxed);
        }
    }

    #[test]
    fn resolve_named_upstream() {
        let mut map = HashMap::new();
        map.insert(
            "backend".into(),
            UpstreamPool::new(vec!["127.0.0.1:9091".parse().unwrap()]),
        );
        assert_eq!(
            resolve_target(
                &ProxyTarget::Upstream("backend".into()),
                &map
            ),
            Some("127.0.0.1:9091".parse().unwrap())
        );
    }

    #[test]
    fn round_robin_cycles_peers() {
        let a: SocketAddr = "127.0.0.1:9091".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:9092".parse().unwrap();
        let c: SocketAddr = "127.0.0.1:9093".parse().unwrap();
        let pool = UpstreamPool::new(vec![a, b, c]);
        let picks: Vec<_> = (0..6).map(|_| pool.pick().unwrap()).collect();
        assert_eq!(picks, vec![a, b, c, a, b, c]);
    }

    #[test]
    fn pick_skips_down_peer() {
        let a: SocketAddr = "127.0.0.1:9091".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:9092".parse().unwrap();
        let pool = UpstreamPool::new(vec![a, b]);
        pool.force_down(a, FAIL_TIMEOUT);
        for _ in 0..4 {
            assert_eq!(pool.pick().unwrap(), b);
        }
    }

    #[test]
    fn peer_recovers_after_cooldown() {
        let a: SocketAddr = "127.0.0.1:9091".parse().unwrap();
        let b: SocketAddr = "127.0.0.1:9092".parse().unwrap();
        let pool = UpstreamPool::new(vec![a, b]);
        pool.force_down(a, FAIL_TIMEOUT);
        pool.force_recovered(a);
        let picks: Vec<_> = (0..2).map(|_| pool.pick().unwrap()).collect();
        assert_eq!(picks, vec![a, b]);
    }

    #[test]
    fn pick_none_when_all_down() {
        let a: SocketAddr = "127.0.0.1:9091".parse().unwrap();
        let pool = UpstreamPool::new(vec![a]);
        pool.force_down(a, FAIL_TIMEOUT);
        assert!(pool.pick().is_none());
    }

    #[test]
    fn peer_failure_statuses() {
        assert!(peer_failure_status(StatusCode::BAD_GATEWAY));
        assert!(peer_failure_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(peer_failure_status(StatusCode::GATEWAY_TIMEOUT));
        assert!(!peer_failure_status(StatusCode::OK));
        assert!(!peer_failure_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!peer_failure_status(StatusCode::NOT_FOUND));
    }

    #[test]
    fn build_uri_from_path_only() {
        let addr: SocketAddr = "127.0.0.1:9091".parse().unwrap();
        let uri: Uri = "/".parse().unwrap();
        let out = build_upstream_uri(addr, &uri).expect("uri");
        assert_eq!(out.to_string(), "http://127.0.0.1:9091/");
    }

    #[test]
    fn build_uri_from_absolute_https() {
        let addr: SocketAddr = "127.0.0.1:9091".parse().unwrap();
        let uri: Uri = "https://127.0.0.1:9443/".parse().unwrap();
        let out = build_upstream_uri(addr, &uri).expect("uri");
        assert_eq!(out.path(), "/");
        assert_eq!(out.authority().unwrap().host(), "127.0.0.1");
        assert_eq!(out.authority().unwrap().port_u16(), Some(9091));
    }

    #[test]
    fn build_uri_keeps_path() {
        let addr: SocketAddr = "127.0.0.1:9091".parse().unwrap();
        let uri: Uri = "http://proxy.local/api?q=1".parse().unwrap();
        let out = build_upstream_uri(addr, &uri).expect("uri");
        assert_eq!(out.path(), "/api");
        assert_eq!(out.query(), Some("q=1"));
        assert_eq!(out.scheme_str(), Some("http"));
    }
}
