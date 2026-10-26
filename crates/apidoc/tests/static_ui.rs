//! UIC-1 Phase 5: `ui = UiMode::Vue` static UI serving.
//! 两个适配器都只做路由接线，读取/路径收敛在 route_core（同一份实现）。

use apidoc::{ApidocConfig, UiMode};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ui")
}

fn vue_config(path: PathBuf) -> ApidocConfig {
    ApidocConfig {
        title: "t".into(),
        ui: UiMode::Vue,
        ui_path: Some(path),
        ..Default::default()
    }
}

#[test]
fn release_config_uses_vue_and_stays_out_of_api_json() {
    assert_eq!(ApidocConfig::default().ui, UiMode::Native);
    let cfg = ApidocConfig::release("t", "ui-build");
    assert_eq!(cfg.ui, UiMode::Vue);
    assert_eq!(
        cfg.ui_path.as_deref(),
        Some(std::path::Path::new("ui-build"))
    );
    let json = serde_json::to_value(&cfg).unwrap();
    assert!(
        json.get("ui").is_none() && json.get("ui_path").is_none(),
        "{json}"
    );
}

#[cfg(feature = "axum")]
fn build(config: ApidocConfig) {
    let _ = apidoc::axum::apidoc_routes(config);
}

#[cfg(all(feature = "actix", not(feature = "axum")))]
fn build(config: ApidocConfig) {
    let _ = apidoc::actix::apidoc_routes(config);
}

#[cfg(any(feature = "axum", feature = "actix"))]
#[test]
#[should_panic(expected = "ui_path")]
fn missing_ui_path_fails_closed() {
    build(vue_config(PathBuf::from("/nonexistent/apidoc-ui-build")));
}

#[cfg(any(feature = "axum", feature = "actix"))]
#[test]
#[should_panic(expected = "requires ui_path")]
fn vue_without_ui_path_fails_closed() {
    build(ApidocConfig {
        ui: UiMode::Vue,
        ..Default::default()
    });
}

