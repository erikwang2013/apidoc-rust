//! v2 分享链接：为指定应用/接口生成可直接打开的文档深链。
//!
//! 链接形如 `{base}/apidoc?app=<appKey>&ep=<接口 url>&method=<方法>[&token=<t>]`，
//! UI 侧据 `app` 选中应用、据 `ep`+`method` 定位接口。开启鉴权时附带服务端
//! 签发的 token（应用密码优先，其次全局密码），打开即免密码。

use crate::auth::{self, AuthConfig};
use crate::{AppConfig, DocEndpoint};

/// 生成分享 URL。
/// - `base`：如 `https://api.example.com`，为空则输出相对路径（同源直接用）。
/// - `app_key`：应用/版本 key，`None` 表示默认应用。
/// - `ep`：要定位的接口，`None` 表示只定位应用。
/// - `token`：`share::token_for` 的返回值，`None` 则不带。
pub fn build_url(
    base: &str,
    app_key: Option<&str>,
    ep: Option<&DocEndpoint>,
    token: Option<&str>,
) -> String {
    let mut q: Vec<String> = Vec::new();
    if let Some(k) = app_key {
        q.push(format!("app={}", enc(k)));
    }
    if let Some(e) = ep {
        q.push(format!("ep={}", enc(&e.url)));
        q.push(format!("method={}", enc(&e.method)));
    }
    if let Some(t) = token {
        q.push(format!("token={}", enc(t)));
    }
    let path = format!("{}/apidoc", base.trim_end_matches('/'));
    if q.is_empty() {
        path
    } else {
        format!("{path}?{}", q.join("&"))
    }
}

/// 为应用（`None` = 全局）签发分享 token；没有可用密码时返回 `None`。
/// - 应用配置了独立密码：不论全局是否开启鉴权（含 `auth: None`）都签发 —— 守卫
///   `auth_guard_ok` 与 `/apidoc/auth` 在该路径同样只认应用密码。应用密码是空串时本函数
///   拒绝签发（失败关闭，且不回落全局），`auth_issue` / `auth_check_app` 同规则。
/// - 否则回落全局密码：未开鉴权或全局密码为空则不签发。
///
/// token 与 `/apidoc/auth` 同构，因此 UI 的既有 token 逻辑无需特殊处理。
pub fn token_for(auth: Option<&AuthConfig>, apps: &[AppConfig], app_key: Option<&str>) -> Option<String> {
    // auth 缺省时与守卫一致地用 AuthConfig::default() 的 secret/expire 签发，
    // 否则签出的 token 校验不过。
    let default = AuthConfig::default();
    let auth_cfg = auth.unwrap_or(&default);
    // 凭据判定与登录/守卫同一个来源（空密码失败关闭、应用密码优先都在里面）
    match auth::resolve_credential(app_key, auth_cfg, apps) {
        auth::Credential::Found(credential) => Some(auth::auth_token(credential, auth_cfg)),
        auth::Credential::Empty(_) | auth::Credential::None => None,
    }
}

/// 最小百分号编码：保留 RFC3986 未保留字符，其余按 UTF-8 逐字节转义。
fn enc(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
        }
    }
    out
}
