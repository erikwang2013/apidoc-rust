//! v2 分享链接测试：`build_url` 的 base 归一化、query 参数取舍与百分号编码，
//! `token_for` 的签发条件（未开鉴权/无有效密码不签发、应用独立密码优先），
//! 以及签发的 token 与既有校验函数（auth_check / auth_check_app / auth_guard_ok）
//! 的互认 —— 后者是硬性正确性验收：链接里的 token 必须真的能免密解锁数据路由。

use apidoc::auth::{auth_check, auth_check_app, auth_guard_ok, AuthConfig};
use apidoc::share::{build_url, token_for};
use apidoc::{AppConfig, ApidocConfig, DocEndpoint};

fn ep(url: &str, method: &str) -> DocEndpoint {
    DocEndpoint { url: url.to_string(), method: method.to_string(), ..Default::default() }
}

fn app(key: &str, password: Option<&str>) -> AppConfig {
    AppConfig {
        key: key.into(),
        title: key.into(),
        items: Vec::new(),
        password: password.map(String::from),
    }
}

fn cfg(enable: bool, password: &str, apps: Vec<AppConfig>) -> ApidocConfig {
    ApidocConfig {
        auth: Some(AuthConfig {
            enable,
            password: password.into(),
            secret_key: "test-secret".into(),
            expire: 0,
        }),
        apps,
        ..Default::default()
    }
}

fn auth_of(c: &ApidocConfig) -> &AuthConfig {
    c.auth.as_ref().expect("测试配置恒含 auth")
}

/// auth: None 的配置：守卫与签发都走 AuthConfig::default() 的 secret/expire。
fn cfg_no_auth(apps: Vec<AppConfig>) -> ApidocConfig {
    ApidocConfig { apps, ..Default::default() }
}

/// 取 URL query 中的某个参数并解码（模拟 UI 读 query），顺带验证编码无损可逆。
fn query(url: &str, key: &str) -> String {
    let qs = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    for pair in qs.split('&') {
        let (k, v) = pair.split_once('=').expect("参数段应形如 k=v");
        if k == key {
            return decode(v);
        }
    }
    panic!("URL 缺少参数 {key}: {url}");
}

/// 最小百分号解码（仅用于断言；输入为 enc 产出的纯 ASCII）。
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).expect("合法两位 hex"));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("参数应为合法 UTF-8")
}

// ---------- build_url ----------

#[test]
fn base_forms_normalize_to_single_slash() {
    // 带尾斜杠、不带尾斜杠、空串三种输入（空串 → 同源相对路径）
    assert_eq!(build_url("https://a.com/", None, None, None), "https://a.com/apidoc");
    assert_eq!(build_url("https://a.com", None, None, None), "https://a.com/apidoc");
    assert_eq!(build_url("", None, None, None), "/apidoc");
    // 多余尾斜杠一并归一，不产生 //apidoc
    assert_eq!(build_url("https://a.com//", None, None, None), "https://a.com/apidoc");
    // 带子路径的部署（base 只吃掉尾斜杠）
    assert_eq!(build_url("https://a.com/docs/", None, None, None), "https://a.com/docs/apidoc");
}

#[test]
fn query_params_are_selected_and_ordered() {
    let e = ep("/api/user/list", "GET");
    // 什么都不给：无 ? 段
    assert_eq!(build_url("", None, None, None), "/apidoc");
    // 只给 app：不出 ep / method / token，且无悬空 &
    assert_eq!(build_url("", Some("api"), None, None), "/apidoc?app=api");
    // 只给接口：不出 app
    assert_eq!(
        build_url("", None, Some(&e), None),
        "/apidoc?ep=%2Fapi%2Fuser%2Flist&method=GET"
    );
    // 全给：顺序恒为 app → ep → method → token
    assert_eq!(
        build_url("https://a.com", Some("api"), Some(&e), Some("T0K3N")),
        "https://a.com/apidoc?app=api&ep=%2Fapi%2Fuser%2Flist&method=GET&token=T0K3N"
    );
}

#[test]
fn encoding_escapes_reserved_and_keeps_unreserved() {
    // RFC3986 未保留字符 -_.~ 原样保留
    assert_eq!(build_url("", Some("a-b_c.d~e"), None, None), "/apidoc?app=a-b_c.d~e");
    // / 空格 中文 & = 全部转义（中文按 UTF-8 逐字节）
    assert_eq!(
        build_url("", Some("应用"), Some(&ep("/路径/x y", "GET")), Some("a+b/c=")),
        "/apidoc?app=%E5%BA%94%E7%94%A8&ep=%2F%E8%B7%AF%E5%BE%84%2Fx%20y\
         &method=GET&token=a%2Bb%2Fc%3D"
    );
}

#[test]
fn encoding_is_reversible() {
    // 编码必须无损：解回来要和原值逐字符一致，否则 UI 定位不到接口
    let e = ep("/路径/x y&z=1", "GET");
    let url = build_url("", Some("应用&1"), Some(&e), Some("a+b/c="));
    assert_eq!(query(&url, "app"), "应用&1");
    assert_eq!(query(&url, "ep"), "/路径/x y&z=1");
    assert_eq!(query(&url, "method"), "GET");
    assert_eq!(query(&url, "token"), "a+b/c=");
}

// ---------- token_for ----------

