use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::header::{HeaderValue, HOST};
use hyper::http::Uri;
use hyper::{Method, Request, Version};
use tokio::time;

use crate::config::{HealthCheck, Http, LocationAction, ProxyScheme, ProxyTarget};

use super::proxy::{
    build_upstream_uri, upstream_host, HttpClient, ProbeConfig, UpstreamPool,
    UpstreamTransitionReason, WorkerHttp,
};

pub fn collect_probe_configs(http: &Http) -> HashMap<String, ProbeConfig> {
    let mut out = HashMap::new();
    for server in &http.servers {
        for loc in &server.locations {
            let LocationAction::ProxyPass(pass) = &loc.action else {
                continue;
            };
            let ProxyTarget::Upstream(name) = &pass.target else {
                continue;
            };
            out.entry(name.clone()).or_insert(ProbeConfig {
                scheme: pass.scheme,
                ssl_verify: pass.ssl_verify,
            });
        }
    }
    out
}

pub fn spawn(http: &Http, ctx: Arc<WorkerHttp>) {
    let probe_configs = collect_probe_configs(http);
    for upstream in &http.upstreams {
        let Some(hc) = &upstream.health_check else {
            continue;
        };
        let Some(probe) = probe_configs.get(&upstream.name) else {
            tracing::debug!(
                upstream = %upstream.name,
                "health_check set but upstream not used in proxy_pass — skipping probes"
            );
            continue;
        };
        let Some(pool) = ctx.upstreams.get(&upstream.name) else {
            continue;
        };
        let pool = Arc::clone(pool);
        let client = ctx.client_for_probe(*probe);
        tracing::debug!(
            upstream = %upstream.name,
            interval_secs = hc.interval_secs,
            uri = %hc.uri,
            "active health probe started"
        );
        let hc = hc.clone();
        let name = upstream.name.clone();
        let probe = *probe;
        tokio::spawn(async move {
            probe_loop(&name, pool, client, probe, hc).await;
        });
    }
}

async fn probe_loop(
    upstream_name: &str,
    pool: Arc<UpstreamPool>,
    client: HttpClient,
    probe: ProbeConfig,
    hc: HealthCheck,
) {
    let mut ticker = time::interval(Duration::from_secs(hc.interval_secs as u64));
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        for addr in pool.peer_addrs() {
            if probe_peer(&client, probe.scheme, addr, &hc.uri).await {
                pool.mark_up(
                    upstream_name,
                    addr,
                    UpstreamTransitionReason::HealthProbeOk,
                );
            } else {
                pool.mark_down(
                    upstream_name,
                    addr,
                    UpstreamTransitionReason::HealthProbeFail,
                );
            }
        }
    }
}

async fn probe_peer(
    client: &HttpClient,
    scheme: ProxyScheme,
    addr: SocketAddr,
    path: &str,
) -> bool {
    let path_uri: Uri = match path.parse() {
        Ok(u) => u,
        Err(_) => return false,
    };
    let uri = match build_upstream_uri(scheme, addr, &path_uri) {
        Some(u) => u,
        None => return false,
    };
    let host: HeaderValue = match upstream_host(scheme, addr).parse() {
        Ok(h) => h,
        Err(_) => return false,
    };
    let req = match Request::builder()
        .method(Method::GET)
        .uri(uri)
        .version(Version::HTTP_11)
        .header(HOST, host)
        .body(Full::new(Bytes::new()))
    {
        Ok(r) => r,
        Err(_) => return false,
    };
    match client.request(req).await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}
