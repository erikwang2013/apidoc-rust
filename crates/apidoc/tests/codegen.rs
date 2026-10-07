//! v2 codegen 模板引擎测试：变量与未定义保留、单层/嵌套 each、空列表、
//! 未闭合块不 panic、三个内置模板的产出标记、find 覆盖规则。

use apidoc::codegen::{find, render, TEMPLATES};
use apidoc::{ApiDoc, ApidocConfig, CodegenTemplate, DocEndpoint, TableDef, TableField};
use std::time::{Duration, Instant};

fn ep(title: &str, url: &str, method: &str, group: &str, author: &str) -> DocEndpoint {
    DocEndpoint {
        title: title.into(),
        url: url.into(),
        method: method.into(),
        group: group.into(),
        author: author.into(),
        desc: format!("{title}的说明"),
        ..Default::default()
    }
}

/// 全量 fixture：2 个 endpoint（同组、同名 url 不同 method）+ 1 张表 3 个字段，
/// 覆盖 required/default/desc/mock 的有值与缺省。
fn full_doc() -> ApiDoc {
    ApiDoc {
        config: ApidocConfig {
            title: "示例 API".into(),
            description: Some("演示文档".into()),
            tables: vec![TableDef {
                key: "user".into(),
                title: "用户表".into(),
                fields: vec![
                    TableField {
                        name: "id".into(),
                        ty: "INTEGER".into(),
                        required: true,
                        default: Some("0".into()),
                        desc: Some("主键".into()),
                        mock: Some("1".into()),
                    },
                    TableField {
                        name: "nickname".into(),
                        ty: "VARCHAR(32)".into(),
                        required: false,
                        default: None,
                        desc: None,
                        mock: None,
                    },
                    TableField {
                        name: "email".into(),
                        ty: "VARCHAR(128)".into(),
                        required: true,
                        default: None,
                        desc: Some("邮箱".into()),
                        mock: None,
                    },
                ],
            }],
            ..Default::default()
        },
        endpoints: vec![
            ep("用户列表", "/api/user", "GET", "user", "张三"),
            ep("新建用户", "/api/user", "POST", "user", "李四"),
        ],
        apps: Vec::new(),
    }
}

/// 最小 fixture：无 description、无 tables、无 endpoints。
fn bare_doc() -> ApiDoc {
    ApiDoc {
        config: ApidocConfig { title: "空文档".into(), ..Default::default() },
        endpoints: Vec::new(),
        apps: Vec::new(),
    }
}

/// 3 个 endpoint 的最小 fixture：深度测试用（叶子数 = 3^n，易断言）。
fn three_eps_doc() -> ApiDoc {
    ApiDoc {
        config: ApidocConfig::default(),
        endpoints: (0..3)
            .map(|i| ep(&format!("E{i}"), &format!("/e/{i}"), "GET", "g", ""))
            .collect(),
        apps: Vec::new(),
    }
}

/// n 个空 endpoint：展开预算边界测试用（模板不读字段，构造极廉价）。
fn flat_doc(n: usize) -> ApiDoc {
    ApiDoc {
        config: ApidocConfig::default(),
        endpoints: vec![DocEndpoint::default(); n],
        apps: Vec::new(),
    }
}

#[test]
fn plain_text_without_tags_is_returned_as_is() {
    let t = "没有任何标签的纯文本\n第二行";
    assert_eq!(render(t, &full_doc()), t);
}

#[test]
fn variables_resolve_and_unknown_keys_stay_verbatim() {
    let out = render("{{title}}|{{description}}|{{nope}}|{{ nope }}", &full_doc());
    assert_eq!(out, "示例 API|演示文档|{{nope}}|{{ nope }}");
    // description 为 None → 空串（已定义的空值，不是未定义）
    assert_eq!(render("[{{description}}]", &bare_doc()), "[]");
    // 顶层只认 title/description
    assert_eq!(render("{{url}}", &full_doc()), "{{url}}");
}

#[test]
fn each_over_endpoints_binds_item_fields() {
    let out = render(
        "{{#each endpoints}}[{{method}} {{url}} {{group}} {{author}}]{{/each}}",
        &full_doc(),
    );
    assert_eq!(out, "[GET /api/user user 张三][POST /api/user user 李四]");
    // 块内未定义名仍原样保留
    assert_eq!(render("{{#each endpoints}}{{bogus}}{{/each}}", &full_doc()), "{{bogus}}{{bogus}}");
}

