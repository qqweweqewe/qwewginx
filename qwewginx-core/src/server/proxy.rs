use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use http_body_util::combinators::BoxBody;
use hyper::body::Incoming;
use hyper::header::HOST;
use hyper::http::uri::{Scheme, Uri};
use hyper::{Request, Response, StatusCode};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

use crate::config::{Http, ProxyTarget};

pub type HttpClient = Client<HttpConnector, Incoming>;
pub type ResponseBody = BoxBody<Bytes, hyper::Error>;

pub struct WorkerHttp {
    pub upstreams: HashMap<String, SocketAddr>,
    pub client: HttpClient,
}

impl WorkerHttp {
    pub fn new(http: &Http) -> Self {
        let mut upstreams = HashMap::new();
        for u in &http.upstreams {
            if let Some(addr) = u.servers.first() {
                upstreams.insert(u.name.clone(), *addr);
            }
        }
        let connector = HttpConnector::new();
        let client = Client::builder(TokioExecutor::new()).build(connector);
        Self { upstreams, client }
    }
}

pub fn resolve_upstream(name: &str, upstreams: &HashMap<String, SocketAddr>) -> Option<SocketAddr> {
    upstreams.get(name).copied()
}

pub fn resolve_target(
    target: &ProxyTarget,
    upstreams: &HashMap<String, SocketAddr>,
) -> Option<SocketAddr> {
    match target {
        ProxyTarget::Direct(addr) => Some(*addr),
        ProxyTarget::Upstream(name) => resolve_upstream(name, upstreams),
    }
}

pub fn build_upstream_uri(addr: SocketAddr, req_uri: &Uri) -> Option<Uri> {
    let mut parts = req_uri.clone().into_parts();
    parts.scheme = Some(Scheme::HTTP);
    let authority = if addr.port() == 80 {
        addr.ip().to_string()
    } else {
        addr.to_string()
    };
    parts.authority = Some(authority.parse().ok()?);
    Uri::from_parts(parts).ok()
}

pub async fn forward(
    client: &HttpClient,
    req: Request<Incoming>,
    upstream: SocketAddr,
) -> Response<ResponseBody> {
    let (mut parts, body) = req.into_parts();
    let uri = match build_upstream_uri(upstream, &parts.uri) {
        Some(u) => u,
        None => {
            tracing::debug!("bad upstream uri");
            return bad_gateway();
        }
    };
    parts.uri = uri;
    if !parts.headers.contains_key(HOST) {
        let host = if upstream.port() == 80 {
            upstream.ip().to_string()
        } else {
            upstream.to_string()
        };
        if let Ok(v) = host.parse() {
            parts.headers.insert(HOST, v);
        }
    }
    let upstream_req = Request::from_parts(parts, body);
    match client.request(upstream_req).await {
        Ok(resp) => resp.map(|b| b.boxed()),
        Err(e) => {
            tracing::debug!("proxy failed: {e}");
            bad_gateway()
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

    #[test]
    fn resolve_named_upstream() {
        let mut map = HashMap::new();
        map.insert("backend".into(), "127.0.0.1:9091".parse().unwrap());
        assert_eq!(
            resolve_target(
                &ProxyTarget::Upstream("backend".into()),
                &map
            ),
            Some("127.0.0.1:9091".parse().unwrap())
        );
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