#[cfg(feature = "axum")]
mod axum_ui {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Option<String>, Vec<u8>) {
        let res = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let ct = res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (status, ct, body)
    }

    #[tokio::test]
    async fn native_is_default_and_serves_embedded_ui() {
        let app = apidoc::axum::apidoc_routes(ApidocConfig {
            title: "t".into(),
            ..Default::default()
        });
        let (status, ct, body) = get(&app, "/apidoc").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ct.as_deref(), Some("text/html; charset=utf-8"));
        assert!(String::from_utf8(body)
            .unwrap()
            .contains("<title>API Documentation</title>"));
        // 原生模式不注册 Vue 路由，`/apidoc/` 维持原来的 404。
        assert_eq!(get(&app, "/apidoc/").await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn vue_redirects_root_and_serves_index_assets_config_and_spa_fallback() {
        let app = apidoc::axum::apidoc_routes(vue_config(fixture()));
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/apidoc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(res.headers()[header::LOCATION], "/apidoc/");

        for (uri, ct, marker) in [
            ("/apidoc/", "text/html; charset=utf-8", "UI fixture index"),
            (
                "/apidoc/config.js",
                "text/javascript; charset=utf-8",
                "apidocFeConfig",
            ),
            (
                "/apidoc/assets/app.js",
                "text/javascript; charset=utf-8",
                "fixture asset",
            ),
            ("/apidoc/assets/app.css", "text/css; charset=utf-8", "#app"),
            (
                "/apidoc/deep/link",
                "text/html; charset=utf-8",
                "UI fixture index",
            ),
        ] {
            let (status, got_ct, body) = get(&app, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(got_ct.as_deref(), Some(ct), "{uri}");
            assert!(String::from_utf8_lossy(&body).contains(marker), "{uri}");
        }
        // 缺资源不能被 SPA 壳掩盖。
        assert_eq!(
            get(&app, "/apidoc/assets/missing.js").await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn api_routes_take_precedence_over_spa() {
        let app = apidoc::axum::apidoc_routes(vue_config(fixture()));
        let (status, ct, body) = get(&app, "/apidoc/api.json").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ct.as_deref(), Some("application/json"));
        assert!(String::from_utf8_lossy(&body).contains("\"endpoints\""));

        let (status, ct, body) = get(&app, "/apidoc/ui/v1/capabilities").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ct.as_deref(), Some("application/json"));
        assert!(String::from_utf8_lossy(&body).contains("\"contract\""));

        // 未声明的 API 子路径保持 404，不落到 SPA 壳。
        assert_eq!(
            get(&app, "/apidoc/ui/v1/nope").await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn traversal_is_blocked() {
        let app = apidoc::axum::apidoc_routes(vue_config(fixture()));
        for uri in [
            "/apidoc/../ui-secret.txt",
            "/apidoc/%2e%2e/ui-secret.txt",
            "/apidoc/..%2fui-secret.txt",
            "/apidoc/assets/../../ui-secret.txt",
        ] {
            let (status, _, body) = get(&app, uri).await;
            assert_ne!(status, StatusCode::OK, "{uri}");
            assert!(
                !String::from_utf8_lossy(&body).contains("TOP-SECRET"),
                "{uri}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_escape_is_blocked() {
        let dir = std::env::temp_dir().join(format!("apidoc-ui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(fixture().join("index.html"), dir.join("index.html")).unwrap();
        std::os::unix::fs::symlink(fixture().join("../ui-secret.txt"), dir.join("escape.txt"))
            .unwrap();

        let app = apidoc::axum::apidoc_routes(vue_config(dir.clone()));
        let (status, _, body) = get(&app, "/apidoc/escape.txt").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!String::from_utf8_lossy(&body).contains("TOP-SECRET"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 5.3 包级 e2e：UIC1_UI_PACKAGE 指向真实 dist/uic1-ui 时运行，否则跳过。
    #[tokio::test]
    async fn packaged_ui_is_served_when_configured() {
        let Some(pkg) = std::env::var_os("UIC1_UI_PACKAGE") else {
            eprintln!("skipped: UIC1_UI_PACKAGE is not set");
            return;
        };
        let app = apidoc::axum::apidoc_routes(vue_config(PathBuf::from(pkg)));

        let (status, ct, body) = get(&app, "/apidoc/").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ct.as_deref(), Some("text/html; charset=utf-8"));
        let index = String::from_utf8(body).unwrap();
        assert!(index.contains("<div id=\"app\">"), "{index}");

        let (status, ct, body) = get(&app, "/apidoc/config.js").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ct.as_deref(), Some("text/javascript; charset=utf-8"));
        assert!(String::from_utf8_lossy(&body).contains("apidocFeConfig"));

        // index.html 里引用的第一个 Vite 资源，文件名带内容哈希。
        let asset = index
            .split("assets/")
            .nth(1)
            .and_then(|rest| rest.split(['"', '\'']).next())
            .filter(|name| name.contains('-'))
            .expect("index.html references a hashed asset");
        let (status, ct, body) = get(&app, &format!("/apidoc/assets/{asset}")).await;
        assert_eq!(status, StatusCode::OK, "{asset}");
        assert!(!body.is_empty(), "{asset}");
        assert_ne!(ct.as_deref(), Some("text/html; charset=utf-8"), "{asset}");

        let (status, ct, body) = get(&app, "/apidoc/manifest.json").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ct.as_deref(), Some("application/json"));
        let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(manifest["name"], "uic1-ui");
    }
}

#[cfg(feature = "actix")]
mod actix_ui {
    use super::*;
    use actix_web::http::header;
    use actix_web::{test as actix_test, App};

    /// (status, content-type, location, body)
    async fn get(
        config: ApidocConfig,
        uri: &str,
    ) -> (u16, Option<String>, Option<String>, Vec<u8>) {
        let app =
            actix_test::init_service(App::new().service(apidoc::actix::apidoc_routes(config)))
                .await;
        let req = actix_test::TestRequest::get().uri(uri).to_request();
        let res = actix_test::call_service(&app, req).await;
        let status = res.status().as_u16();
        let hdr = |name| {
            res.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        (
            status,
            hdr(header::CONTENT_TYPE),
            hdr(header::LOCATION),
            actix_test::read_body(res).await.to_vec(),
        )
    }

    #[actix_web::test]
    async fn native_is_default_and_serves_embedded_ui() {
        let (status, ct, _, body) = get(
            ApidocConfig {
                title: "t".into(),
                ..Default::default()
            },
            "/apidoc",
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(ct.as_deref(), Some("text/html; charset=utf-8"));
        assert!(String::from_utf8(body)
            .unwrap()
            .contains("<title>API Documentation</title>"));
        let (status, ..) = get(ApidocConfig::default(), "/apidoc/").await;
        assert_eq!(status, 404);
    }

    #[actix_web::test]
    async fn vue_redirects_root_and_serves_index_assets_config_and_spa_fallback() {
        let (status, _, location, _) = get(vue_config(fixture()), "/apidoc").await;
        assert_eq!(status, 307);
        assert_eq!(location.as_deref(), Some("/apidoc/"));

        for (uri, ct, marker) in [
            ("/apidoc/", "text/html; charset=utf-8", "UI fixture index"),
            (
                "/apidoc/config.js",
                "text/javascript; charset=utf-8",
                "apidocFeConfig",
            ),
            (
                "/apidoc/assets/app.js",
                "text/javascript; charset=utf-8",
                "fixture asset",
            ),
            ("/apidoc/assets/app.css", "text/css; charset=utf-8", "#app"),
            (
                "/apidoc/deep/link",
                "text/html; charset=utf-8",
                "UI fixture index",
            ),
        ] {
            let (status, got_ct, _, body) = get(vue_config(fixture()), uri).await;
            assert_eq!(status, 200, "{uri}");
            assert_eq!(got_ct.as_deref(), Some(ct), "{uri}");
            assert!(String::from_utf8_lossy(&body).contains(marker), "{uri}");
        }
        assert_eq!(
            get(vue_config(fixture()), "/apidoc/assets/missing.js")
                .await
                .0,
            404
        );
    }

    #[actix_web::test]
    async fn api_routes_take_precedence_over_spa() {
        let (status, ct, _, body) = get(vue_config(fixture()), "/apidoc/api.json").await;
        assert_eq!(status, 200);
        assert_eq!(ct.as_deref(), Some("application/json"));
        assert!(String::from_utf8_lossy(&body).contains("\"endpoints\""));

        let (status, ct, _, body) = get(vue_config(fixture()), "/apidoc/ui/v1/capabilities").await;
        assert_eq!(status, 200);
        assert_eq!(ct.as_deref(), Some("application/json"));
        assert!(String::from_utf8_lossy(&body).contains("\"contract\""));

        assert_eq!(
            get(vue_config(fixture()), "/apidoc/ui/v1/nope").await.0,
            404
        );
    }

    #[actix_web::test]
    async fn traversal_is_blocked() {
        for uri in [
            "/apidoc/../ui-secret.txt",
            "/apidoc/%2e%2e/ui-secret.txt",
            "/apidoc/..%2fui-secret.txt",
            "/apidoc/assets/../../ui-secret.txt",
        ] {
            let (status, _, _, body) = get(vue_config(fixture()), uri).await;
            assert_ne!(status, 200, "{uri}");
            assert!(
                !String::from_utf8_lossy(&body).contains("TOP-SECRET"),
                "{uri}"
            );
        }
    }
}