#[test]
fn nested_each_over_tables_and_fields() {
    let tpl = "{{#each tables}}T:{{key}}/{{title}}\n\
               {{#each fields}}  {{name}} {{ty}} req={{required}} d={{default}} \
               desc={{desc}} mock={{mock}} nn=[{{not_null}}] dc=[{{default_clause}}]{{comma}}\n\
               {{/each}}{{/each}}";
    let out = render(tpl, &full_doc());
    let expect = "T:user/用户表\n\
                  \x20 id INTEGER req=true d=0 desc=主键 mock=1 nn=[ NOT NULL] dc=[ DEFAULT 0],\n\
                  \x20 nickname VARCHAR(32) req=false d= desc= mock= nn=[] dc=[],\n\
                  \x20 email VARCHAR(128) req=true d= desc=邮箱 mock= nn=[ NOT NULL] dc=[]\n";
    assert_eq!(out, expect);
    // 作用域链：fields 块内未命中的名字回落到外层 tables 块
    assert_eq!(
        render("{{#each tables}}{{#each fields}}{{key}}.{{name}};{{/each}}{{/each}}", &full_doc()),
        "user.id;user.nickname;user.email;"
    );
}

#[test]
fn empty_or_missing_lists_render_empty() {
    assert_eq!(render("A{{#each endpoints}}X{{/each}}B", &bare_doc()), "AB");
    assert_eq!(render("A{{#each tables}}X{{/each}}B", &bare_doc()), "AB");
    assert_eq!(render("A{{#each nonsense}}X{{/each}}B", &full_doc()), "AB");
    // fields 不在任何表块内 → 不可得 → 空
    assert_eq!(render("A{{#each fields}}X{{/each}}B", &full_doc()), "AB");
}

#[test]
fn unclosed_blocks_are_kept_verbatim_without_panic() {
    // 未闭合 each：整段原样
    let t = "前{{#each endpoints}}后";
    assert_eq!(render(t, &full_doc()), t);
    // 外层未闭合（内层完整）：仍整段原样
    let t = "{{#each tables}}x{{#each fields}}y{{/each}}";
    assert_eq!(render(t, &full_doc()), t);
    // 标签本身没闭合
    let t = "A{{title";
    assert_eq!(render(t, &full_doc()), t);
    // 游离的 /each
    let t = "A{{/each}}B";
    assert_eq!(render(t, &full_doc()), t);
}

#[test]
fn nesting_at_the_cap_is_left_unexpanded_and_returns_immediately() {
    // 33 层：前 32 层各展开 1 次（full_doc 恰有 1 张表），第 33 个块触到
    // MAX_DEPTH=32 → 整段原样保留，不展开、不递归
    let tpl = format!("{}X{}", "{{#each tables}}".repeat(33), "{{/each}}".repeat(33));
    let t0 = Instant::now();
    let out = render(&tpl, &full_doc());
    let took = t0.elapsed();
    assert_eq!(out, "{{#each tables}}X{{/each}}", "超限块应原样保留");
    assert!(took < Duration::from_secs(1), "应在毫秒级返回，实测 {took:?}");
}

#[test]
fn very_deep_nesting_does_not_stack_overflow() {
    // 无上限时 2 万层递归会栈溢出（fatal runtime error，进程 abort）；
    // 有上限后第 33 层起不再展开，只剩原样文本
    const N: usize = 20_000;
    let tpl = format!("{}X{}", "{{#each tables}}".repeat(N), "{{/each}}".repeat(N));
    let t0 = Instant::now();
    let out = render(&tpl, &full_doc());
    let took = t0.elapsed();
    assert!(out.contains('X'));
    assert_eq!(out.matches("{{#each tables}}").count(), N - 32);
    assert_eq!(out.matches("{{/each}}").count(), N - 32);
    // 实测 debug ~0.8s（32 层 × 2 万标签的扫描开销，线性）；上界只防退化成挂死
    assert!(took < Duration::from_secs(5), "应在时限内返回，实测 {took:?}");
}

#[test]
fn depth_12_self_nesting_is_cut_by_budget_and_keeps_raw_text() {
    // 12 层 < 深度上限 32，但 3^12=531441 叶 > 展开预算 100_000：预算中途耗尽，
    // 在展开中的祖先链一并回退 → 整段渲染结果就是模板原文（未展开的 {{#each 可见，
    // 证明是显式截断而非静默丢内容）；末尾再挂一个普通块验证"后续块同样原样"
    let nested = format!("{}X{}", "{{#each endpoints}}".repeat(12), "{{/each}}".repeat(12));
    let tpl = nested + "|{{#each endpoints}}Y{{/each}}";
    let doc = three_eps_doc();
    let t0 = Instant::now();
    let out = render(&tpl, &doc);
    let took = t0.elapsed();
    eprintln!("depth12 self-nest render took {took:?}");
    assert_eq!(out, tpl, "超预算 → 整块+祖先链回退为原文，后续块同样原样，不半展开");
    assert!(took < Duration::from_secs(1), "应在时限内返回，实测 {took:?}");
    // 预算计数确定性：同输入同样输出
    assert_eq!(out, render(&tpl, &doc));
}

