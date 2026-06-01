use std::path::PathBuf;

use qwewginx_core::config::{parse_file, LocationAction};

#[test]
fn parse_tls_conf() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/tls.conf");
    let cfg = parse_file(&path).expect("parse tls.conf");
    assert_eq!(cfg.http.servers.len(), 2);

    let tls_srv = &cfg.http.servers[0];
    assert_eq!(tls_srv.listeners.len(), 1);
    assert!(tls_srv.listeners[0].ssl);
    assert_eq!(tls_srv.listeners[0].addr.port(), 443);
    let files = tls_srv.tls.as_ref().expect("tls files");
    assert!(files.cert.ends_with("examples/tls/cert.pem"));
    match &tls_srv.locations[0].action {
        LocationAction::Return(ret) => assert_eq!(ret.body, "hello from qwewginx tls\n"),
        _ => panic!("expected return"),
    }

    let plain = &cfg.http.servers[1];
    assert!(!plain.listeners[0].ssl);
    assert_eq!(plain.listeners[0].addr.port(), 80);
    assert!(plain.tls.is_none());
}
