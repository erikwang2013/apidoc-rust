//! 文档访问密码鉴权：Rust 原生实现（无状态签名令牌 + 恒定时间比对 + 失败退避）。
//!
//! - **登录**：`POST /apidoc/auth`，body `{"password": "...", "appKey": "..."}`。
//!   密码走请求体（不进 URL、不进访问日志与浏览器历史），服务端恒定时间比对；
//!   客户端不做任何摘要——把密码的哈希当"可重放凭据"用，等于把摘要本身变成密码。
//! - **令牌**：`base64url(payload) + "." + base64url(HMAC-SHA256(payload))`，
//!   payload 为 `{"exp": <unix 秒>}`（serde 序列化，写入/读取同一份结构定义）。
//!   签名密钥 = HMAC-SHA256(secret_key, 凭据)，因此**改密码即刻作废旧令牌**。
//!   校验：恒定时间比对 MAC → 检查过期。令牌只用 URL 安全字符，无需再编码。
//! - **secret_key 缺省**：取进程内随机值（重启即失效），不存在公开默认密钥。
//! - **防爆破**：登录失败按凭据分别计退避（前 3 次不罚，之后 1s→2s→…封顶 10s；
//!   静默 2 分钟自动清零）。退避是进程内的，多实例部署要挡爆破应放到网关。
//! 密码与 secret_key 永不出现在任何输出里。

use crate::AppConfig;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// 令牌有效期缺省 24 小时。
pub const DEFAULT_EXPIRE: u64 = 86400;

/// 令牌版本参与密钥派生：将来换签名方案时，旧令牌自然失效，不需要额外的版本字段。
const TOKEN_VERSION: &[u8] = b"apidoc-token-v1";

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

/// 恒定时间字节比较（MAC 校验与密码比对共用）。长度不等直接 false：
/// 两侧长度或为固定 32 字节摘要、或长度本身不是秘密。
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 密码比对：比两侧的 SHA-256 摘要，长度差也不产生信息（两侧都固定 32 字节）。
fn password_eq(a: &str, b: &str) -> bool {
    ct_eq(&Sha256::digest(a.as_bytes()), &Sha256::digest(b.as_bytes()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn effective_expire(cfg: &AuthConfig) -> u64 {
    if cfg.expire == 0 { DEFAULT_EXPIRE } else { cfg.expire }
}

/// secret_key 缺省时的进程内随机密钥（首次签发/校验时生成一次）。
/// 重启即失效——比"公开默认常量"安全得多，配了 secret_key 的部署不受影响。
fn random_secret_key() -> &'static [u8] {
    static KEY: OnceLock<[u8; 32]> = OnceLock::new();
    KEY.get_or_init(|| {
        warn_random_key();
        let mut k = [0u8; 32];
        getrandom::getrandom(&mut k).expect("OS entropy source must be available");
        k
    })
}

fn secret_key(cfg: &AuthConfig) -> &[u8] {
    if cfg.secret_key.is_empty() { random_secret_key() } else { cfg.secret_key.as_bytes() }
}

/// 由 (secret_key, 凭据) 派生的令牌签名密钥：换密码 = 换密钥 = 旧令牌全失效。
fn signing_key(cfg: &AuthConfig, credential: &str) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret_key(cfg)).expect("HMAC accepts any key length");
    mac.update(TOKEN_VERSION);
    mac.update(b"\0");
    mac.update(credential.as_bytes());
    mac.finalize().into_bytes().into()
}

#[derive(Deserialize, Serialize)]
struct TokenPayload {
    exp: u64,
}

/// 为凭据（全局密码 / 应用独立密码）签发令牌。签发侧与校验侧共用本函数派生密钥，
/// 因此两侧必须传入同一个凭据。
pub fn auth_token(credential: &str, cfg: &AuthConfig) -> String {
    // saturating：expire 配成 u64::MAX 时 `now + expire` 会溢出 panic（debug）/回绕成过去
    let payload = serde_json::to_vec(&TokenPayload { exp: now().saturating_add(effective_expire(cfg)) })
        .expect("token payload must serialize");
    let mut mac = HmacSha256::new_from_slice(&signing_key(cfg, credential))
        .expect("HMAC accepts any key length");
    mac.update(&payload);
    let sig = mac.finalize().into_bytes();
    format!("{}.{}", URL_SAFE_NO_PAD.encode(&payload), URL_SAFE_NO_PAD.encode(sig))
}