#[test]
fn normal_nesting_within_budget_fully_expands() {
    // 3 层 × 3 endpoint = 27 叶，远低于预算 → 完整展开，不得误伤
    let tpl = "{{#each endpoints}}{{#each endpoints}}{{#each endpoints}}X{{/each}}{{/each}}{{/each}}";
    let out = render(tpl, &three_eps_doc());
    assert_eq!(out.matches('X').count(), 27);
    assert!(!out.contains("{{"), "预算内不应残留未展开标签");
}

#[test]
fn expansion_budget_boundary() {
    let tpl = "{{#each endpoints}}X{{/each}}";
    // 恰好 100_000 次 item 迭代：预算内，完整展开
    let out = render(tpl, &flat_doc(100_000));
    assert_eq!(out.matches('X').count(), 100_000);
    assert!(!out.contains("{{"));
    // 100_001 次：剩余预算不够展开整块 → 整块原样保留（不半展开：一个都不展开）
    assert_eq!(render(tpl, &flat_doc(100_001)), tpl);
}

#[test]
fn builtin_api_ts_has_a_function_per_endpoint_with_url_and_method() {
    let t = find("api.ts", &[]).expect("内置 api.ts 缺失");
    let doc = full_doc();
    let out = render(t, &doc);
    assert!(out.contains("示例 API"), "顶部应有 config.title 注释");
    for e in &doc.endpoints {
        assert!(out.contains(&format!("{} {}", e.method, e.url)), "缺 method+url: {}", e.url);
        assert!(
            out.contains(&format!("fetch(\"{}\", {{ method: \"{}\" }}", e.url, e.method)),
            "函数体应 fetch 对应 url（{}）",
            e.url
        );
        assert!(out.contains(&e.title), "缺 title: {}", e.title);
    }
    assert!(out.contains("分组: user"), "应逐条标注 group");
    assert_eq!(
        out.matches("async (): Promise<unknown> => {").count(),
        doc.endpoints.len(),
        "每个 endpoint 应有且仅有一个函数"
    );
    assert!(!out.contains("{{"), "不应残留未解析标签");
}

#[test]
fn builtin_handler_rs_has_a_skeleton_per_endpoint() {
    let t = find("handler.rs", &[]).expect("内置 handler.rs 缺失");
    let doc = full_doc();
    let out = render(t, &doc);
    for e in &doc.endpoints {
        assert!(
            out.contains(&format!("// {} {} {}", e.title, e.method, e.url)),
            "缺注释行: {}",
            e.url
        );
        assert!(out.contains(&format!("fn {}_{}() {{}}", e.method, e.group)), "缺函数骨架");
    }
    assert_eq!(out.matches("() {}").count(), doc.endpoints.len());
    assert!(out.contains("示例 API"));
    assert!(!out.contains("{{"), "不应残留未解析标签");
}

#[test]
fn builtin_schema_sql_creates_tables_with_fields() {
    let t = find("schema.sql", &[]).expect("内置 schema.sql 缺失");
    let out = render(t, &full_doc());
    assert!(out.contains("CREATE TABLE IF NOT EXISTS user ("));
    assert!(out.contains("id INTEGER NOT NULL DEFAULT 0, -- 主键"));
    assert!(out.contains("nickname VARCHAR(32), -- "));
    assert!(out.contains("email VARCHAR(128) NOT NULL -- 邮箱"));
    assert!(out.contains(");"));
    assert!(!out.contains(",\n)"), "末字段不能带逗号");
    assert!(!out.contains("{{"), "不应残留未解析标签");
    // 无 tables：只剩注释说明，没有建表语句
    let out = render(t, &bare_doc());
    assert!(!out.contains("CREATE TABLE"));
    assert!(out.lines().all(|l| l.is_empty() || l.starts_with("--")), "应只剩注释:\n{out}");
}

#[test]
fn find_prefers_custom_and_falls_back_to_builtin() {
    assert!(find("api.ts", &[]).is_some());
    let custom =
        vec![CodegenTemplate { name: "api.ts".into(), template: "CUSTOM {{title}}".into() }];
    assert_eq!(find("api.ts", &custom), Some("CUSTOM {{title}}"));
    assert_eq!(render(find("api.ts", &custom).unwrap(), &full_doc()), "CUSTOM 示例 API");
    assert_eq!(find("missing.ts", &custom), None);
    for name in ["api.ts", "handler.rs", "schema.sql"] {
        assert!(TEMPLATES.iter().any(|(n, _)| *n == name), "缺内置模板 {name}");
    }
}

#[test]
fn render_is_deterministic() {
    let t = find("api.ts", &[]).unwrap();
    assert_eq!(render(t, &full_doc()), render(t, &full_doc()));
}
