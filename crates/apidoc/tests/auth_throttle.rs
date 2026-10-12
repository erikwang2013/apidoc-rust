//! 登录失败退避（`auth::auth_issue` 的进程内全局计数器）。
//!
//! 单独一个测试二进制 + 单个测试函数：退避状态是进程内全局的，与 m6_auth.rs
//! 那些"错一次密码"的用例同进程会互相影响，本文件内多个测试并发也会互相
//! 清零/上锁（踩过：两个 #[test] 并行时 reset 会把对方的计数抹掉）。

use apidoc::auth::{auth_issue, reset_throttle, AuthConfig, AuthResult};
use apidoc::AppConfig;

const PW: &str = "secret";

fn cfg() -> AuthConfig {
    AuthConfig { enable: true, password: PW.into(), secret_key: "k".into(), expire: 0 }
}

#[test]
fn failed_logins_back_off_and_recover() {
    reset_throttle();
    let c = cfg();

    // ① 前 3 次失败不罚：每次都立刻返回 Error
    for i in 0..3 {
        assert!(
            matches!(auth_issue("bad", None, Some(&c), &[]), AuthResult::Error),
            "第 {} 次失败应为 Error",
            i + 1
        );
    }
    // ② 第 4 次失败触发 1s 退避；退避期内连"密码正确"也不签发（不透露密码对错）
    assert!(matches!(auth_issue("bad", None, Some(&c), &[]), AuthResult::Error));
    assert!(
        matches!(auth_issue(PW, None, Some(&c), &[]), AuthResult::Error),
        "退避期内即使密码正确也不得签发"
    );

    // ③ 退避窗口结束后正确密码恢复签发
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(
        matches!(auth_issue(PW, None, Some(&c), &[]), AuthResult::Token(_)),
        "退避结束后正确密码应恢复签发"
    );

    // ④ 成功即清零：紧接着的错误密码不把实例重新锁住
    assert!(matches!(auth_issue("bad", None, Some(&c), &[]), AuthResult::Error));
    assert!(
        matches!(auth_issue(PW, None, Some(&c), &[]), AuthResult::Token(_)),
        "成功后计数清零，应立即恢复"
    );

    // ⑤ 未配置密码的失败关闭分支不进退避计数：用它探测不会把实例锁死。
    //    （未知 app key 走的是全局凭据回退，属于真实登录失败，会计数。）
    reset_throttle();
    let empty = AuthConfig { enable: true, password: String::new(), secret_key: "k".into(), expire: 0 };
    for _ in 0..5 {
        assert!(matches!(auth_issue("x", None, Some(&empty), &[]), AuthResult::Error));
    }
    assert!(
        matches!(auth_issue(PW, None, Some(&c), &[]), AuthResult::Token(_)),
        "空密码分支不应累积退避"
    );

    // ⑥ 退避按凭据分桶（回归：全局共用一个计数器时，应用密码持有者登录一次
    //    就能给全局密码的爆破计数清零，等于给全局爆破开了无限次免费尝试）。
    reset_throttle();
    let apps = vec![AppConfig {
        key: "api".into(),
        title: "API".into(),
        items: Vec::new(),
        password: Some("apppass".into()),
    }];
    for _ in 0..3 {
        assert!(matches!(auth_issue("x", None, Some(&c), &apps), AuthResult::Error));
    }
    assert!(matches!(auth_issue("x", Some("api"), Some(&c), &apps), AuthResult::Error));
    // 全局第 4 次失败 → 全局凭据进入 1s 退避
    assert!(matches!(auth_issue("x", None, Some(&c), &apps), AuthResult::Error));
    assert!(
        matches!(auth_issue(PW, None, Some(&c), &apps), AuthResult::Error),
        "全局凭据应已进入退避"
    );
    // 应用凭据只错了 1 次，不受全局退避牵连
    assert!(
        matches!(auth_issue("apppass", Some("api"), Some(&c), &apps), AuthResult::Token(_)),
        "应用凭据不应被全局凭据的退避牵连"
    );
    // 而应用登录成功也不得清掉全局的账
    assert!(
        matches!(auth_issue(PW, None, Some(&c), &apps), AuthResult::Error),
        "应用登录成功不得清掉全局凭据的退避"
    );
    reset_throttle();
}
