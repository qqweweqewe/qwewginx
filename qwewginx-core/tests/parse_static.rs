use std::path::PathBuf;

use qwewginx_core::config::{parse_file, parse_str, LocationAction, StaticFiles};

#[test]
fn parse_static_conf() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/static.conf");
    let cfg = parse_file(&path).expect("parse static.conf");
    match &cfg.http.servers[0].locations[0].action {
        LocationAction::Static(StaticFiles { root, index }) => {
            assert_eq!(root, &PathBuf::from("examples/static"));
            assert_eq!(index, &["index.html"]);
        }
        _ => panic!("expected static root"),
    }
}

#[test]
fn parse_root_with_multiple_index() {
    let src = r#"
events { worker_connections 1024; }
http {
    server {
        listen 127.0.0.1:8080;
        location / {
            root /var/www;
            index index.html index.htm;
        }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    match &cfg.http.servers[0].locations[0].action {
        LocationAction::Static(StaticFiles { root, index }) => {
            assert_eq!(root, &PathBuf::from("/var/www"));
            assert_eq!(index, &["index.html", "index.htm"]);
        }
        _ => panic!("expected static"),
    }
}

#[test]
fn default_index_when_root_only() {
    let src = r#"
events { worker_connections 1024; }
http {
    server {
        listen 127.0.0.1:8080;
        location / {
            root /srv/www;
        }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    match &cfg.http.servers[0].locations[0].action {
        LocationAction::Static(StaticFiles { index, .. }) => {
            assert_eq!(index, &["index.html"]);
        }
        _ => panic!("expected static"),
    }
}
