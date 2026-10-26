#[test]
fn fixtures_load() {
    let raw = std::fs::read_to_string("../../contract/uic-1/fixtures/menus.json")
        .expect("menus fixture must exist");
    let rows: serde_json::Value = serde_json::from_str(&raw).expect("menus fixture must be JSON");
    let rows = rows.as_array().expect("fixtures must be an array");
    assert!(!rows.is_empty(), "no fixtures");
    assert_eq!(rows[0]["response"]["code"], 0);
}
