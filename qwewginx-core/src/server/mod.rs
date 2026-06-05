mod error;
mod listen;
mod proxy;
mod static_files;
mod tls;
mod upstream_tls;

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;

pub use error::ServerError;

use crate::config::{Config, Location, LocationAction, ProxyTarget, ReturnDirective, Server};
use proxy::{ResponseBody, WorkerHttp};

pub async fn run(cfg: Config) -> Result<(), ServerError> {
    let conn_builder = auto::Builder::new(TokioExecutor::new());
    let http_ctx = proxy::worker_http_arc(&cfg.http);
    let mut n = 0;

    for server in &cfg.http.servers {
        let srv = Arc::new(server.clone());
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
                    let (stream, _) = match listener.accept().await {
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
                                async move { Ok::<_, Infallible>(handle(req, &srv, &http_ctx).await) }
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
                                async move { Ok::<_, Infallible>(handle(req, &srv, &http_ctx).await) }
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

async fn handle(
    req: Request<Incoming>,
    server: &Server,
    http_ctx: &WorkerHttp,
) -> Response<ResponseBody> {
    let path = req.uri().path();
    let Some(loc) = match_location(path, &server.locations) else {
        return not_found();
    };
    match &loc.action {
        LocationAction::Return(ret) => return_response(ret),
        LocationAction::ProxyPass(pass) => match &pass.target {
            ProxyTarget::Upstream(name) => match http_ctx.upstreams.get(name) {
                Some(pool) => proxy::proxy_upstream(http_ctx, pass, pool, req).await,
                None => {
                    tracing::debug!("unknown upstream for proxy_pass");
                    proxy::bad_gateway_response()
                }
            },
            ProxyTarget::Direct(addr) => proxy::forward(http_ctx, pass, req, *addr).await,
        },
        LocationAction::Static(cfg) => {
            static_files::serve(req.method(), path, &loc.path, cfg).await
        }
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
