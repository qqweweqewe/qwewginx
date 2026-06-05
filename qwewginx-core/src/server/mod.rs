mod access_log;
mod error;
mod health_probe;
mod listen;
mod proxy;
mod static_files;
mod tls;
mod upstream_tls;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;

pub use error::ServerError;

use crate::config::{Config, Location, LocationAction, ProxyTarget, ReturnDirective, Server};
use access_log::{
    content_length_from_headers, resolve_access_log_path, response_body_bytes, write_entry,
    AccessLog, AccessLogEntry,
};
use proxy::{ProxyResult, ResponseBody, UpstreamMeta, WorkerHttp};

struct ServerCtx {
    server: Server,
    access_log: Option<Arc<AccessLog>>,
}

pub async fn run(cfg: Config) -> Result<(), ServerError> {
    let conn_builder = auto::Builder::new(TokioExecutor::new());
    let http_ctx = proxy::worker_http_arc(&cfg.http);
    health_probe::spawn(&cfg.http, Arc::clone(&http_ctx));
    let mut n = 0;

    for server in &cfg.http.servers {
        let access_log = resolve_access_log_path(&cfg.http, server)
            .map(|path| AccessLog::open(&path).map(Arc::new))
            .transpose()
            .map_err(|e| ServerError::Msg(format!("access_log open failed: {e}")))?;
        let srv = Arc::new(ServerCtx {
            server: server.clone(),
            access_log,
        });
        let tls_acceptor = match &server.tls {
            Some(files) => Some(tls::TlsAcceptorHandle::load(&files.cert, &files.key)?),
            None => None,
        };

        for endpoint in &server.listeners {
            let listener = listen::bind_reuseport(endpoint.addr).await?;
            let scheme = if endpoint.ssl { "https" } else { "http" };
            tracing::info!(
                "worker {} listening on {scheme}://{} (http/1.1 + h2)",
                std::process::id(),
                endpoint.addr
            );
            let srv = Arc::clone(&srv);
            let http_ctx = Arc::clone(&http_ctx);
            let conn_builder = conn_builder.clone();
            let tls_acceptor = tls_acceptor.clone();
            let ssl = endpoint.ssl;
            n += 1;

            tokio::spawn(async move {
                loop {
                    let (stream, remote_addr) = match listener.accept().await {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::error!("accept failed: {e}");
                            continue;
                        }
                    };
                    let srv = Arc::clone(&srv);
                    let http_ctx = Arc::clone(&http_ctx);
                    let conn_builder = conn_builder.clone();
                    if ssl {
                        let acceptor = tls_acceptor
                            .as_ref()
                            .expect("ssl listener needs tls config")
                            .clone();
                        tokio::spawn(async move {
                            let Ok(tls_stream) = acceptor.inner.accept(stream).await else {
                                tracing::debug!("tls handshake failed");
                                return;
                            };
                            let io = TokioIo::new(tls_stream);
                            let svc = service_fn(move |req| {
                                let srv = Arc::clone(&srv);
                                let http_ctx = Arc::clone(&http_ctx);
                                async move {
                                    Ok::<_, Infallible>(
                                        handle(req, Some(remote_addr), &srv, &http_ctx).await,
                                    )
                                }
                            });
                            if let Err(e) =
                                conn_builder.serve_connection_with_upgrades(io, svc).await
                            {
                                tracing::debug!("connection closed: {e}");
                            }
                        });
                    } else {
                        let io = TokioIo::new(stream);
                        tokio::spawn(async move {
                            let svc = service_fn(move |req| {
                                let srv = Arc::clone(&srv);
                                let http_ctx = Arc::clone(&http_ctx);
                                async move {
                                    Ok::<_, Infallible>(
                                        handle(req, Some(remote_addr), &srv, &http_ctx).await,
                                    )
                                }
                            });
                            if let Err(e) =
                                conn_builder.serve_connection_with_upgrades(io, svc).await
                            {
                                tracing::debug!("connection closed: {e}");
                            }
                        });
                    }
                }
            });
        }
    }

    if n == 0 {
        return Err(ServerError::Msg("no listen addresses".into()));
    }

    std::future::pending().await
}

struct Handled {
    response: Response<ResponseBody>,
    upstream: UpstreamMeta,
    body_fallback: Option<usize>,
}

async fn handle(
    req: Request<Incoming>,
    remote_addr: Option<SocketAddr>,
    ctx: &ServerCtx,
    http_ctx: &WorkerHttp,
) -> Response<ResponseBody> {
    let started = Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let version = req.version();

    let handled = dispatch(req, &ctx.server, http_ctx).await;

    if let Some(log) = &ctx.access_log {
        let status = handled.response.status().as_u16();
        let body_bytes = response_body_bytes(
            status,
            content_length_from_headers(handled.response.headers()),
            handled.body_fallback,
        );
        write_entry(
            log,
            AccessLogEntry {
                remote_addr,
                method: &method,
                uri: &uri,
                version,
                status,
                body_bytes,
                request_time: started,
                upstream: &handled.upstream,
            },
        );
    }

    handled.response
}

