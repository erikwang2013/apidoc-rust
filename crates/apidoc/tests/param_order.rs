//! 参数顺序：聚合后顺序与声明一致。
//! linkme 链接器迭代顺序非源码序，恢复依赖宏展开时分配的 seq。

use apidoc::{DocParam, DocRegistry};

#[allow(dead_code)]
#[apidoc::param(name = "alpha", ty = "int")]
#[apidoc::param(name = "beta", ty = "int")]
#[apidoc::param(name = "gamma", ty = "int")]
fn ordered_params() {}

#[allow(dead_code)]
#[apidoc::query(name = "q1")]
#[apidoc::query(name = "q2")]
#[apidoc::query(name = "q3")]
fn ordered_querys() {}

#[allow(dead_code)]
#[apidoc::returned(name = "r1", children = [
    { name = "c1", ty = "string" },
    { name = "c2", ty = "string" },
    { name = "c3", ty = "string" },
])]
fn ordered_returned() {}

#[allow(dead_code)]
// 三类注解交错声明：每个列表各自的顺序独立保持
#[apidoc::param(name = "p1")]
#[apidoc::query(name = "qq1")]
#[apidoc::returned(name = "rr1")]
#[apidoc::param(name = "p2")]
#[apidoc::query(name = "qq2")]
#[apidoc::returned(name = "rr2")]
fn interleaved() {}

fn names(params: &[DocParam]) -> Vec<&'static str> {
    params.iter().map(|p| p.name).collect()
}

#[test]
fn params_keep_declaration_order() {
    let eps = DocRegistry::collect();
    let ep = eps.iter().find(|e| e.params.iter().any(|p| p.name == "alpha")).unwrap();
    assert_eq!(names(&ep.params), ["alpha", "beta", "gamma"]);
}

#[test]
fn querys_keep_declaration_order() {
    let eps = DocRegistry::collect();
    let ep = eps.iter().find(|e| e.querys.iter().any(|p| p.name == "q1")).unwrap();
    assert_eq!(names(&ep.querys), ["q1", "q2", "q3"]);
}

#[test]
fn returned_keep_declaration_order_and_children_too() {
    let eps = DocRegistry::collect();
    let ep = eps.iter().find(|e| e.returned.iter().any(|p| p.name == "r1")).unwrap();
    assert_eq!(names(&ep.returned), ["r1"]);
    assert_eq!(names(&ep.returned[0].children), ["c1", "c2", "c3"]);
}

#[test]
fn interleaved_annotations_keep_per_list_order() {
    let eps = DocRegistry::collect();
    let ep = eps.iter().find(|e| e.params.iter().any(|p| p.name == "p1")).unwrap();
    assert_eq!(names(&ep.params), ["p1", "p2"]);
    assert_eq!(names(&ep.querys), ["qq1", "qq2"]);
    assert_eq!(names(&ep.returned), ["rr1", "rr2"]);
}
