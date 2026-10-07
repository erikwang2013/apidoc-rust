//! v2 文档缓存：`api.json` / `export` / `mock` 输出的进程内记忆化。
//!
//! 文档数据在进程生命周期内是静态的（注解在编译期收集），所以"缓存"本质是
//! 记忆化：同一个 key 首次构建后复用，`ttl` 秒后失效重建（`ttl = 0` 表示永久）。
//! 适配器在 `ApidocConfig.cache` 开启时用它包住构建闭包，关掉时完全不走此模块
//! ——保证未启用时输出与 v1 字节级一致。
//!
//! 并发模型：全局一把 `Mutex`，锁只在读写 map 的瞬间持有，`build` 一律在锁外
//! 执行。因此同一 key 并发未命中会重复构建（允许，结果一致则无副作用），但各自
//! 拿到的是自己构建的完整值；`build` 里再调 `memo` 也不会死锁。

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

type Entry = (Instant, String);

fn store() -> &'static Mutex<HashMap<String, Entry>> {
    static S: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取全局锁；锁中毒（持锁线程 panic）时恢复内部数据继续用，绝不 panic。
///
/// 中毒在本模块实际不可达：持锁期间不跑用户代码（`build` 在锁外），键是 `String`
/// （Hash 不会 panic），插入失败是 abort 而非 panic。这里纯属兜底——万一中毒，
/// 缓存继续可用（只是可能带一点脏数据），而不是退化成永久 miss 或直接崩。
fn lock() -> MutexGuard<'static, HashMap<String, Entry>> {
    store().lock().unwrap_or_else(|e| e.into_inner())
}

/// 命中且未过期则返回副本，否则用 `build` 重新生成并写回。
///
/// `ttl_secs == 0` 永久有效；`> 0` 时年龄达到 TTL 即重建。`key` 只是 `String`
/// 键，含 UTF-8 / 特殊字符 / 空串都安全。
pub fn memo(key: &str, ttl_secs: u64, build: impl FnOnce() -> String) -> String {
    // 取锁前采样一次即可：saturating 保证"采样早于写入"（拿完 now 才等到锁）
    // 只算年龄 0（视为刚写入），不会 panic。
    let now = Instant::now();
    {
        let m = lock();
        if let Some((at, val)) = m.get(key) {
            let age = now.saturating_duration_since(*at);
            if ttl_secs == 0 || age < Duration::from_secs(ttl_secs) {
                return val.clone();
            }
        }
    } // 锁在此释放：build 在锁外跑，既不重入死锁，build panic 也不影响其他线程

    let fresh = build();

    // 值整体插入、从不原地改，所以读到的永远是完整值，不存在半写数据
    lock().insert(key.to_string(), (Instant::now(), fresh.clone()));
    fresh
}

/// 清空全部缓存（测试用；也让配置热更新后能立即生效）。
pub fn clear() {
    lock().clear();
}

/// 当前缓存条目数（测试用）。
pub fn len() -> usize {
    lock().len()
}
