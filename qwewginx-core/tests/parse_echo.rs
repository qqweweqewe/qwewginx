use std::path::PathBuf;

use qwewginx_core::config::parse_file;

#[test]
fn parse_echo_conf() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/echo.conf");
    let cfg = parse_file(&path).expect("parse echo.conf");
    assert_eq!(cfg.worker_processes, 1);
    assert_eq!(cfg.events.worker_connections, 1024);
    assert_eq!(cfg.http.servers.len(), 1);
    let srv = &cfg.http.servers[0];
    assert_eq!(srv.listeners.len(), 1);
    assert!(!srv.listeners[0].ssl);
    assert_eq!(srv.locations.len(), 1);
    assert_eq!(srv.locations[0].path, "/");
    match &srv.locations[0].action {
        qwewginx_core::config::LocationAction::Return(ret) => {
            assert_eq!(ret.status, 200);
            assert_eq!(ret.body, "hello from qwewginx\n");
        }
        _ => panic!("expected return"),
    }
}