/// 校验令牌：签名恒定时间比对 + 过期检查。
fn verify(token: &str, credential: &str, cfg: &AuthConfig) -> bool {
    let Some((payload_b64, sig_b64)) = token.split_once('.') else {
        return false;
    };
    let (Ok(payload), Ok(sig)) =
        (URL_SAFE_NO_PAD.decode(payload_b64), URL_SAFE_NO_PAD.decode(sig_b64))
    else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(&signing_key(cfg, credential))
        .expect("HMAC accepts any key length");
    mac.update(&payload);
    if !ct_eq(&sig, &mac.finalize().into_bytes()) {
        return false;
    }
    serde_json::from_slice::<TokenPayload>(&payload).is_ok_and(|p| p.exp > now())
}

/// 校验全局令牌（cfg.password 为配置里的原始密码）。
pub fn auth_check(token: &str, cfg: &AuthConfig) -> bool {
    if cfg.password.is_empty() {
        return false;
    }
    verify(token, &cfg.password, cfg)
}

/// 校验应用令牌（app_password 为应用原始密码）。
pub fn auth_check_app(token: &str, app_password: &str, cfg: &AuthConfig) -> bool {
    if app_password.is_empty() {
        return false;
    }
    verify(token, app_password, cfg)
}

/// POST /apidoc/auth 的请求体。
#[derive(Deserialize)]
pub struct LoginRequest {
    pub password: String,
    #[serde(rename = "appKey")]
    pub app_key: Option<String>,
}

/// 解析登录请求体：非法 JSON 或密码为空 → None（调用方回 400）。
pub fn parse_login(body: &[u8]) -> Option<LoginRequest> {
    let req: LoginRequest = serde_json::from_slice(body).ok()?;
    (!req.password.is_empty()).then_some(req)
}

/// /apidoc/auth 的裁决。
pub enum AuthResult {
    /// 未配置任何密码（路由 404，与"鉴权未启用"同义）。
    Disabled,
    /// 密码错误（401）。
    Error,
    Token(String),
}

/// 数据路由守卫失败时的 401 响应体（两适配器共享，1:1 对齐不靠复制保证）。
pub const DENIED_BODY: &str = r#"{"error":"password required"}"#;

