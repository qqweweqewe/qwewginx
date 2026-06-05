use std::net::SocketAddr;

use qwewginx_core::config::parse_str;

#[test]
fn parse_stream_server() {
    let src = r#"
events { worker_connections 1024; }
stream {
    server {
        listen 127.0.0.1:25565;
        proxy_pass 127.0.0.1:25566;
    }
}
http {
    server {
        listen 127.0.0.1:9090;
        location / { return 200 "ok\n"; }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    assert_eq!(cfg.stream.servers.len(), 1);
    let s = &cfg.stream.servers[0];
    assert_eq!(s.listen, "127.0.0.1:25565".parse::<SocketAddr>().unwrap());
    assert_eq!(s.proxy_pass, "127.0.0.1:25566".parse::<SocketAddr>().unwrap());
}

#[test]
fn parse_stream_only_config() {
    let src = r#"
events { worker_connections 1024; }
stream {
    server {
        listen 127.0.0.1:15432;
        proxy_pass 127.0.0.1:15433;
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    assert!(cfg.http.servers.is_empty());
    assert_eq!(cfg.stream.servers[0].listen.port(), 15432);
}

#[test]
fn stream_server_needs_listen_and_proxy_pass() {
    let src = r#"
events { worker_connections 1024; }
stream {
    server {
        listen 127.0.0.1:25565;
    }
}
"#;
    let err = parse_str(src).unwrap_err().to_string();
    assert!(err.contains("proxy_pass"));
}

#[test]
fn empty_config_errors() {
    let src = r#"
events { worker_connections 1024; }
"#;
    let err = parse_str(src).unwrap_err().to_string();
    assert!(err.contains("need at least one server"));
}
