//! 跨 crate 收集：fixture crate（apidoc-test-fixtures）与本测试 crate
//! 的注解片段汇入同一 registry；两个 crate 都有同名 fn ping()，
//! id 由 crate 前缀（module_path!）区分，必须产出两个独立 endpoint。

use apidoc::DocRegistry;

#[allow(dead_code)]
#[apidoc::title("test crate ping")]
#[apidoc::url("/test/ping")]
#[apidoc::method("POST")]
fn ping() {}

#[test]
fn fragments_from_two_crates_coexist_and_do_not_collide() {
    // 触发 fixture crate 被链接器拉入（rlib 归档成员未被引用会被丢弃）
    apidoc_test_fixtures::ping();
    let eps = DocRegistry::collect();
    let titles: Vec<&str> = eps.iter().map(|e| e.title.as_str()).collect();
    assert!(
        titles.contains(&"fixture crate ping"),
        "missing fixture fragment, got {titles:?}"
    );
    assert!(
        titles.contains(&"test crate ping"),
        "missing local fragment, got {titles:?}"
    );
    // 同名 fn 在不同 crate 中不串扰
    assert_eq!(eps.len(), 2, "expected exactly 2 endpoints, got {titles:?}");
}
