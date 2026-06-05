use qwewginx_core::config::{parse_str, AccessLogSetting};

#[test]
fn parse_access_log_http_default() {
    let src = r#"
events { worker_connections 1024; }
http {
    access_log /var/log/qwewginx/access.log;
    server {
        listen 127.0.0.1:8080;
        location / { return 200 "ok\n"; }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    assert_eq!(
        cfg.http.access_log,
        Some(AccessLogSetting::Path("/var/log/qwewginx/access.log".into()))
    );
    assert!(cfg.http.servers[0].access_log.is_none());
}

#[test]
fn parse_access_log_server_off_overrides_http() {
    let src = r#"
events { worker_connections 1024; }
http {
    access_log /var/log/http.log;
    server {
        listen 127.0.0.1:8080;
        access_log off;
        location / { return 200 "ok\n"; }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    assert_eq!(
        cfg.http.servers[0].access_log,
        Some(AccessLogSetting::Off)
    );
}

#[test]
fn parse_access_log_off_at_http() {
    let src = r#"
events { worker_connections 1024; }
http {
    access_log off;
    server {
        listen 127.0.0.1:8080;
        location / { return 200 "ok\n"; }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    assert_eq!(cfg.http.access_log, Some(AccessLogSetting::Off));
}

#[test]
fn parse_access_log_invalid_args() {
    let src = r#"
events { worker_connections 1024; }
http {
    access_log off /extra;
    server { listen 127.0.0.1:8080; location / { return 200 "x\n"; } }
}
"#;
    let err = parse_str(src).unwrap_err().to_string();
    assert!(err.contains("access_log off takes no extra args"));
}