async fn dispatch(req: Request<Incoming>, server: &Server, http_ctx: &WorkerHttp) -> Handled {
    if server.forward_proxy {
        let ProxyResult { response, upstream } =
            proxy::forward_proxy_request(http_ctx, req).await;
        return Handled {
            response,
            upstream,
            body_fallback: None,
        };
    }

    let path = req.uri().path();
    let Some(loc) = match_location(path, &server.locations) else {
        return Handled {
            response: not_found(),
            upstream: UpstreamMeta::default(),
            body_fallback: Some("not found\n".len()),
        };
    };
    match &loc.action {
        LocationAction::Return(ret) => Handled {
            response: return_response(ret),
            upstream: UpstreamMeta::default(),
            body_fallback: Some(ret.body.len()),
        },
        LocationAction::ProxyPass(pass) => match &pass.target {
            ProxyTarget::Upstream(name) => match http_ctx.upstreams.get(name) {
                Some(pool) => {
                    let ProxyResult { response, upstream } =
                        proxy::proxy_upstream(http_ctx, pass, pool.as_ref(), name, req).await;
                    Handled {
                        response,
                        upstream,
                        body_fallback: None,
                    }
                }
                None => {
                    tracing::debug!("unknown upstream for proxy_pass");
                    Handled {
                        response: proxy::bad_gateway_response(),
                        upstream: UpstreamMeta {
                            upstream_name: Some(name.clone()),
                            ..Default::default()
                        },
                        body_fallback: Some("bad gateway\n".len()),
                    }
                }
            },
            ProxyTarget::Direct(addr) => {
                let ProxyResult { response, upstream } =
                    proxy::forward(http_ctx, pass, req, *addr).await;
                Handled {
                    response,
                    upstream,
                    body_fallback: None,
                }
            }
        },
        LocationAction::Static(cfg) => Handled {
            response: static_files::serve(req.method(), path, &loc.path, cfg).await,
            upstream: UpstreamMeta::default(),
            body_fallback: None,
        },
    }
}

fn location_prefix_matches(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return path.starts_with('/');
    }
    path == prefix
        || (path.len() > prefix.len()
            && path.starts_with(prefix)
            && path.as_bytes()[prefix.len()] == b'/')
}

fn match_location<'a>(path: &str, locations: &'a [Location]) -> Option<&'a Location> {
    locations
        .iter()
        .filter(|loc| location_prefix_matches(path, &loc.path))
        .max_by_key(|loc| loc.path.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReturnDirective;

    fn loc(path: &str, body: &str) -> Location {
        Location {
            path: path.into(),
            action: LocationAction::Return(ReturnDirective {
                status: 200,
                body: body.into(),
            }),
        }
    }

    fn routing_locations() -> Vec<Location> {
        vec![loc("/", "root\n"), loc("/api", "api\n"), loc("/api/v1", "api v1\n")]
    }

    fn match_return<'a>(path: &str, locations: &'a [Location]) -> Option<&'a ReturnDirective> {
        match match_location(path, locations).map(|l| &l.action)? {
            LocationAction::Return(r) => Some(r),
            LocationAction::ProxyPass(_) | LocationAction::Static(_) => None,
        }
    }

    #[test]
    fn longest_prefix_wins() {
        let locations = routing_locations();
        assert_eq!(
            match_return("/api/v1/x", &locations).unwrap().body,
            "api v1\n"
        );
        assert_eq!(match_return("/api/other", &locations).unwrap().body, "api\n");
        assert_eq!(match_return("/", &locations).unwrap().body, "root\n");
        assert_eq!(match_return("/stuff", &locations).unwrap().body, "root\n");
    }

    #[test]
    fn prefix_boundary() {
        let locations = vec![loc("/api", "api\n")];
        assert!(match_return("/apifoo", &locations).is_none());
        assert_eq!(match_return("/api", &locations).unwrap().body, "api\n");
        assert_eq!(match_return("/api/foo", &locations).unwrap().body, "api\n");
    }
}

fn return_response(ret: &ReturnDirective) -> Response<ResponseBody> {
    let status =
        StatusCode::from_u16(ret.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(
            Full::new(Bytes::copy_from_slice(ret.body.as_bytes()))
                .map_err(|e: Infallible| match e {})
                .boxed(),
        )
        .unwrap()
}

fn not_found() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(
            Full::new(Bytes::from("not found\n"))
                .map_err(|e: Infallible| match e {})
                .boxed(),
        )
        .unwrap()
}
