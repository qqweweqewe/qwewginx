use std::path::PathBuf;

use qwewginx_core::config::{parse_file, LocationAction};

#[test]
fn parse_h2_conf() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/h2.conf");
    let cfg = parse_file(&path).expect("parse h2.conf");
    match &cfg.http.servers[0].locations[0].action {
        LocationAction::Return(ret) => assert_eq!(ret.body, "hello from qwewginx h2\n"),
        _ => panic!("expected return"),
    }
}
