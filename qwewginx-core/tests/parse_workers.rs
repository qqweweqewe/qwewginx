use std::path::PathBuf;

use qwewginx_core::config::parse_file;

#[test]
fn parse_workers_conf() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/workers.conf");
    let cfg = parse_file(&path).expect("parse workers.conf");
    assert_eq!(cfg.worker_processes, 4);
}
