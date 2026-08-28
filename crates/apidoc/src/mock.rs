//! M4 在线调试的 mock 引擎：把聚合后的端点压缩成可 Clone 的子集，并按
//! `mock = "fake:规则名"` 注解生成 mock JSON。feature "mock" 门控
//! （axum/actix feature 隐含启用），默认构建不拉 fake 依赖（M4 红线）。

use std::sync::atomic::{AtomicBool, Ordering};

use crate::{DocEndpoint, DocParam};
use fake::locales::EN;
use fake::Fake;
use serde_json::{json, Map, Value};

/// 未知 fake 规则只警告一次（每次 /apidoc/mock 请求都会走到这里，全量刷屏无意义）。
static UNKNOWN_RULE_WARNED: AtomicBool = AtomicBool::new(false);

/// DocEndpoint 中 mock 引擎需要的子集（DocEndpoint 本身不可 Clone）。
/// url+method 用于路由精确匹配，三类参数用于生成 mock。
pub struct MockEndpointSpec {
    pub url: String,
    pub method: String,
    pub params: Vec<DocParam>,
    pub querys: Vec<DocParam>,
    pub route_params: Vec<DocParam>,
}

/// 从聚合端点列表复制出 mock 匹配所需的子集。
pub fn mock_specs(endpoints: &[DocEndpoint]) -> Vec<MockEndpointSpec> {
    endpoints
        .iter()
        .map(|e| MockEndpointSpec {
            url: e.url.clone(),
            method: e.method.clone(),
            params: e.params.clone(),
            querys: e.querys.clone(),
            route_params: e.route_params.clone(),
        })
        .collect()
}

/// 生成 mock 数据：`{"route_params":{...},"params":{...},"querys":{...}}`，
/// 某类参数为空时输出 `{}`。叶子值一律 String。
///
/// 每参数取值优先级：① mock 以 `fake:` 开头 → fake 规则表；未知名规则
/// eprintln 警告后落到 ③。② 其余非空 mock 原样直出。③ 无 mock：有 children
/// 递归成嵌套对象（array 生成 2 项），否则按 ty 取默认值。
pub fn generate_mock(spec: &MockEndpointSpec) -> Value {
    json!({
        "route_params": gen_params(&spec.route_params),
        "params": gen_params(&spec.params),
        "querys": gen_params(&spec.querys),
    })
}

fn gen_params(list: &[DocParam]) -> Value {
    let mut m = Map::new();
    for p in list {
        m.insert(p.name.to_string(), gen_value(p));
    }
    Value::Object(m)
}

fn gen_value(p: &DocParam) -> Value {
    match p.mock {
        Some(m) if m.starts_with("fake:") => fake_value(&m[5..], p),
        Some(m) if !m.is_empty() => Value::String(m.to_string()),
        _ => gen_by_ty(p),
    }
}

fn gen_by_ty(p: &DocParam) -> Value {
    if !p.children.is_empty() {
        if p.ty == "array" {
            // ponytail: 数组长度固定 2，要配置化等真有需求
            Value::Array(vec![gen_object(p.children), gen_object(p.children)])
        } else {
            gen_object(p.children)
        }
    } else {
        let leaf = match p.ty {
            "int" => "1",
            "float" => "0.5",
            "bool" => "true",
            "object" => "{}",
            _ => "string",
        };
        Value::String(leaf.to_string())
    }
}

fn gen_object(children: &[DocParam]) -> Value {
    let mut m = Map::new();
    for c in children {
        m.insert(c.name.to_string(), gen_value(c));
    }
    Value::Object(m)
}

