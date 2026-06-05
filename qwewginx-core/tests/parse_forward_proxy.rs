use qwewginx_core::config::parse_str;

#[test]
fn parse_forward_proxy_server() {
    let src = r#"
events { worker_connections 1024; }
http {
    server {
        listen 127.0.0.1:3128;
        forward_proxy true;
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    let srv = &cfg.http.servers[0];
    assert!(srv.forward_proxy);
    assert!(srv.locations.is_empty());
    assert_eq!(srv.listeners[0].addr.port(), 3128);
}

#[test]
fn forward_proxy_rejects_locations() {
    let src = r#"
events { worker_connections 1024; }
http {
    server {
        listen 127.0.0.1:3128;
        forward_proxy true;
        location / { return 200 "nope\n"; }
    }
}
"#;
    let err = parse_str(src).unwrap_err().to_string();
    assert!(err.contains("forward_proxy server cannot have location"));
}

#[test]
fn forward_proxy_false_by_default() {
    let src = r#"
events { worker_connections 1024; }
http {
    server {
        listen 127.0.0.1:8080;
        location / { return 200 "ok\n"; }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    assert!(!cfg.http.servers[0].forward_proxy);
}

#[test]
fn forward_proxy_must_be_true_or_false() {
    let src = r#"
events { worker_connections 1024; }
http {
    server {
        listen 127.0.0.1:3128;
        forward_proxy on;
    }
}
"#;
    let err = parse_str(src).unwrap_err().to_string();
    assert!(err.contains("forward_proxy must be true or false"));
}
