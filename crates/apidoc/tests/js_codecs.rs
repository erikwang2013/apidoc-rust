//! P1 纯 JS 编解码测试入口：cargo test 调 node --test。
//! Node 不在环境里时打印 skip，不让 Rust 测试因缺少外部工具而 red。

#[test]
fn js_codecs_pass() {
    let status = std::process::Command::new("node")
        .args(["--test", "tests/js/codecs.test.js"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status();
    match status {
        Ok(s) => assert!(s.success(), "node codec tests failed"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("node not found, skipping JS codec tests")
        }
        Err(e) => panic!("failed to run node: {e}"),
    }
}
