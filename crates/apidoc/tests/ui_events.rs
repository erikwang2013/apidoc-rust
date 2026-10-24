// v2 调试事件（前置/后置脚本）：逻辑在 ui.debug.js 内，Rust 侧无可测函数，断言脚本标记与执行点顺序
#![cfg(feature = "axum")]

/// 前置脚本在 fetch 之前执行、后置脚本在响应读取之后执行；两处均经 runScript 统一执行
#[test]
fn ui_debug_has_pre_post_script() {
    let debug = include_str!("../src/ui.debug.js");
    assert!(debug.contains("apidoc_pre_script"), "缺少前置脚本 localStorage 键");
    assert!(debug.contains("apidoc_post_script"), "缺少后置脚本 localStorage 键");
    assert!(debug.contains("new Function("), "缺少脚本执行入口");
    assert!(debug.contains("ctx"), "缺少脚本上下文 ctx");
    assert!(debug.contains("function runScript"), "缺少 runScript 执行函数");
    assert!(debug.contains("Object.assign"), "缺少返回值浅合并回 ctx");
    let pre_at = debug.find("runScript(preTa.value").expect("缺少前置脚本执行点");
    let fetch_at = debug.find("await fetch(fullUrl").expect("缺少 fetch 调用");
    let post_at = debug.find("runScript(postTa.value").expect("缺少后置脚本执行点");
    assert!(pre_at < fetch_at, "前置脚本必须在 fetch 之前执行");
    assert!(fetch_at < post_at, "后置脚本必须在 fetch 之后执行");
    // 抛错不中断：两处错误提示 + 继续正常流程
    assert!(debug.matches("warnBox(").count() >= 3, "脚本错误提示未接入结果区");
}

/// 红线（与 axum_m2_routes / actix_m2_routes 同款）：全 UI 拼接串禁用 innerHTML
#[test]
fn ui_events_no_innerhtml() {
    let ui = concat!(
        include_str!("../src/ui.html"),
        include_str!("../src/ui.js"),
        include_str!("../src/ui.debug.js")
    );
    assert!(!ui.contains("innerHTML"), "禁用 innerHTML");
    assert!(ui.contains("debug-script"), "缺少调试事件样式/节点标记");
}

/// P1：流式面板框架与 WebSocket 适配器。
#[test]
fn ui_has_stream_panel_and_ws_adapter() {
    let ui = include_str!("../src/ui.debug.stream.js");
    assert!(ui.contains("function streamPanel"));
    assert!(ui.contains("MAX_FRAMES = 1000"));
    assert!(ui.contains("registerStreamAdapter"));
    assert!(ui.contains("'websocket'"));
    assert!(!ui.contains("innerHTML"));
}

/// P1：所有浏览器原生协议都有分发入口。
#[test]
fn ui_dispatches_all_browser_protocol_panels() {
    let ui = include_str!("../src/ui.debug.js");
    for marker in [
        "streamPanel(ep, 'sse')",
        "streamPanel(ep, 'socketio')",
        "streamPanel(ep, 'stomp')",
        "'graphql-ws'",
        "webrtcPanel(ep)",
        "hlsPanel(ep)",
        "P2+",
    ] {
        assert!(ui.contains(marker), "missing dispatch marker: {marker}");
    }
}

/// P1：HLS 预览面板与分发。
#[test]
fn ui_has_hls_panel() {
    let ui = include_str!("../src/ui.debug.media.js");
    assert!(ui.contains("function hlsPanel"));
    assert!(ui.contains("createElement('video')"));
    let debug = include_str!("../src/ui.debug.js");
    assert!(debug.contains("hlsPanel(ep)"));
}

/// P1：WebRTC / DataChannel 面板。
#[test]
fn ui_has_webrtc_panel() {
    let ui = include_str!("../src/ui.debug.media.js");
    assert!(ui.contains("function webrtcPanel"));
    assert!(ui.contains("RTCPeerConnection"));
    assert!(ui.contains("createDataChannel"));
    assert!(!ui.contains("innerHTML"));
}

/// P1：SSE 适配器与断开能力。
#[test]
fn ui_has_sse_adapter_and_abort() {
    let ui = include_str!("../src/ui.debug.stream.js");
    assert!(ui.contains("'sse'"));
    assert!(ui.contains("AbortController"));
    assert!(ui.contains("sseDecode"));
}

/// P1：HTTP 族协议预设入口。
#[test]
fn ui_has_http_protocol_presets() {
    let ui = include_str!("../src/ui.protocols.js");
    assert!(ui.contains("function protocolExtras"));
    assert!(ui.contains("buildGraphqlBody"));
    assert!(ui.contains("buildJsonRpcBody"));
    assert!(ui.contains("buildODataQuery"));
    assert!(!ui.contains("innerHTML"));
}

/// P0：协议徽标、HTTP 族表单分发、服务端代发开关与原始 Body 编码入口。
#[test]
fn ui_has_protocol_and_server_debug_hooks() {
    let ui = concat!(
        include_str!("../src/ui.html"),
        include_str!("../src/ui.js"),
        include_str!("../src/ui.debug.js")
    );
    assert!(ui.contains("protocolBadge"), "缺少协议徽标");
    assert!(ui.contains("HTTP_FAMILY"), "缺少 HTTP 族表单分发");
    assert!(ui.contains("enable_server_side"), "缺少服务端代发开关判定");
    assert!(ui.contains("base + 'debug'"), "缺少 /apidoc/debug 调用点");
    assert!(ui.contains("decodeBytes"), "缺少 hex/base64 body 解码");
    assert!(ui.contains("原始 Body"), "缺少原始 Body 编辑器");
    assert!(ui.contains("协议选项"), "缺少服务端协议的 opts 编辑器");
    assert!(ui.contains("protocol_opts.target"), "缺少 target 预填");
    assert!(ui.contains("endpoint: { url: ep.url"), "缺少 endpoint 引用");
}
