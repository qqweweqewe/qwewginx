mod error;

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

pub use error::ServerError;

use crate::config::{Config, Location, ReturnDirective, Server};

pub async fn run(cfg: Config) -> Result<(), ServerError> {
    let mut n = 0;

    for server in &cfg.http.servers {
        let srv = Arc::new(server.clone());
        for addr in &server.listen {
            let listener = TcpListener::bind(*addr).await?;
            tracing::info!("listening on http://{addr}");
            let srv = Arc::clone(&srv);
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
                    let io = TokioIo::new(stream);
                    let srv = Arc::clone(&srv);
                    tokio::spawn(async move {
                        let svc = service_fn(move |req| {
                            let srv = Arc::clone(&srv);
                            async move { Ok::<_, Infallible>(handle(req, &srv)) }
                        });
                        if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                            tracing::debug!("connection closed: {e}");
                        }
                    });
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

fn match_location<'a>(path: &str, locations: &'a [Location]) -> Option<&'a ReturnDirective> {
    // feature 2: longest prefix
    for loc in locations {
        if path.starts_with(&loc.path) {
            return Some(&loc.ret);
        }
    }
    None
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
