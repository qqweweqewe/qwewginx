mod error;

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

pub use error::ServerError;

use crate::config::{Config, Location, ReturnDirective, Server};

pub async fn run(cfg: Config) -> Result<(), ServerError> {
    let (shutdown_tx, _) = broadcast::channel(1);
    let active = Arc::new(AtomicUsize::new(0));
    let mut accept_handles = Vec::new();
    let mut n = 0;

    for server in &cfg.http.servers {
        let srv = Arc::new(server.clone());
        for addr in &server.listen {
            let listener = TcpListener::bind(*addr).await?;
            tracing::info!("listening on http://{addr}");
            let srv = Arc::clone(&srv);
            let mut shutdown_rx = shutdown_tx.subscribe();
            let active = Arc::clone(&active);
            n += 1;

            accept_handles.push(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = shutdown_rx.recv() => break,
                        accept = listener.accept() => {
                            let (stream, _) = match accept {
                                Ok(v) => v,
                                Err(e) => {
                                    tracing::error!("accept failed: {e}");
                                    continue;
                                }
                            };
                            active.fetch_add(1, Ordering::SeqCst);
                            let io = TokioIo::new(stream);
                            let srv = Arc::clone(&srv);
                            let active = Arc::clone(&active);
                            tokio::spawn(async move {
                                let _guard = ConnectionGuard(active);
                                let svc = service_fn(move |req| {
                                    let srv = Arc::clone(&srv);
                                    async move { Ok::<_, Infallible>(handle(req, &srv)) }
                                });
                                if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                                    tracing::debug!("connection closed: {e}");
                                }
                            });
                        }
                    }
                }
            }));
        }
    }

    if n == 0 {
        return Err(ServerError::Msg("no listen addresses".into()));
    }

    wait_shutdown_signal().await;
    let _ = shutdown_tx.send(());

    for handle in accept_handles {
        let _ = handle.await;
    }

    drain_connections(&active).await;
    tracing::info!("shutdown complete");
    Ok(())
}

struct ConnectionGuard(Arc<AtomicUsize>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn wait_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut term =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl-c handler");
    }
    tracing::info!("shutdown signal received");
}

async fn drain_connections(active: &AtomicUsize) {
    while active.load(Ordering::SeqCst) > 0 {
        tokio::task::yield_now().await;
    }
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

pub(crate) fn match_location<'a>(
    path: &str,
    locations: &'a [Location],
) -> Option<&'a ReturnDirective> {
    let root_is_exact = locations.len() > 1;
    locations
        .iter()
        .filter(|loc| path_matches_prefix(path, &loc.path, root_is_exact))
        .max_by_key(|loc| loc.path.len())
        .map(|loc| &loc.ret)
}

fn path_matches_prefix(path: &str, prefix: &str, root_is_exact: bool) -> bool {
    if !path.starts_with(prefix) {
        return false;
    }
    if path.len() == prefix.len() {
        return true;
    }
    // sole "location /" still catches any path; with siblings it's exact "/" only
    if prefix == "/" {
        return !root_is_exact;
    }
    path.as_bytes()[prefix.len()] == b'/'
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Location;

    fn loc(path: &str, body: &str) -> Location {
        Location {
            path: path.to_string(),
            ret: ReturnDirective {
                status: 200,
                body: body.to_string(),
            },
        }
    }

    #[test]
    fn longest_prefix_wins() {
        let locations = vec![
            loc("/", "root"),
            loc("/api", "api"),
            loc("/api/v1", "api v1"),
        ];
        assert_eq!(match_location("/", &locations).unwrap().body, "root");
        assert_eq!(match_location("/api", &locations).unwrap().body, "api");
        assert_eq!(match_location("/api/foo", &locations).unwrap().body, "api");
        assert_eq!(
            match_location("/api/v1/x", &locations).unwrap().body,
            "api v1"
        );
    }

    #[test]
    fn no_match() {
        let locations = vec![loc("/only", "only")];
        assert!(match_location("/other", &locations).is_none());
    }

    #[test]
    fn unknown_path_404_with_routing_locations() {
        let locations = vec![
            loc("/", "root"),
            loc("/api", "api"),
            loc("/api/v1", "api v1"),
        ];
        assert_eq!(match_location("/", &locations).unwrap().body, "root");
        assert!(match_location("/unknown", &locations).is_none());
        assert!(match_location("/apifoo", &locations).is_none());
    }

    #[test]
    fn sole_root_location_is_catch_all() {
        let locations = vec![loc("/", "root")];
        assert_eq!(match_location("/anything", &locations).unwrap().body, "root");
    }
}
