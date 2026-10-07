//! `cache` 模块测试。
//!
//! 缓存是进程内全局单例，而 cargo 默认多线程跑测试，所以每个测试开头都取一次
//! `SERIAL` 串行化，并用各自独立的 key 前缀——否则 `clear()` / `len()` 会互相踩，
//! 断言 build 次数的测试会 flaky。

use apidoc::cache;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// 串行化本文件的测试；中毒也恢复（某个测试 panic 不该连累后面的测试）。
fn serial() -> MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

/// 未命中调 build，命中后不再调 build。
#[test]
fn miss_builds_then_hit_does_not_rebuild() {
    let _g = serial();
    let calls = AtomicUsize::new(0);
    let count = || calls.fetch_add(1, Ordering::SeqCst);

    let first = cache::memo("t1:key", 3600, || {
        count();
        "first".to_string()
    });
    assert_eq!(first, "first");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // 同一个 key 再取：返回缓存值，build 一次都不许跑
    let second = cache::memo("t1:key", 3600, || {
        count();
        "second".to_string()
    });
    assert_eq!(second, "first");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// TTL：期限内命中，超过后重建。
///
/// 取舍：真睡满 TTL 会拖慢这套测试（本测试 ~1.1s），但只有它覆盖"年龄 >= TTL"
/// 这个分支；换成 `clear()` 只等价于"未命中"，测不到时间判断。用整秒 TTL + 300ms
/// 余量，避开毫秒级抖动。
#[test]
fn ttl_expiry_rebuilds() {
    let _g = serial();
    let calls = AtomicUsize::new(0);
    let build = || {
        calls.fetch_add(1, Ordering::SeqCst);
        "fresh".to_string()
    };

    assert_eq!(cache::memo("t2:key", 1, build), "fresh");

    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(cache::memo("t2:key", 1, build), "fresh"); // TTL 内仍命中
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    std::thread::sleep(Duration::from_millis(800)); // 累计 ~1.1s > 1s
    assert_eq!(cache::memo("t2:key", 1, build), "fresh"); // 过期重建
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// ttl = 0 永久有效：睡过任何有限 TTL 都还命中。
#[test]
fn ttl_zero_never_expires() {
    let _g = serial();
    let calls = AtomicUsize::new(0);
    let build = || {
        calls.fetch_add(1, Ordering::SeqCst);
        "permanent".to_string()
    };

    assert_eq!(cache::memo("t3:key", 0, build), "permanent");
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(cache::memo("t3:key", 0, build), "permanent");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// clear() 后重建；len() 计数正确（命中不新增条目）。
#[test]
fn clear_resets_and_len_counts() {
    let _g = serial();
    cache::clear();
    assert_eq!(cache::len(), 0);

    cache::memo("t4:a", 0, || "a".to_string());
    cache::memo("t4:b", 0, || "b".to_string());
    assert_eq!(cache::len(), 2);

    cache::memo("t4:a", 0, || "should-not-run".to_string()); // 命中，不新增
    assert_eq!(cache::len(), 2);

    cache::clear();
    assert_eq!(cache::len(), 0);

    let calls = AtomicUsize::new(0);
    let v = cache::memo("t4:a", 0, || {
        calls.fetch_add(1, Ordering::SeqCst);
        "rebuilt".to_string()
    });
    assert_eq!(v, "rebuilt");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

/// 8 线程抢同一 key：不 panic、各自拿到完整值、最终缓存值一致。
#[test]
fn concurrent_same_key_is_safe() {
    let _g = serial();
    const N: usize = 8;
    let key = "t5:contended";
    let payload = "x".repeat(64);
    let expected = format!("{key}:{payload}");

    let barrier = std::sync::Barrier::new(N);
    let got: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..N)
            .map(|_| {
                s.spawn(|| {
                    barrier.wait(); // 尽量同时进 memo
                    cache::memo(key, 0, || format!("{key}:{}", "x".repeat(64)))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    assert_eq!(got.len(), N);
    for v in &got {
        assert_eq!(v, &expected); // 没有半写数据
    }

    // 最终缓存的就是这个值：build 里 panic 也拿得到（说明确实命中而非重建）
    let final_v = cache::memo(key, 0, || panic!("应命中缓存，不该重建"));
    assert_eq!(final_v, expected);
}

/// build 抛 panic：panic 照常传播，缓存的后续使用不受影响（不返回错值、不死锁）。
#[test]
fn build_panic_leaves_cache_usable() {
    let _g = serial();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // 静音这条预期 panic 的输出
    let result = std::panic::catch_unwind(|| cache::memo("t6:key", 0, || panic!("boom")));
    std::panic::set_hook(hook);
    assert!(result.is_err(), "build 的 panic 应当传播给调用方");

    // 失败的结果没被缓存，之后照常用
    assert_eq!(cache::memo("t6:key", 0, || "ok".to_string()), "ok");
    assert_eq!(cache::memo("t6:key", 0, || "ignored".to_string()), "ok");
}

/// key 含特殊字符 / UTF-8 / 空串都不炸，且互不串味。
#[test]
fn exotic_keys_work() {
    let _g = serial();
    let keys = ["", "含中文/键", "quote\"back\\slash", "nul\u{0}byte", "emoji-🐾"];

    for (i, k) in keys.iter().enumerate() {
        let expected = format!("v{i}");
        assert_eq!(cache::memo(k, 0, || expected.clone()), expected);

        let hit = cache::memo(k, 0, || panic!("应命中缓存: {k:?}"));
        assert_eq!(hit, expected);
    }
}