/// AuthResult → HTTP 响应（状态码 + body；Disabled 为 404 无 body）。
pub fn auth_result_response(r: AuthResult) -> (u16, String) {
    match r {
        AuthResult::Disabled => (404, String::new()),
        AuthResult::Error => (401, r#"{"error":"password error"}"#.to_string()),
        AuthResult::Token(t) => (200, serde_json::json!({"token": t}).to_string()),
    }
}

// ---------- 登录失败退避（挡在线爆破） ----------

/// 一个凭据的退避状态。
struct FailBucket {
    fails: u64,
    /// 最近一次失败时刻（静默期判定的基准）。
    last: Instant,
    /// 退避截止时刻（None = 未锁定）。
    until: Option<Instant>,
}

/// 退避上限（秒）。全局锁天然可以被用来"挡住别人登录"，上限越小越不容易被当武器；
/// ponytail: 真正按来源限速要在网关做（route_core 拿不到对端 IP）。
const BACKOFF_CAP_SECS: u64 = 10;
/// 静默这么久即视为放弃尝试，计数清零——否则一次误输会让退避在整个进程生命周期里
/// 直接从 1s 起跳（评审实测：计数只在成功时清零，几周后仍从高位起步）。
const DECAY: Duration = Duration::from_secs(120);
/// 前 3 次不罚。
const FREE_FAILS: u64 = 3;

fn buckets() -> &'static Mutex<HashMap<[u8; 16], FailBucket>> {
    static B: OnceLock<Mutex<HashMap<[u8; 16], FailBucket>>> = OnceLock::new();
    B.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 计数键 = 凭据摘要：未知 appKey 会回落到全局凭据，所以换 appKey 骗不到额外免费次数；
/// 用摘要而不是原文，免得把密码留在另一个容器里。
fn bucket_key(credential: &str) -> [u8; 16] {
    let d = Sha256::digest(credential.as_bytes());
    let mut k = [0u8; 16];
    k.copy_from_slice(&d[..16]);
    k
}

/// 前 3 次不罚，之后 2^(n-4) 秒封顶 [`BACKOFF_CAP_SECS`]（第 4 次 1s、第 5 次 2s……）。
fn backoff_secs(fails: u64) -> u64 {
    if fails <= FREE_FAILS {
        0
    } else {
        (1u64 << (fails - FREE_FAILS - 1).min(6)).min(BACKOFF_CAP_SECS)
    }
}

/// 静默期判定（纯函数，便于单测）：距上次失败超过 [`DECAY`] 就把旧账清零。
fn decayed(fails: u64, since_last: Duration) -> u64 {
    if since_last > DECAY { 0 } else { fails }
}

/// 该凭据是否处于退避窗口内（窗口内不比对、不签发，连"密码对不对"都不透露）。
fn locked(credential: &str) -> bool {
    let g = buckets().lock().unwrap_or_else(|e| e.into_inner());
    matches!(g.get(&bucket_key(credential)), Some(b) if b.until.is_some_and(|t| Instant::now() < t))
}

fn record_failure(credential: &str) {
    let now = Instant::now();
    let mut g = buckets().lock().unwrap_or_else(|e| e.into_inner());
    let b = g
        .entry(bucket_key(credential))
        .or_insert_with(|| FailBucket { fails: 0, last: now, until: None });
    b.fails = decayed(b.fails, now.duration_since(b.last)); // 静默期内没再失败，旧账一笔勾销
    b.last = now;
    b.fails += 1;
    let secs = backoff_secs(b.fails);
    if secs > 0 {
        b.until = Some(now + Duration::from_secs(secs));
        warn_lockout(b.fails, secs);
    }
}

/// 成功即清掉该凭据的账（只清它自己：一个应用密码登录成功不该给全局密码的爆破计数清零）。
fn record_success(credential: &str) {
    buckets().lock().unwrap_or_else(|e| e.into_inner()).remove(&bucket_key(credential));
}

// ---------- 凭据判定（唯一来源） ----------

/// 某次请求实际该比对哪条凭据。
pub(crate) enum Credential<'a> {
    /// 可用的凭据（应用独立密码优先，其次全局密码）。
    Found(&'a str),
    /// 配了密码但是空串——失败关闭，不比对也不签发。`Some(app_key)` 表示是应用密码。
    Empty(Option<&'a str>),
    /// 一条凭据都没有 = 鉴权未启用。
    None,
}

/// 签发（`share::token_for`）、登录（`auth_issue`）、守卫（`auth_guard_ok`）三条路径
/// **唯一**的凭据判定来源。历史上三处各写一份，漂移过一次直接造成越权
/// （share 用 appKey 认应用、签发却回落全局）。
pub(crate) fn resolve_credential<'a>(
    app_key: Option<&str>,
    cfg: &'a AuthConfig,
    apps: &'a [AppConfig],
) -> Credential<'a> {
    if let Some(app) = app_key.and_then(|k| crate::find_app(apps, k)) {
        if let Some(pw) = app.password.as_deref() {
            return if pw.is_empty() { Credential::Empty(Some(&app.key)) } else { Credential::Found(pw) };
        }
    }
    if !cfg.enable {
        return Credential::None;
    }
    if cfg.password.is_empty() {
        return Credential::Empty(None);
    }
    Credential::Found(&cfg.password)
}

/// 登录裁决：应用独立密码优先，其次全局密码；空密码失败关闭；退避期内不比对。
pub fn auth_issue(
    password: &str,
    app_key: Option<&str>,
    auth: Option<&AuthConfig>,
    apps: &[AppConfig],
) -> AuthResult {
    let default = AuthConfig::default();
    let cfg = auth.unwrap_or(&default);
    let credential = match resolve_credential(app_key, cfg, apps) {
        Credential::Found(c) => c,
        Credential::Empty(app) => {
            warn_empty_password(app.unwrap_or(""));
            return AuthResult::Error;
        }
        Credential::None => return AuthResult::Disabled,
    };
    if locked(credential) {
        return AuthResult::Error;
    }
    if password_eq(password, credential) {
        record_success(credential);
        AuthResult::Token(auth_token(credential, cfg))
    } else {
        record_failure(credential);
        AuthResult::Error
    }
}

