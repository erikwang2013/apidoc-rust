//! UIC-1 share records: file-backed JSON store + scoped access.
//!
//! Records live in `$APIDOC_SHARE_DIR/shares.json` (default
//! `<tmp>/apidoc-rust-shares/shares.json`). Writes are atomic (temp file +
//! rename) and serialized by a process-wide mutex; the file is 0600 on unix.
//! `password_hash` and `secret` stay on disk only — response mapping goes
//! through [`record_json`], which never emits them.
//!
//! A share token is an ordinary auth token signed with the record's random
//! secret (`auth::auth_token`), so it can never pass an app/global guard.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth;

/// 列表分页大小：UI 只传 pageIndex，PHP/Go/UI 每页均为 5。
const PAGE_SIZE: usize = 5;
/// 单个范围列表的条目上限：分享记录是小型管理数据，超过即为配置错误。
const MAX_SCOPE_ITEMS: usize = 500;
const MAX_SCOPE_KEY_LEN: usize = 256;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Record {
    pub key: String,
    pub name: String,
    #[serde(rename = "type", default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub app_keys: Vec<String>,
    #[serde(default)]
    pub api_keys: Vec<String>,
    /// SHA-256 hex of the share password; empty = public link share.
    #[serde(default)]
    pub password_hash: String,
    /// HMAC credential for share tokens issued by `verifyAuth`.
    #[serde(default)]
    pub secret: String,
    /// Unix seconds; the API exposes both `create_time` and a UTC `create_at`.
    #[serde(default)]
    pub created: u64,
}

pub(crate) fn default_kind() -> String {
    "all".to_string()
}

#[derive(Debug)]
pub enum Error {
    NotFound,
    Invalid(&'static str),
    Store,
}

impl Record {
    /// 应用级可见性：app 类型必须在 appKeys 内；api 类型若填了 appKeys 也须命中
    /// （UI 为 api 分享同时提交 appKeys，防止跨应用遍历）。
    pub fn app_allowed(&self, app_key: &str) -> bool {
        match self.kind.as_str() {
            "all" => true,
            "app" => self.app_keys.iter().any(|k| k == app_key),
            "api" => self.app_keys.is_empty() || self.app_keys.iter().any(|k| k == app_key),
            _ => false,
        }
    }

    /// 接口级可见性：api 类型只放行 apiKeys 内的稳定 menuKey。
    pub fn api_allowed(&self, app_key: &str, menu_key: &str) -> bool {
        match self.kind.as_str() {
            "all" => true,
            "app" => self.app_allowed(app_key),
            "api" => {
                self.app_allowed(app_key) && self.api_keys.iter().any(|k| k == menu_key)
            }
            _ => false,
        }
    }

    pub fn password_ok(&self, password: &str) -> bool {
        if self.password_hash.is_empty() {
            return true; // 无密码分享：shareKey 本身就是凭据
        }
        auth::ct_eq(hash_hex(password).as_bytes(), self.password_hash.as_bytes())
    }
}

pub fn check_token(record: &Record, token: &str, cfg: &auth::AuthConfig) -> bool {
    !record.secret.is_empty() && auth::auth_check_app(token, &record.secret, cfg)
}

pub fn issue_token(record: &Record, cfg: &auth::AuthConfig) -> String {
    auth::auth_token(&record.secret, cfg)
}

pub fn find(key: &str) -> Result<Record, Error> {
    if !valid_key(key) {
        return Err(Error::NotFound);
    }
    with_store(|records| {
        Ok((records.iter().find(|r| r.key == key).cloned(), false))
    })?
    .ok_or(Error::NotFound)
}

/// `page_index` 从 1 开始（UI 语义）；记录按创建时间倒序。
pub fn list(page_index: u64) -> Result<(Vec<Record>, usize), Error> {
    let mut records = with_store(|records| Ok((records.clone(), false)))?;
    records.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.key.cmp(&b.key)));
    let total = records.len();
    let start = page_index.max(1).saturating_sub(1).saturating_mul(PAGE_SIZE as u64);
    let data = records
        .into_iter()
        .skip(start.try_into().unwrap_or(usize::MAX))
        .take(PAGE_SIZE)
        .collect();
    Ok((data, total))
}

pub struct NewShare {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub app_keys: Vec<String>,
    pub api_keys: Vec<String>,
    pub password: Option<String>,
}

