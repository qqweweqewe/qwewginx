use std::net::SocketAddr;
use std::path::PathBuf;

use qwewginx_core::config::{
    parse_file, parse_str, LocationAction, ProxyPass, ProxyScheme, ProxyTarget,
};

#[test]
fn parse_lb_conf() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/lb.conf");
    let cfg = parse_file(&path).expect("parse lb.conf");
    assert_eq!(cfg.http.upstreams.len(), 1);
    let upstream = &cfg.http.upstreams[0];
    assert_eq!(upstream.name, "backend");
    assert_eq!(
        upstream.servers,
        vec![
            "127.0.0.1:9091".parse::<SocketAddr>().unwrap(),
            "127.0.0.1:9092".parse::<SocketAddr>().unwrap(),
        ]
    );
    match &cfg.http.servers[0].locations[0].action {
        LocationAction::ProxyPass(ProxyPass {
            scheme: ProxyScheme::Http,
            ssl_verify: true,
            target: ProxyTarget::Upstream(name),
        }) => assert_eq!(name, "backend"),
        _ => panic!("expected proxy_pass"),
    }
}

#[test]
fn parse_upstream_three_servers() {
    let src = r#"
events { worker_connections 1024; }
http {
    upstream pool {
        server 127.0.0.1:8001;
        server 127.0.0.1:8002;
        server 127.0.0.1:8003;
    }
    server {
        listen 127.0.0.1:8080;
        location / { proxy_pass http://pool; }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    assert_eq!(cfg.http.upstreams[0].servers.len(), 3);
}