/// 数据路由守卫：true 放行。
/// appKey 指定且该应用配置了独立密码 → 只校验应用令牌；否则全局鉴权未启用即放行。
pub fn auth_guard_ok(
    token: &str,
    app_key: Option<&str>,
    auth: Option<&AuthConfig>,
    apps: &[AppConfig],
) -> bool {
    let default = AuthConfig::default();
    let cfg = auth.unwrap_or(&default);
    match resolve_credential(app_key, cfg, apps) {
        Credential::Found(c) => verify(token, c, cfg),
        // 空密码失败关闭：即使全局没开鉴权也不放行
        Credential::Empty(_) => false,
        Credential::None => true,
    }
}

/// 构建期预热：`secret_key` 缺省时要取一次 OS 熵源。放在这里，故障在启动时暴露，
/// 而不是第一个登录/分享请求上 panic（OnceLock 失败后每次调用都会再 panic）。
pub fn warm_secret_key(cfg: &AuthConfig) {
    let _ = secret_key(cfg);
}

// ---------- 一次性告警（诊断信息不刷屏） ----------

static WARNED: AtomicU32 = AtomicU32::new(0);

/// 每条诊断只报一次；位掩码区分不同告警。
const W_RANDOM_KEY: u32 = 1;
const W_EMPTY_PW: u32 = 2;
const W_LOCKOUT: u32 = 4;

fn warn_once(bit: u32, msg: impl FnOnce()) {
    let old = WARNED.fetch_or(bit, Ordering::Relaxed);
    if old & bit == 0 {
        msg();
    }
}

fn warn_random_key() {
    warn_once(W_RANDOM_KEY, || {
        eprintln!(
            "apidoc: auth is using a random per-process secret_key — every restart invalidates issued tokens; set AuthConfig.secret_key explicitly for stable tokens (and serve over HTTPS)"
        );
    });
}

fn warn_empty_password(app_key: &str) {
    warn_once(W_EMPTY_PW, || {
        if app_key.is_empty() {
            eprintln!("apidoc: auth enabled with an empty password — all logins are rejected until a password is set");
        } else {
            eprintln!("apidoc: app `{app_key}` has an empty password — logins for it are rejected until a password is set");
        }
    });
}

fn warn_lockout(fails: u64, secs: u64) {
    warn_once(W_LOCKOUT, || {
        eprintln!(
            "apidoc: repeated failed logins ({fails}); backing off {secs}s between attempts"
        );
    });
}

/// 清空全部退避状态（测试用）。
pub fn reset_throttle() {
    buckets().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    // 退避阶梯：前 3 次不罚，第 4 次起 1s→2s→4s→…，封顶 BACKOFF_CAP_SECS
    #[test]
    fn backoff_ladder_and_cap() {
        assert_eq!([1, 2, 3].map(backoff_secs), [0, 0, 0]);
        assert_eq!(backoff_secs(4), 1);
        assert_eq!(backoff_secs(5), 2);
        assert_eq!(backoff_secs(6), 4);
        assert_eq!(backoff_secs(7), 8);
        assert_eq!(backoff_secs(8), BACKOFF_CAP_SECS);
        assert_eq!(backoff_secs(50), BACKOFF_CAP_SECS);
        assert_eq!(backoff_secs(u64::MAX), BACKOFF_CAP_SECS, "极大值不得移位溢出");
    }

    // 静默期：超过 DECAY 的旧账清零，窗口内保留
    #[test]
    fn decay_forgives_after_quiet_period() {
        assert_eq!(decayed(7, Duration::from_secs(1)), 7);
        assert_eq!(decayed(7, DECAY), 7, "恰好等于静默期不算超时");
        assert_eq!(decayed(7, DECAY + Duration::from_millis(1)), 0);
        assert_eq!(decayed(0, DECAY * 2), 0);
    }

    // 计数键只看凭据内容：同密码的不同 appKey 共用一格（未知 appKey 回落全局也不算新格子）
    #[test]
    fn bucket_key_is_credential_scoped() {
        assert_eq!(bucket_key("same"), bucket_key("same"));
        assert_ne!(bucket_key("a"), bucket_key("b"));
        assert_eq!(bucket_key("").len(), 16);
    }
}