/// 新增或编辑（带 key 即编辑：保留 secret/created，未传密码则保留原密码）。
pub fn add(input: &NewShare) -> Result<(), Error> {
    if input.name.trim().is_empty() || input.name.len() > 200 {
        return Err(Error::Invalid("invalid share name"));
    }
    if !matches!(input.kind.as_str(), "all" | "app" | "api") {
        return Err(Error::Invalid("invalid share type"));
    }
    if !input.key.is_empty() && !valid_key(&input.key) {
        return Err(Error::Invalid("invalid share key"));
    }
    for scope in [&input.app_keys, &input.api_keys] {
        if scope.len() > MAX_SCOPE_ITEMS || scope.iter().any(|k| k.len() > MAX_SCOPE_KEY_LEN) {
            return Err(Error::Invalid("invalid share scope"));
        }
    }
    let password = input
        .password
        .as_deref()
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    if password.as_ref().is_some_and(|p| p.len() > 1024) {
        return Err(Error::Invalid("invalid share password"));
    }

    with_store(|records| {
        if let Some(record) = records.iter_mut().find(|r| !input.key.is_empty() && r.key == input.key) {
            record.name = input.name.clone();
            record.kind = input.kind.clone();
            record.app_keys = input.app_keys.clone();
            record.api_keys = input.api_keys.clone();
            if let Some(password) = password.as_deref() {
                record.password_hash = hash_hex(password);
            }
            return Ok(((), true));
        }
        let key = if input.key.is_empty() {
            unique_key(records)?
        } else {
            input.key.clone()
        };
        records.push(Record {
            key,
            name: input.name.clone(),
            kind: input.kind.clone(),
            app_keys: input.app_keys.clone(),
            api_keys: input.api_keys.clone(),
            password_hash: password.as_deref().map(hash_hex).unwrap_or_default(),
            secret: rand_hex(32)?,
            created: now(),
        });
        Ok(((), true))
    })
}

pub fn delete(key: &str) -> Result<(), Error> {
    if !valid_key(key) {
        return Ok(());
    }
    with_store(|records| {
        let before = records.len();
        records.retain(|r| r.key != key);
        Ok(((), records.len() != before))
    })
}

/// API 映射：password_hash/secret 永不出现。
pub fn record_json(record: &Record) -> serde_json::Value {
    let created = fmt_time(record.created);
    let mut value = serde_json::json!({
        "key": record.key,
        "name": record.name,
        "type": record.kind,
        "create_at": created,
        "create_time": created,
    });
    if !record.app_keys.is_empty() {
        value["appKeys"] = serde_json::json!(record.app_keys);
    }
    if !record.api_keys.is_empty() {
        value["apiKeys"] = serde_json::json!(record.api_keys);
    }
    value
}

pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn hash_hex(password: &str) -> String {
    hex(&Sha256::digest(password.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn rand_hex(bytes: usize) -> Result<String, Error> {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).map_err(|_| Error::Store)?;
    Ok(hex(&buf))
}

fn unique_key(records: &[Record]) -> Result<String, Error> {
    for _ in 0..8 {
        let key = rand_hex(16)?;
        if records.iter().all(|r| r.key != key) {
            return Ok(key);
        }
    }
    Err(Error::Store)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UTC `YYYY-MM-DD HH:MM:SS`（不引入 chrono；create_at 只用于展示）。
fn fmt_time(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        tod / 3_600,
        (tod % 3_600) / 60,
        tod % 60
    )
}

fn lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn store_path() -> PathBuf {
    let dir = std::env::var_os("APIDOC_SHARE_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("apidoc-rust-shares"));
    dir.join("shares.json")
}

/// 在进程内互斥下读-改-写；`dirty` 为 true 时原子落盘。
fn with_store<T>(
    f: impl FnOnce(&mut Vec<Record>) -> Result<(T, bool), Error>,
) -> Result<T, Error> {
    let _guard = lock().lock().unwrap_or_else(|e| e.into_inner());
    let path = store_path();
    let mut records = read_records(&path)?;
    let (value, dirty) = f(&mut records)?;
    if dirty {
        write_atomic(&path, &records)?;
    }
    Ok(value)
}

fn read_records(path: &Path) -> Result<Vec<Record>, Error> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            // 损坏时不静默清空：下一次写会把旧记录永久覆盖。
            eprintln!("apidoc: share store {} is invalid JSON: {e}", path.display());
            Error::Store
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => {
            eprintln!("apidoc: cannot read share store {}: {e}", path.display());
            Err(Error::Store)
        }
    }
}

fn write_atomic(path: &Path, records: &[Record]) -> Result<(), Error> {
    let parent = path.parent().ok_or(Error::Store)?;
    std::fs::create_dir_all(parent).map_err(|_| Error::Store)?;
    let body = serde_json::to_vec_pretty(records).map_err(|_| Error::Store)?;
    // 进程 id 后缀：跨进程并发写不会互相截断临时文件。
    let tmp = parent.join(format!(".shares.json.tmp.{}", std::process::id()));
    {
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|_| Error::Store)?
        };
        #[cfg(not(unix))]
        let mut file = std::fs::File::create(&tmp).map_err(|_| Error::Store)?;
        file.write_all(&body).map_err(|_| Error::Store)?;
        file.sync_all().map_err(|_| Error::Store)?;
    }
    std::fs::rename(&tmp, path).map_err(|_| Error::Store)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: &str) -> Record {
        Record {
            key: "share-1".into(),
            kind: kind.into(),
            app_keys: vec!["A".into(), "B".into()],
            api_keys: vec!["api-a1".into()],
            secret: "s3cr3t".into(),
            ..Default::default()
        }
    }

    #[test]
    fn key_sanitization() {
        assert!(valid_key("share-1"));
        assert!(valid_key("AbC09"));
        assert!(!valid_key(""));
        assert!(!valid_key("has space"));
        assert!(!valid_key("under_score"));
        assert!(!valid_key(&"x".repeat(65)));
    }

    #[test]
    fn password_hash_is_salted_by_nothing_but_never_plaintext() {
        let hash = hash_hex("secret");
        assert_eq!(hash.len(), 64);
        assert_ne!(hash, "secret");
        let mut r = record("all");
        r.password_hash = hash;
        assert!(r.password_ok("secret"));
        assert!(!r.password_ok("Secret"));
        assert!(!r.password_ok(""));
        // 无密码分享：shareKey 本身即凭据
        r.password_hash.clear();
        assert!(r.password_ok(""));
    }

    #[test]
    fn scope_semantics() {
        let all = record("all");
        assert!(all.app_allowed("C"));
        assert!(all.api_allowed("C", "anything"));

        let app = record("app");
        assert!(app.app_allowed("A"));
        assert!(!app.app_allowed("C"));
        assert!(app.api_allowed("B", "any-api"));
        assert!(!app.api_allowed("C", "api-a1"));

        let api = record("api");
        assert!(api.api_allowed("A", "api-a1"));
        assert!(!api.api_allowed("A", "api-a2"));
        let mut scoped = api.clone();
        scoped.app_keys = vec!["A".into()];
        assert!(!scoped.app_allowed("B"));
        assert!(!scoped.api_allowed("B", "api-a1"));

        let unknown = record("bogus");
        assert!(!unknown.app_allowed("A"));
        assert!(!unknown.api_allowed("A", "api-a1"));
    }

    #[test]
    fn api_json_never_leaks_secrets() {
        let mut r = record("app");
        r.password_hash = hash_hex("pw");
        let value = record_json(&r);
        assert_eq!(value["type"], "app");
        assert_eq!(value["appKeys"][0], "A");
        assert!(value.get("password_hash").is_none());
        assert!(value.get("passwordHash").is_none());
        assert!(value.get("secret").is_none());
    }

    #[test]
    fn share_token_is_bound_to_record_secret() {
        let cfg = auth::AuthConfig { secret_key: "k".into(), ..Default::default() };
        let r = record("all");
        let token = issue_token(&r, &cfg);
        assert!(check_token(&r, &token, &cfg));
        let other = Record { secret: "other".into(), ..r.clone() };
        assert!(!check_token(&other, &token, &cfg));
        assert!(!check_token(&r, "", &cfg));
    }

    #[test]
    fn create_at_is_utc() {
        assert_eq!(fmt_time(0), "1970-01-01 00:00:00");
        assert_eq!(fmt_time(1_700_000_000), "2023-11-14 22:13:20");
        assert_eq!(fmt_time(1_790_000_000), "2026-09-21 14:13:20");
    }
}
