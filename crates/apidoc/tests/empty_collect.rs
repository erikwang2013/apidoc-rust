//! 无任何注解的 crate：collect() 优雅返回空，不 panic。

use apidoc::DocRegistry;

#[test]
fn no_annotations_yields_empty_vec() {
    assert!(DocRegistry::collect().is_empty());
}
