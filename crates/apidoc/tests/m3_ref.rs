//! M3 `r#ref` 注解语义：精确 id 匹配、链式解析（A→B→C）、环与缺失目标的
//! 安全降级（不 panic、returned 保持空）、重复 ref 覆盖、多命中取先者。

use apidoc::{DocEndpoint, DocRegistry};

#[allow(dead_code)]
#[apidoc::title("链目标")]
#[apidoc::returned(
    name = "base",
    ty = "object",
    children = [{ name = "id", ty = "int", required }],
)]
fn chain_target() {}

#[allow(dead_code)]
#[apidoc::title("链中继")]
#[apidoc::r#ref("chain_target")]
fn chain_mid() {}

#[allow(dead_code)]
#[apidoc::title("链末端")]
#[apidoc::r#ref("chain_mid")]
fn chain_end() {}

#[allow(dead_code)]
#[apidoc::title("环A")]
#[apidoc::r#ref("cycle_b")]
fn cycle_a() {}

#[allow(dead_code)]
#[apidoc::title("环B")]
#[apidoc::r#ref("cycle_a")]
fn cycle_b() {}

#[allow(dead_code)]
#[apidoc::title("缺失目标")]
#[apidoc::r#ref("no_such_fn")]
fn missing_target() {}

#[allow(dead_code)]
#[apidoc::title("ref覆盖")]
#[apidoc::r#ref("no_such_fn")]
#[apidoc::r#ref("chain_target")]
fn ref_overwrite() {}

#[allow(dead_code)]
#[apidoc::title("精确id目标")]
#[apidoc::returned(name = "exact", ty = "string", required)]
fn exact_target() {}

#[allow(dead_code)]
#[apidoc::title("精确id引用")]
#[apidoc::r#ref("m3_ref::exact_target")]
fn exact_ref() {}

mod nested {
    #[allow(dead_code)]
    #[apidoc::title("嵌套同名目标")]
    #[apidoc::returned(name = "nested_data", ty = "string", required)]
    pub fn chain_target() {}
}

#[allow(dead_code)]
#[apidoc::title("多命中引用")]
#[apidoc::r#ref("chain_target")]
fn multi_hit_ref() {}

fn find<'a>(eps: &'a [DocEndpoint], title: &str) -> &'a DocEndpoint {
    eps.iter().find(|e| e.title == title).expect("endpoint not found")
}

#[test]
fn ref_chain_resolves_recursively() {
    let eps = DocRegistry::collect();
    let end = find(&eps, "链末端");
    assert_eq!(end.returned.len(), 1);
    assert_eq!(end.returned[0].name, "base");
    assert_eq!(end.returned[0].children[0].name, "id");
    // 中继端点也被填充（递归副作用），目标自身不受影响
    assert_eq!(find(&eps, "链中继").returned[0].name, "base");
    assert_eq!(find(&eps, "链目标").returned.len(), 1);
}

#[test]
fn ref_cycle_degrades_without_panic() {
    let eps = DocRegistry::collect();
    assert!(find(&eps, "环A").returned.is_empty());
    assert!(find(&eps, "环B").returned.is_empty());
}

#[test]
fn missing_ref_target_yields_empty_returned() {
    let eps = DocRegistry::collect();
    let ep = find(&eps, "缺失目标");
    assert_eq!(ep.r#ref.as_deref(), Some("no_such_fn"));
    assert!(ep.returned.is_empty());
}

#[test]
fn repeated_ref_overwrites_later_wins() {
    let eps = DocRegistry::collect();
    let ep = find(&eps, "ref覆盖");
    assert_eq!(ep.r#ref.as_deref(), Some("chain_target"));
    assert_eq!(ep.returned[0].name, "base");
}

#[test]
fn ref_exact_id_matches_without_suffix_ambiguity() {
    let eps = DocRegistry::collect();
    let ep = find(&eps, "精确id引用");
    assert_eq!(ep.returned[0].name, "exact");
}

#[test]
fn ambiguous_ref_suffix_takes_first_declared() {
    // 外层 chain_target 先声明（seq 更小），多命中时取它而非 nested::chain_target
    let eps = DocRegistry::collect();
    let ep = find(&eps, "多命中引用");
    assert_eq!(ep.returned[0].name, "base");
}
