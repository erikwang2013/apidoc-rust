//! M6a 密码鉴权核心测试：token 签发/校验、过期、篡改、序列化红线与
//! auth_issue / auth_guard_ok 的策略（应用密码优先于全局密码）。

use apidoc::auth::{auth_check, auth_guard_ok, auth_issue, auth_token, md5_hex, AuthConfig, AuthResult};
use apidoc::{AppConfig, ApidocConfig, DocRegistry};
use serde_json::Value;
use std::time::Duration;

const PW: &str = "secret";

fn cfg(enable: bool) -> AuthConfig {
    AuthConfig { enable, password: PW.into(), secret_key: "k".into(), expire: 0 }
}

fn app_pw(password: Option<&str>) -> AppConfig {
    AppConfig {
        key: "api".into(),
        title: "API".into(),
        items: Vec::new(),
        password: password.map(String::from),
    }
}

#[test]
fn token_roundtrip_and_wrong_password_rejected() {
    let c = cfg(true);
    let ok = auth_token(&md5_hex(PW), &c);
    assert!(auth_check(&ok, &c), "正确密码签发的 token 应通过校验");
    let bad = auth_token(&md5_hex("wrong"), &c);
    assert!(!auth_check(&bad, &c), "错误密码签发的 token 不应通过校验");
    assert!(!auth_check("garbage", &c), "非 token 串不应通过校验");
}

#[test]
fn non_ascii_token_does_not_panic() {
    // 多字节 UTF-8 token 不得触发字节切片 panic（回归：字符边界越界切 str）
    let c = cfg(true);
    for t in ["€€x€€", "日本語", "🔥🔥"] {
        assert!(!auth_check(t, &c), "{t:?} 不应通过校验");
    }
}

#[test]
fn tampered_token_rejected() {
    let c = cfg(true);
    let t = auth_token(&md5_hex(PW), &c);
    let mut chars: Vec<char> = t.chars().collect();
    let idx = chars.len() / 2;
    chars[idx] = if chars[idx] == 'A' { 'B' } else { 'A' };
    let tampered: String = chars.into_iter().collect();
    assert_ne!(t, tampered);
    assert!(!auth_check(&tampered, &c), "篡改过的 token 不应通过校验");
}

#[test]
fn expired_token_rejected() {
    let mut c = cfg(true);
    c.expire = 1;
    let t = auth_token(&md5_hex(PW), &c);
    assert!(auth_check(&t, &c));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!auth_check(&t, &c), "过期 token 不应通过校验");
}

#[test]
fn auth_config_never_serializes_password_or_secret_key() {
    // 默认值 → 空对象
    assert_eq!(serde_json::to_string(&AuthConfig::default()).unwrap(), "{}");
    // 仅 enable 非默认；expire 0 与 86400（上游默认）都省略
    assert_eq!(serde_json::to_string(&cfg(true)).unwrap(), r#"{"enable":true}"#);
    let mut c = cfg(true);
    c.expire = 86400;
    assert_eq!(serde_json::to_string(&c).unwrap(), r#"{"enable":true}"#);
    c.expire = 3600;
    assert_eq!(serde_json::to_string(&c).unwrap(), r#"{"enable":true,"expire":3600}"#);
    // 红线：password / secret_key 明文永不进任何输出
    let s = serde_json::to_string(&cfg(true)).unwrap();
    assert!(!s.contains(PW) && !s.contains("secret_key"), "密码/密钥泄漏进序列化输出");
}

#[test]
fn config_auth_and_apps_omitted_when_absent() {
    let doc = DocRegistry::collect_doc(ApidocConfig {
        title: "t".into(),
        description: None,
        auth: None,
        apps: Vec::new(),
    });
    let v: Value = serde_json::to_value(&doc).unwrap();
    assert!(v["config"].get("auth").is_none(), "无 auth 配置时不得出现 auth 键（M5 字节级一致）");
    assert!(v.get("apps").is_none(), "无 app 注解时不得出现 apps 键（M5 字节级一致）");
    assert_eq!(v["config"]["title"], "t");
}

#[test]
fn auth_issue_policies() {
    // auth 未配置 → Disabled（404）
    assert!(matches!(auth_issue(&md5_hex(PW), None, None, &[]), AuthResult::Disabled));
    let c = cfg(true);
    // 全局：密码正确发 token，错误 → Error
    assert!(matches!(auth_issue(&md5_hex(PW), None, Some(&c), &[]), AuthResult::Token(_)));
    assert!(matches!(auth_issue(&md5_hex("nope"), None, Some(&c), &[]), AuthResult::Error));
}

#[test]
fn auth_issue_app_password_takes_priority() {
    let apps = vec![app_pw(Some("app-pw"))];
    let c = cfg(true);
    // 应用密码正确（全局密码故意错）→ Token：应用密码优先
    assert!(matches!(
        auth_issue(&md5_hex("app-pw"), Some("api"), Some(&c), &apps),
        AuthResult::Token(_)
    ));
    // 应用密码错误（全局密码正确）→ Error：不回落全局
    assert!(matches!(auth_issue(&md5_hex(PW), Some("api"), Some(&c), &apps), AuthResult::Error));
    // 应用未配置密码：回落全局规则
    let no_pw = vec![app_pw(None)];
    assert!(matches!(
        auth_issue(&md5_hex(PW), Some("api"), Some(&c), &no_pw),
        AuthResult::Token(_)
    ));
    // auth 未配置但应用有密码：应用密码仍可签发（verifyAuth 语义，独立于 enable）
    assert!(matches!(
        auth_issue(&md5_hex("app-pw"), Some("api"), None, &apps),
        AuthResult::Token(_)
    ));
}

#[test]
fn auth_guard_policies() {
    // 全局未启用：恒放行
    assert!(auth_guard_ok("", None, None, &[]));
    let c = cfg(true);
    assert!(!auth_guard_ok("", None, Some(&c), &[]), "启用后缺 token 应拒绝");
    let t = auth_token(&md5_hex(PW), &c);
    assert!(auth_guard_ok(&t, None, Some(&c), &[]));
    // 应用有密码：只认应用 token（全局未启用也拒绝缺 token）。
    // 签发与守卫必须用同一 auth 配置（真实流程都来自 doc.config.auth）：
    // 这里 auth 为 None，两侧都走默认 secret_key。
    let apps = vec![app_pw(Some("app-pw"))];
    assert!(!auth_guard_ok("", Some("api"), None, &apps), "应用有密码时缺 token 应拒绝");
    let app_t = auth_token(&md5_hex("app-pw"), &AuthConfig::default());
    assert!(auth_guard_ok(&app_t, Some("api"), None, &apps));
    assert!(!auth_guard_ok(&t, Some("api"), None, &apps), "全局 token 不能通过应用密码");
    // 应用无密码：回落全局规则
    let no_pw = vec![app_pw(None)];
    assert!(auth_guard_ok("", Some("api"), None, &no_pw), "应用无密码且全局未启用 → 放行");
}
