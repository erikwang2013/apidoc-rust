//! 适配器集成测试共享脚手架：axum / actix 两侧的断言体只此一份，driver 只留
//! 「建 app + 取响应」的 get。
//!
//! 状态码/头特意归一化成基础类型：axum 走 http 1.x 的 StatusCode/HeaderMap，
//! actix-web 走 http 0.2 的，两者类型不同，共享层不做类型抽象——比较值
//! （200/404/400/401、头名、头值）与原断言逐字一致。
#![allow(dead_code)] // 同一份共享模块被 8 个测试二进制各自编译，单文件未用到的辅助函数属常态

/// 归一化响应：status 为数字，header 名统一小写，body 为 UTF-8 文本。
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Resp {
    /// header 名统一小写、值 UTF-8 lossy（仅 ASCII 参与断言，lossy 只为不 panic）。
    pub fn new(status: u16, headers: impl IntoIterator<Item = (String, String)>, body: String) -> Self {
        Self {
            status,
            headers: headers.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect(),
            body,
        }
    }

    /// 按小写名取头值（对比 `headers.get(header::CONTENT_TYPE)`）。
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// token 含 base64 的 + / 字符，作为 query 传值须百分号编码（同 ui.js 的 encodeURIComponent）。
pub fn enc(s: &str) -> String {
    s.replace('+', "%2B").replace('/', "%2F")
}

/// 解析 share 响应的 {"url": ...}。
pub fn link_of(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body).expect("share 响应必须是合法 JSON")["url"]
        .as_str()
        .expect("share 响应的 url 必须是字符串")
        .to_string()
}

/// 取 share 深链里的 token 参数原样编码值（build_url 保证 token 是最后一个参数）。
/// 原样回灌请求 URL 即为「链接开箱可用」的端到端验证，不需要再解码/重编码。
pub fn link_token(link: &str) -> String {
    link.split("token=")
        .nth(1)
        .unwrap_or_else(|| panic!("分享链接缺少 token 参数: {link}"))
        .to_string()
}
