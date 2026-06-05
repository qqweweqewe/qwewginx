use qwewginx_core::config::parse_str;

#[test]
fn parse_health_check_defaults() {
    let src = r#"
events { worker_connections 1024; }
http {
    upstream backend {
        server 127.0.0.1:9091;
        health_check;
    }
    server {
        listen 127.0.0.1:8080;
        location / { proxy_pass http://backend; }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    let hc = cfg.http.upstreams[0].health_check.as_ref().expect("hc");
    assert_eq!(hc.interval_secs, 5);
    assert_eq!(hc.uri, "/");
}

#[test]
fn parse_health_check_options() {
    let src = r#"
events { worker_connections 1024; }
http {
    upstream backend {
        server 127.0.0.1:9091;
        health_check interval 3 uri /health;
    }
    server {
        listen 127.0.0.1:8080;
        location / { proxy_pass http://backend; }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    let hc = cfg.http.upstreams[0].health_check.as_ref().expect("hc");
    assert_eq!(hc.interval_secs, 3);
    assert_eq!(hc.uri, "/health");
}

#[test]
fn inconsistent_ssl_verify_errors() {
    let src = r#"
events { worker_connections 1024; }
http {
    upstream backend { server 127.0.0.1:9441; health_check; }
    server {
        listen 127.0.0.1:8080;
        location /a { proxy_pass https://backend; }
    }
    server {
        listen 127.0.0.1:8081;
        location /b {
            proxy_ssl_verify off;
            proxy_pass https://backend;
        }
    }
}
"#;
    assert!(parse_str(src).is_err());
}
