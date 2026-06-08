use qwewginx_core::config::{parse_file, parse_str, PluginSource};

#[test]
fn parse_plugins_block_from_example_conf() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/plugins.conf");
    let cfg = parse_file(&path)
    .expect("parse plugins.conf");

    assert_eq!(
        cfg.plugins.registry.as_deref(),
        Some("https://plugins.example.com")
    );
    assert_eq!(cfg.plugins.entries.len(), 2);

    let hello = &cfg.plugins.entries[0];
    assert_eq!(hello.name, "hello");
    assert_eq!(hello.version, "0.1.0");
    assert_eq!(hello.source, PluginSource::Curated);
    assert_eq!(hello.directives.len(), 2);
    assert_eq!(hello.directives[0].name, "listen");
    assert_eq!(hello.directives[0].args, ["127.0.0.1:9098"]);
    assert_eq!(hello.directives[1].name, "path");
    assert_eq!(hello.directives[1].args, ["/hello"]);

    let local = &cfg.plugins.entries[1];
    assert_eq!(local.name, "my-tool");
    assert_eq!(local.version, "0.0.1");
    assert_eq!(local.source, PluginSource::Local);
    assert_eq!(local.directives[0].name, "listen");
}

#[test]
fn parse_plugins_inline_minimal() {
    let src = r#"
worker_processes 1;
plugins {
    registry https://registry.test;
    demo 1.2.3 {
        path /demo;
    }
}
events { worker_connections 1024; }
http {
    server {
        listen 127.0.0.1:9090;
        location / { return 200 "ok\n"; }
    }
}
"#;
    let cfg = parse_str(src).expect("parse");
    assert_eq!(cfg.plugins.entries.len(), 1);
    assert_eq!(cfg.plugins.entries[0].name, "demo");
    assert_eq!(cfg.plugins.entries[0].version, "1.2.3");
    assert_eq!(cfg.plugins.entries[0].source, PluginSource::Curated);
}

#[test]
fn plugin_entry_requires_version() {
    let src = r#"
events { worker_connections 1024; }
plugins {
    broken {
        listen 127.0.0.1:1;
    }
}
stream {
    server {
        listen 127.0.0.1:1;
        proxy_pass 127.0.0.1:2;
    }
}
"#;
    let err = parse_str(src).unwrap_err().to_string();
    assert!(err.contains("needs a version"));
}

#[test]
fn plugins_block_rejects_unknown_directive() {
    let src = r#"
events { worker_connections 1024; }
plugins {
    worker_processes 2;
}
stream {
    server {
        listen 127.0.0.1:1;
        proxy_pass 127.0.0.1:2;
    }
}
"#;
    let err = parse_str(src).unwrap_err().to_string();
    assert!(err.contains("unknown plugins directive"));
}