#[test]
fn global_branch_needs_enabled_auth_and_password() {
    assert!(token_for(&cfg(true, "", vec![]), None).is_none(), "全局密码为空：不签发");
    assert!(token_for(&cfg(false, "gpass", vec![]), None).is_none(), "未开鉴权：不签发");
    assert!(token_for(&ApidocConfig::default(), None).is_none(), "未配置 auth：不签发");
    assert!(
        token_for(&cfg(true, "gpass", vec![app("api", None)]), Some("missing")).is_some(),
        "未知应用回落全局密码（全局密码存在，应签发）"
    );
    // 边界：应用密码配成空串时本函数拒绝签发（失败关闭）、不回落全局 —— 因为数据路由守卫
    // 对该应用走 auth_check_app(_, "", _)，用全局密码签出的 token 反而会被拒，宁可不给 token。
    // 注意 auth_issue 在该边界仍会签发（M6 既有行为），二者不一致，要统一应改 auth 侧。
    assert!(
        token_for(&cfg(true, "gpass", vec![app("api", Some(""))]), Some("api")).is_none(),
        "空串应用密码：不签发"
    );
}

#[test]
fn global_token_passes_auth_check() {
    let c = cfg(true, "gpass", vec![app("api", None)]);
    let ac = auth_of(&c);
    // 显式全局（None）与未知应用 key 都应落回全局密码
    for key in [None, Some("api"), Some("missing")] {
        let t = token_for(&c, key).expect("全局密码存在应签发 token");
        assert!(auth_check(&t, ac), "{key:?} 签发的全局 token 必须通过 auth_check");
    }
}

#[test]
fn app_token_passes_auth_check_app() {
    let c = cfg(true, "gpass", vec![app("api", Some("apppass"))]);
    let ac = auth_of(&c);

    let t = token_for(&c, Some("api")).expect("应用独立密码应签发 token");
    // 硬性验收：应用 token 必须被 auth_check_app 认账（= 数据路由守卫对该应用放行）
    assert!(auth_check_app(&t, "apppass", ac), "应用 token 必须通过 auth_check_app");
    assert!(!auth_check(&t, ac), "应用 token 不应被全局校验接受");

    // 应用独立密码优先：全局 token 反向不被应用校验接受
    let g = token_for(&c, None).expect("全局密码应签发 token");
    assert!(auth_check(&g, ac), "全局 token 必须通过 auth_check");
    assert!(!auth_check_app(&g, "apppass", ac), "全局 token 不应通过应用密码校验");
    assert!(!auth_check_app(&t, "gpass", ac), "拿全局密码去校验应用 token 应失败");
}

#[test]
fn app_password_signs_even_without_global_auth() {
    // 回归：守卫对「有独立密码的应用」无条件要应用 token（与全局 enable / auth 有无无关），
    // 全局未开鉴权或根本没配 auth 时也必须签发，否则分享链接对该应用 401。
    let default = AuthConfig::default();
    for c in [
        cfg(false, "gpass", vec![app("api", Some("apppass"))]), // 全局未开鉴权
        cfg_no_auth(vec![app("api", Some("apppass"))]),          // 完全没有 auth 配置
    ] {
        // 守卫/signing 都走同一份配置（auth 缺省时即 AuthConfig::default()）
        let ac = c.auth.as_ref().unwrap_or(&default);
        let t = token_for(&c, Some("api")).expect("应用独立密码应签发，不受全局 enable/auth 影响");
        assert!(auth_check_app(&t, "apppass", ac), "应用 token 必须通过 auth_check_app");
        assert!(auth_guard_ok(&t, Some("api"), c.auth.as_ref(), &c.apps), "守卫必须放行");
        assert!(!auth_guard_ok("", Some("api"), c.auth.as_ref(), &c.apps), "空 token 不应放行");
    }
    // 未开鉴权 + 应用无独立密码：守卫本就不拦，仍不签发（全局分支）
    assert!(token_for(&cfg(false, "gpass", vec![app("api", None)]), Some("api")).is_none());
    assert!(token_for(&cfg_no_auth(vec![app("api", None)]), Some("api")).is_none());
}

#[test]
fn nested_app_password_is_found() {
    // find_app 递归：嵌套版本节点的独立密码同样生效
    let mut parent = app("api", None);
    parent.items.push(app("v2", Some("v2pass")));
    let c = cfg(true, "gpass", vec![parent]);
    let t = token_for(&c, Some("v2")).expect("嵌套应用密码应被找到");
    assert!(auth_check_app(&t, "v2pass", auth_of(&c)));
}

// ---------- 端到端：链接里的 token 真的能免密 ----------

#[test]
fn share_url_token_unlocks_data_route() {
    // 应用独立密码场景：token 经 URL 编码后取回，必须能解锁该应用数据路由
    let c = cfg(true, "gpass", vec![app("api", Some("apppass"))]);
    let t = token_for(&c, Some("api")).expect("应签发 token");
    let url = build_url("https://a.com", Some("api"), Some(&ep("/api/x", "GET")), Some(&t));
    let from_url = query(&url, "token");
    assert_eq!(from_url, t, "URL 里的 token 应无损取回");
    assert!(auth_guard_ok(&from_url, Some("api"), c.auth.as_ref(), &c.apps), "分享链接应免密访问");
    assert!(!auth_guard_ok("", Some("api"), c.auth.as_ref(), &c.apps), "空 token 不应放行");
    assert!(!auth_guard_ok(&from_url, Some("nope"), c.auth.as_ref(), &c.apps), "全局未开该 token 场景应拒");

    // 全局密码场景（应用无独立密码）：同一个 token 对默认应用与该应用都放行
    let c2 = cfg(true, "gpass", vec![app("api", None)]);
    let t2 = token_for(&c2, Some("api")).expect("应回落全局密码");
    assert!(auth_guard_ok(&t2, Some("api"), c2.auth.as_ref(), &c2.apps));
    assert!(auth_guard_ok(&t2, None, c2.auth.as_ref(), &c2.apps));
    assert!(!auth_guard_ok("bad", None, c2.auth.as_ref(), &c2.apps), "乱码 token 不应放行");
}
