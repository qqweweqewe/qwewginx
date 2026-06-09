use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use tokio::net::TcpListener;
use tracing::info;

use super::runtime::{MasterHttpRoute, SharedPluginMaster};

pub async fn spawn_master_http(
    routes: Vec<MasterHttpRoute>,
    master: SharedPluginMaster,
) -> Result<(), String> {
    for route in routes {
        let listener = TcpListener::bind(route.listen)
            .await
            .map_err(|e| format!("plugin http bind {}: {e}", route.listen))?;
        info!(
            "master plugin http {} on http://{}{}",
            route.plugin_name, route.listen, route.path
        );
        let master = Arc::clone(&master);
        let path_prefix = route.path.clone();
        let plugin_index = route.plugin_index;
        tokio::spawn(async move {
            let conn_builder = auto::Builder::new(TokioExecutor::new());
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let io = TokioIo::new(stream);
                let master = Arc::clone(&master);
                let path_prefix = path_prefix.clone();
                let svc = service_fn(move |req: Request<Incoming>| {
                    let master = Arc::clone(&master);
                    let path_prefix = path_prefix.clone();
                    async move {
                        Ok::<_, Infallible>(dispatch_plugin_http(
                            req,
                            &master,
                            plugin_index,
                            &path_prefix,
                        )
                        .await)
                    }
                });
                let conn_builder = conn_builder.clone();
                tokio::spawn(async move {
                    let _ = conn_builder.serve_connection(io, svc).await;
                });
            }
        });
    }
    Ok(())
}

async fn dispatch_plugin_http(
    req: Request<Incoming>,
    master: &SharedPluginMaster,
    plugin_index: usize,
    path_prefix: &str,
) -> Response<Full<Bytes>> {
    if req.uri().path() != path_prefix {
        return text_response(StatusCode::NOT_FOUND, "not found\n");
    }
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let body = String::new();
    let Ok(mut guard) = master.lock() else {
        return text_response(StatusCode::INTERNAL_SERVER_ERROR, "plugin lock poisoned\n");
    };
    match guard.handle_master_http(plugin_index, &method, &path, &body) {
        Ok((status, body)) => {
            let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            text_response(code, &body)
        }
        Err(e) => {
            tracing::warn!("plugin handle_http: {e}");
            text_response(StatusCode::INTERNAL_SERVER_ERROR, "plugin error\n")
        }
    }
}

fn text_response(status: StatusCode, body: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}
