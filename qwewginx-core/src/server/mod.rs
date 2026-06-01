mod error;
mod listen;
mod tls;

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;

pub use error::ServerError;

use crate::config::{Config, Location, ReturnDirective, Server};

pub async fn run(cfg: Config) -> Result<(), ServerError> {
    let conn_builder = auto::Builder::new(TokioExecutor::new());
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
                                async move { Ok::<_, Infallible>(handle(req, &srv)) }
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
                                async move { Ok::<_, Infallible>(handle(req, &srv)) }
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

fn handle(req: Request<Incoming>, server: &Server) -> Response<Full<Bytes>> {
    let path = req.uri().path();
    match match_location(path, &server.locations) {
        Some(ret) => return_response(ret),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("not found\n")))
            .unwrap(),
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

fn match_location<'a>(path: &str, locations: &'a [Location]) -> Option<&'a ReturnDirective> {
    locations
        .iter()
        .filter(|loc| location_prefix_matches(path, &loc.path))
        .max_by_key(|loc| loc.path.len())
        .map(|loc| &loc.ret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ReturnDirective;

    fn loc(path: &str, body: &str) -> Location {
        Location {
            path: path.into(),
            ret: ReturnDirective {
                status: 200,
                body: body.into(),
            },
        }
    }

    fn routing_locations() -> Vec<Location> {
        vec![loc("/", "root\n"), loc("/api", "api\n"), loc("/api/v1", "api v1\n")]
    }

    #[test]
    fn longest_prefix_wins() {
        let locations = routing_locations();
        assert_eq!(match_location("/api/v1/x", &locations).unwrap().body, "api v1\n");
        assert_eq!(match_location("/api/other", &locations).unwrap().body, "api\n");
        assert_eq!(match_location("/", &locations).unwrap().body, "root\n");
        assert_eq!(match_location("/stuff", &locations).unwrap().body, "root\n");
    }

    #[test]
    fn prefix_boundary() {
        let locations = vec![loc("/api", "api\n")];
        assert!(match_location("/apifoo", &locations).is_none());
        assert_eq!(match_location("/api", &locations).unwrap().body, "api\n");
        assert_eq!(match_location("/api/foo", &locations).unwrap().body, "api\n");
    }
}

fn return_response(ret: &ReturnDirective) -> Response<Full<Bytes>> {
    let status =
        StatusCode::from_u16(ret.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::copy_from_slice(ret.body.as_bytes())))
        .unwrap()
}