/// fake 规则表：规则名 → 对应 faker（fake 3.x，默认 EN locale）。
fn fake_value(rule: &str, p: &DocParam) -> Value {
    use fake::faker::address::raw::{CityName, CountryName};
    use fake::faker::boolean::raw::Boolean;
    use fake::faker::chrono::raw::Date;
    use fake::faker::company::raw::CompanyName;
    use fake::faker::internet::raw::{DomainSuffix, IPv4, SafeEmail, Username};
    use fake::faker::lorem::raw::Paragraph;
    use fake::faker::name::raw::Name;
    use fake::faker::number::raw::NumberWithFormat;
    use fake::faker::phone_number::raw::PhoneNumber;
    use fake::uuid::UUIDv4;

    let s: String = match rule {
        "name" => Name(EN).fake(),
        "company" => CompanyName(EN).fake(),
        "email" => SafeEmail(EN).fake(),
        "phone" => PhoneNumber(EN).fake(),
        "url" => {
            let u: String = Username(EN).fake();
            let d: String = DomainSuffix(EN).fake();
            format!("https://{u}.{d}/")
        }
        "ip" => IPv4(EN).fake(),
        "city" => CityName(EN).fake(),
        "country" => CountryName(EN).fake(),
        "text" => Paragraph(EN, 3..5).fake(),
        "number" => NumberWithFormat(EN, "######").fake(),
        "int" => NumberWithFormat(EN, "########").fake(),
        "float" => format!("{:.2}", (0.0..1000.0).fake::<f64>()),
        "bool" => Boolean(EN, 50).fake::<bool>().to_string(),
        "uuid" => UUIDv4.fake(),
        "date" => Date(EN).fake(),
        other => {
            if UNKNOWN_RULE_WARNED
                .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                eprintln!(
                    "apidoc: unknown fake rule `fake:{other}` for param `{}`, falling back to type default",
                    p.name
                );
            }
            return gen_by_ty(p);
        }
    };
    Value::String(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &'static str, ty: &'static str, mock: Option<&'static str>) -> DocParam {
        DocParam { name, ty, required: false, default: None, desc: None, mock, children: &[] }
    }

    fn spec(url: &str, method: &str, params: Vec<DocParam>, querys: Vec<DocParam>) -> MockEndpointSpec {
        MockEndpointSpec { url: url.into(), method: method.into(), params, querys, route_params: vec![] }
    }

    fn mock_of<'a>(v: &'a Value, key: &str) -> &'a Value {
        v.get(key).unwrap()
    }

    // 优先级：显式 mock 值 > fake: 前缀 > ty 默认
    #[test]
    fn priority_explicit_mock_beats_fake_and_ty_default() {
        let v = generate_mock(&spec("/x", "GET", vec![
            param("a", "string", Some("explicit")),
            param("b", "string", Some("fake:name")),
            param("c", "int", None),
        ], vec![]));
        let params = mock_of(&v, "params");
        assert_eq!(params["a"], "explicit");
        let b = params["b"].as_str().unwrap();
        assert!(!b.is_empty() && b != "fake:name", "fake:name 应产出真实值, got {b}");
        assert_eq!(params["c"], "1");
    }

    // fake: 规则产出非空且不等于字面量
    #[test]
    fn fake_rules_produce_real_values() {
        let rules = [
            "fake:name", "fake:company", "fake:email", "fake:phone", "fake:url",
            "fake:ip", "fake:city", "fake:country", "fake:text", "fake:number",
            "fake:int", "fake:float", "fake:bool", "fake:uuid", "fake:date",
        ];
        let params: Vec<DocParam> = rules
            .iter()
            .enumerate()
            .map(|(i, r)| param(Box::leak(format!("p{i}").into_boxed_str()), "string", Some(r)))
            .collect();
        let v = generate_mock(&spec("/x", "GET", params, vec![]));
        let params = mock_of(&v, "params");
        for (i, r) in rules.iter().enumerate() {
            let got = params[&format!("p{i}")].as_str().unwrap();
            assert!(!got.is_empty(), "{r} 产出为空");
            assert_ne!(got, *r, "{r} 产出了字面量自身");
        }
    }

    // 未知名 fake 规则回退 ty 默认
    #[test]
    fn unknown_fake_rule_falls_back_to_ty_default() {
        let v = generate_mock(&spec("/x", "GET", vec![param("a", "int", Some("fake:no_such_rule"))], vec![]));
        assert_eq!(mock_of(&v, "params")["a"], "1");
    }

    // mock=""（空字符串，注解层可产生）不算显式值，落 ty 默认
    #[test]
    fn empty_mock_string_falls_back_to_ty_default() {
        let v = generate_mock(&spec("/x", "GET", vec![param("a", "int", Some(""))], vec![]));
        assert_eq!(mock_of(&v, "params")["a"], "1");
    }

    // 显式 mock / fake 规则优先于 children 递归（children 只在无 mock 时展开）
    #[test]
    fn explicit_mock_beats_children() {
        const CHILD: DocParam = DocParam {
            name: "id", ty: "int", required: false,
            default: None, desc: None, mock: None, children: &[],
        };
        static LITERAL: DocParam = DocParam {
            name: "user", ty: "object", required: false,
            default: None, desc: None, mock: Some("override"), children: &[CHILD],
        };
        static FAKED: DocParam = DocParam {
            name: "org", ty: "object", required: false,
            default: None, desc: None, mock: Some("fake:name"), children: &[CHILD],
        };
        let v = generate_mock(&spec("/x", "GET", vec![LITERAL.clone(), FAKED.clone()], vec![]));
        let params = mock_of(&v, "params");
        assert_eq!(params["user"], "override", "显式 mock 应覆盖 children");
        let org = params["org"].as_str().unwrap();
        assert!(!org.is_empty() && org != "fake:name", "fake 规则应覆盖 children, got {org}");
    }

    // children 递归嵌套对象；array 生成 2 项
    #[test]
    fn children_recurse_into_nested_object_and_array_makes_two() {
        const CHILD_ID: DocParam = DocParam {
            name: "id", ty: "int", required: false,
            default: None, desc: None, mock: None, children: &[],
        };
        const CHILD_NAME: DocParam = DocParam {
            name: "name", ty: "string", required: false,
            default: None, desc: None, mock: Some("fake:name"), children: &[],
        };
        static USER: DocParam = DocParam {
            name: "user", ty: "object", required: false,
            default: None, desc: None, mock: None, children: &[CHILD_ID, CHILD_NAME],
        };
        static ITEMS: DocParam = DocParam {
            name: "items", ty: "array", required: false,
            default: None, desc: None, mock: None, children: &[CHILD_ID],
        };
        let v = generate_mock(&spec("/x", "GET", vec![USER.clone(), ITEMS.clone()], vec![]));
        let params = mock_of(&v, "params");
        assert_eq!(params["user"]["id"], "1");
        let inner_name = params["user"]["name"].as_str().unwrap();
        assert!(!inner_name.is_empty() && inner_name != "fake:name");
        let items = params["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "array 应生成 2 项");
        assert_eq!(items[0]["id"], "1");
        assert_eq!(items[1]["id"], "1");
    }

    // 嵌套数组：array 内的 array 逐层展开成 2 项（ui 表单 mockAt 降入 [0] 依赖此形状）
    #[test]
    fn nested_array_recurses_two_items_per_level() {
        const C: DocParam = DocParam {
            name: "c", ty: "int", required: false,
            default: None, desc: None, mock: None, children: &[],
        };
        const B: DocParam = DocParam {
            name: "b", ty: "array", required: false,
            default: None, desc: None, mock: None, children: &[C],
        };
        static A: DocParam = DocParam {
            name: "a", ty: "array", required: false,
            default: None, desc: None, mock: None, children: &[B],
        };
        let v = generate_mock(&spec("/x", "GET", vec![A.clone()], vec![]));
        let a = mock_of(&v, "params")["a"].as_array().unwrap();
        assert_eq!(a.len(), 2, "外层 array 2 项");
        assert_eq!(a[0]["b"].as_array().unwrap().len(), 2, "内层 array 2 项");
        assert_eq!(a[0]["b"][0]["c"], "1");
    }

    // ty 默认映射
    #[test]
    fn ty_default_mapping() {
        let v = generate_mock(&spec("/x", "GET", vec![
            param("i", "int", None),
            param("f", "float", None),
            param("b", "bool", None),
            param("o", "object", None),
            param("s", "string", None),
            param("u", "unknown", None),
        ], vec![]));
        let params = mock_of(&v, "params");
        assert_eq!(params["i"], "1");
        assert_eq!(params["f"], "0.5");
        assert_eq!(params["b"], "true");
        assert_eq!(params["o"], "{}");
        assert_eq!(params["s"], "string");
        assert_eq!(params["u"], "string", "未知名 ty 落 string 默认");
    }

    // 空参数列表 → 对应键为 {}
    #[test]
    fn empty_lists_produce_empty_objects() {
        let v = generate_mock(&spec("/x", "GET", vec![], vec![]));
        assert_eq!(mock_of(&v, "route_params"), &serde_json::json!({}));
        assert_eq!(mock_of(&v, "params"), &serde_json::json!({}));
        assert_eq!(mock_of(&v, "querys"), &serde_json::json!({}));
    }

    // mock_specs 正确复制字段
    #[test]
    fn mock_specs_copies_endpoint_subset() {
        let mut ep = DocEndpoint {
            url: "/api/user/{id}".to_string(),
            method: "POST".to_string(),
            params: vec![param("name", "string", Some("alice"))],
            ..Default::default()
        };
        ep.querys = vec![param("page", "int", None)];
        ep.route_params = vec![param("id", "int", None)];
        let specs = mock_specs(&[ep]);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].url, "/api/user/{id}");
        assert_eq!(specs[0].method, "POST");
        assert_eq!(specs[0].params.len(), 1);
        assert_eq!(specs[0].querys.len(), 1);
        assert_eq!(specs[0].route_params.len(), 1);
        assert_eq!(specs[0].params[0].name, "name");
    }
}
