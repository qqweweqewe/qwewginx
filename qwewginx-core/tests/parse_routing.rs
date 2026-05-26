use std::path::PathBuf;

use qwewginx_core::config::parse_file;

#[test]
fn parse_routing_conf() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/routing.conf");
    let cfg = parse_file(&path).expect("parse routing.conf");
    let srv = &cfg.http.servers[0];
    assert_eq!(srv.locations.len(), 3);
    let paths: Vec<_> = srv.locations.iter().map(|l| l.path.as_str()).collect();
    assert!(paths.contains(&"/"));
    assert!(paths.contains(&"/api"));
    assert!(paths.contains(&"/api/v1"));
}
