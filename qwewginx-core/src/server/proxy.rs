use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use http_body_util::combinators::BoxBody;
use hyper::body::Incoming;
use hyper::header::{HeaderValue, HOST};
use hyper::http::uri::Uri;
use hyper::{Request, Response, StatusCode, Version};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

use crate::config::{Http, ProxyTarget};

pub type HttpClient = Client<HttpConnector, Full<Bytes>>;
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

fn hop_by_hop(name: &hyper::header::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection" | "keep-alive" | "proxy-connection" | "te" | "trailer"
            | "transfer-encoding" | "upgrade"
    ) || name.as_str().starts_with(':')
}

pub async fn forward(
    client: &HttpClient,
    req: Request<Incoming>,
    upstream: SocketAddr,
) -> Response<ResponseBody> {
    let (parts, body) = req.into_parts();
    let uri = match build_upstream_uri(upstream, &parts.uri) {
        Some(u) => u,
        None => return bad_gateway(),
    };
    let body_bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            tracing::debug!("proxy body read failed: {e}");
            return bad_gateway();
        }
    };
    let host = if upstream.port() == 80 {
        upstream.ip().to_string()
    } else {
        upstream.to_string()
    };
    let host: HeaderValue = host.parse().expect("host is valid header value");
    let mut builder = Request::builder()
        .method(parts.method)
        .uri(uri)
        .version(Version::HTTP_11)
        .header(HOST, host);
    for (name, value) in parts.headers.iter() {
        if hop_by_hop(name) || *name == HOST {
            continue;
        }
        builder = builder.header(name, value);
    }
    let mut upstream_req = match builder.body(Full::new(body_bytes)) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("proxy request build failed: {e}");
            return bad_gateway();
        }
    };
    *upstream_req.version_mut() = Version::HTTP_11;
    upstream_req.extensions_mut().clear();
    match client.request(upstream_req).await {
        Ok(resp) => resp.map(|b| b.boxed()),
        Err(e) => {
            tracing::debug!("proxy upstream failed: {e}");
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
