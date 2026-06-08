use std::path::PathBuf;

use qwewginx_core::config::{parse_file, LocationAction};

#[test]
fn parse_h2_conf() {
    // h2c is runtime-default on any listener — no dedicated h2.conf needed
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/echo.conf");
    let cfg = parse_file(&path).expect("parse echo.conf");
    match &cfg.http.servers[0].locations[0].action {
        LocationAction::Return(ret) => assert_eq!(ret.body, "hello from qwewginx\n"),
        _ => panic!("expected return"),
    }
}
