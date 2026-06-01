use std::net::SocketAddr;
use std::path::PathBuf;

use qwewginx_core::config::{
    parse_file, LocationAction, ProxyPass, ProxyTarget,
};

#[test]
fn parse_proxy_conf() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/proxy.conf");
    let cfg = parse_file(&path).expect("parse proxy.conf");
    assert_eq!(cfg.http.upstreams.len(), 1);
    let upstream = &cfg.http.upstreams[0];
    assert_eq!(upstream.name, "backend");
    assert_eq!(
        upstream.servers,
        vec!["127.0.0.1:9091".parse::<SocketAddr>().unwrap()]
    );

    let srv = &cfg.http.servers[0];
    assert_eq!(srv.listeners[0].addr.port(), 9090);
    assert_eq!(srv.locations.len(), 1);
    match &srv.locations[0].action {
        LocationAction::ProxyPass(ProxyPass {
            target: ProxyTarget::Upstream(name),
        }) => assert_eq!(name, "backend"),
        _ => panic!("expected proxy_pass to backend"),
    }
}

#[test]
fn parse_direct_proxy_pass() {
    let src = r#"
events { worker_connections 1024; }
http {
    server {
        listen 127.0.0.1:8080;
        location / {
            proxy_pass http://127.0.0.1:9091;
        }
    }
}
"#;
    let cfg = qwewginx_core::config::parse_str(src).expect("parse");
    match &cfg.http.servers[0].locations[0].action {
        LocationAction::ProxyPass(ProxyPass {
            target: ProxyTarget::Direct(addr),
        }) => assert_eq!(*addr, "127.0.0.1:9091".parse().unwrap()),
        _ => panic!("expected direct proxy_pass"),
    }
}
