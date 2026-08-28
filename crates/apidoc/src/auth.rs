//! M6a 密码鉴权：逐行移植上游 apidoc-php 的 Auth.php（createToken / checkToken /
//! verifyAuth / checkAuth）。
//!
//! token = authcode(JSON{key: md5(md5(原始密码)), expire: now+expire}, secret_key)
//! authcode 是 Discuz 风格加密封套：RC4 式 XOR + md5 校验和 + 标准 base64（无 padding）。
//! 客户端提交 md5(密码)，服务端比对 md5(配置密码)；密码与 secret_key 永不出现在任何输出。

use crate::AppConfig;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 上游 Auth.php 的空值默认：secret_key 缺省、expire 为 0 时。
pub const DEFAULT_SECRET_KEY: &str = "apidoc#hgcode";
pub const DEFAULT_EXPIRE: u64 = 86400;

/// 密码鉴权配置。password / secret_key 序列化时永久跳过（泄漏红线）；
/// enable 为 false、expire 为默认值时也从 api.json 省略。
#[derive(Clone, Serialize, Default)]
pub struct AuthConfig {
    #[serde(skip_serializing_if = "is_false")]
    pub enable: bool,
    #[serde(skip)]
    pub password: String,
    #[serde(skip)]
    pub secret_key: String,
    #[serde(skip_serializing_if = "is_default_expire")]
    pub expire: u64,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_default_expire(e: &u64) -> bool {
    *e == 0 || *e == DEFAULT_EXPIRE
}

/// 便捷 md5 十六进制，测试与外部调用共用。
pub fn md5_hex(s: &str) -> String {
    format!("{:x}", md5::compute(s.as_bytes()))
}

/// 恒定时间字符串比较（token MAC 校验用，防时序侧信道；长度不等直接 false，
/// 双方为固定长度 hex，长度差无信息量）。
fn const_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn effective_secret_key(cfg: &AuthConfig) -> &str {
    if cfg.secret_key.is_empty() { DEFAULT_SECRET_KEY } else { &cfg.secret_key }
}

fn effective_expire(cfg: &AuthConfig) -> u64 {
    if cfg.expire == 0 { DEFAULT_EXPIRE } else { cfg.expire }
}

/// 创建 token（对齐上游 createToken）。`password_md5` 是客户端提交的 md5 值
/// （已与服务端 md5(配置密码) 比对通过），载荷 key = md5(md5(原始密码)) = md5(password_md5)。
pub fn auth_token(password_md5: &str, cfg: &AuthConfig) -> String {
    let key = md5_hex(password_md5);
    let expire = now() + effective_expire(cfg);
    let payload = format!(r#"{{"key":"{key}","expire":{expire}}}"#);
    encrypt(&payload, effective_secret_key(cfg))
}

/// 校验全局 token（cfg.password 为原始密码）。
pub fn auth_check(token: &str, cfg: &AuthConfig) -> bool {
    check_token(token, &md5_hex(&md5_hex(&cfg.password)), cfg)
}

/// 校验应用 token（app_password 为应用原始密码）。
pub fn auth_check_app(token: &str, app_password: &str, cfg: &AuthConfig) -> bool {
    check_token(token, &md5_hex(&md5_hex(app_password)), cfg)
}

fn check_token(token: &str, expected_key: &str, cfg: &AuthConfig) -> bool {
    let Some(plain) = decrypt(token, effective_secret_key(cfg)) else {
        return false;
    };
    let Ok(payload) = serde_json::from_str::<TokenPayload>(&plain) else {
        return false;
    };
    payload.key == expected_key && payload.expire > now()
}

#[derive(Deserialize)]
struct TokenPayload {
    key: String,
    expire: u64,
}

/// /apidoc/auth 的裁决（对齐上游 verifyAuth：应用密码优先于全局密码）。
pub enum AuthResult {
    /// auth 未启用（路由返回 404）。
    Disabled,
    /// 密码错误（401 {"error":"password error"}）。
    Error,
    Token(String),
}

/// 数据路由守卫失败时的 401 响应体（两适配器共享，1:1 对齐不靠复制保证）。
pub const DENIED_BODY: &str = r#"{"error":"password required"}"#;

/// AuthResult → HTTP 响应（状态码 + body；Disabled 为 404 无 body，与原行为一致）。
pub fn auth_result_response(r: AuthResult) -> (u16, String) {
    match r {
        AuthResult::Disabled => (404, String::new()),
        AuthResult::Error => (401, r#"{"error":"password error"}"#.to_string()),
        AuthResult::Token(t) => (200, serde_json::json!({"token": t}).to_string()),
    }
}

pub fn auth_issue(
    password_md5: &str,
    app_key: Option<&str>,
    auth: Option<&AuthConfig>,
    apps: &[AppConfig],
) -> AuthResult {
    let default = AuthConfig::default();
    let cfg = auth.unwrap_or(&default);
    if let Some(app) = app_key.and_then(|k| crate::find_app(apps, k)) {
        if let Some(pw) = &app.password {
            return if md5_hex(pw) == password_md5 {
                AuthResult::Token(auth_token(password_md5, cfg))
            } else {
                AuthResult::Error
            };
        }
    }
    if cfg.enable {
        if cfg.secret_key.is_empty() {
            warn_default_secret_key();
        }
        if md5_hex(&cfg.password) == password_md5 {
            AuthResult::Token(auth_token(password_md5, cfg))
        } else {
            AuthResult::Error
        }
    } else {
        AuthResult::Disabled
    }
}

/// 数据路由守卫（对齐上游 checkAuth）：true 放行。
/// appKey 指定且该应用配置了独立密码 → 只校验应用 token；否则全局 auth 未启用放行。
pub fn auth_guard_ok(
    token: &str,
    app_key: Option<&str>,
    auth: Option<&AuthConfig>,
    apps: &[AppConfig],
) -> bool {
    let default = AuthConfig::default();
    let cfg = auth.unwrap_or(&default);
    if let Some(app) = app_key.and_then(|k| crate::find_app(apps, k)) {
        if let Some(pw) = app.password.as_deref() {
            return auth_check_app(token, pw, cfg);
        }
    }
    !cfg.enable || auth_check(token, cfg)
}

// ---------- authcode 加密封套（上游 Auth::handleToken 逐行移植） ----------

/// 默认 secret_key 是公开常量，配合 URL 明文 md5 提交，抓包即可离线伪造 token，
/// 只在启用鉴权且未配 secret_key 时警告一次（每次 /apidoc/auth 请求都会走到这里）。
static DEFAULT_KEY_WARNED: AtomicBool = AtomicBool::new(false);

fn warn_default_secret_key() {
    if DEFAULT_KEY_WARNED
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        eprintln!(
            "apidoc: auth enabled with default secret_key `{DEFAULT_SECRET_KEY}` — set a custom secret_key and serve over HTTPS"
        );
    }
}

static KEYC_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 随机 4 字符 keyc：md5(纳秒时间戳+自增计数) 尾部 4 个 hex 字符，
/// 等价上游 substr(md5(microtime()), -4)。
fn keyc() -> String {
    let seed = format!(
        "{}{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        KEYC_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let hex = format!("{:x}", md5::compute(seed.as_bytes()));
    hex[hex.len() - 4..].to_string()
}

/// 加密：keyc + base64(rc4("0000000000" + md5(payload+keyb)[0..16] + payload))。
fn encrypt(payload: &str, secret_key: &str) -> String {
    let key = md5_hex(secret_key);
    let (keya, keyb) = key.split_at(16);
    let keya = md5_hex(keya);
    let keyb = md5_hex(keyb);
    let keyc = keyc();
    let cryptkey = format!("{keya}{}", md5_hex(&format!("{keya}{keyc}")));
    let checksum = &md5_hex(&format!("{payload}{keyb}"))[0..16];
    // expiry 恒为 0（对齐 createToken 不传 expiry）：10 位 '0' + 16 位校验和 + 载荷
    let plain = format!("0000000000{checksum}{payload}");
    let cipher = rc4(plain.as_bytes(), cryptkey.as_bytes());
    format!("{keyc}{}", base64_encode(&cipher))
}

/// 解密：authcode 校验和通过才返回载荷，否则 None。
fn decrypt(token: &str, secret_key: &str) -> Option<String> {
    if token.len() < 4 {
        return None;
    }
    let key = md5_hex(secret_key);
    let (keya, keyb) = key.split_at(16);
    let keya = md5_hex(keya);
    let keyb = md5_hex(keyb);
    let keyc = token.get(0..4)?;
    let cryptkey = format!("{keya}{}", md5_hex(&format!("{keya}{keyc}")));
    let body = base64_decode(token.get(4..)?)?;
    let result = rc4(&body, cryptkey.as_bytes());
    if result.len() < 26 {
        return None;
    }
    // 前 10 位是 authcode 过期戳：恒 0 或未过期（本实现加密侧恒为 0）
    let exp: u64 = String::from_utf8_lossy(&result[0..10]).parse().unwrap_or(0);
    if exp != 0 && exp <= now() {
        return None;
    }
    let expect = &String::from_utf8_lossy(&result[10..26]);
    let got = &md5_hex(&format!("{}{}", String::from_utf8_lossy(&result[26..]), keyb))[0..16];
    // MAC 比对走恒定时间，防时序侧信道
    const_time_eq(expect, got).then(|| String::from_utf8_lossy(&result[26..]).into_owned())
}

/// Discuz authcode 的 RC4 变体流加密（加密与解密同函数）。
fn rc4(data: &[u8], cryptkey: &[u8]) -> Vec<u8> {
    let mut rndkey = [0u8; 256];
    for (i, k) in rndkey.iter_mut().enumerate() {
        *k = cryptkey[i % cryptkey.len()];
    }
    let mut box_ = [0u8; 256];
    for (i, b) in box_.iter_mut().enumerate() {
        *b = i as u8;
    }
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(box_[i]).wrapping_add(rndkey[i]);
        box_.swap(i, j as usize);
    }
    let mut a = 0u8;
    let mut j = 0u8;
    let mut out = vec![0u8; data.len()];
    for (i, &byte) in data.iter().enumerate() {
        a = a.wrapping_add(1);
        j = j.wrapping_add(box_[a as usize]);
        box_.swap(a as usize, j as usize);
        out[i] = byte ^ box_[(box_[a as usize] as usize + box_[j as usize] as usize) % 256];
    }
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 标准 base64，无 padding（对齐 PHP base64_encode + str_replace('=')）。
fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() / 3 * 4 + 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(B64[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(B64[n as usize & 63] as char);
        }
    }
    out
}

/// 标准 base64 解码（容忍无 padding），非法字符返回 None。
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => return None,
            };
            n |= (v as u32) << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}
