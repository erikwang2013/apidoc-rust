//! v2 代码生成器：模板 + 文档模型 → 前端 Api 文件 / 接口骨架 / 建表 SQL。
//!
//! 模板语法（刻意极小，够用即止，不加依赖、不用正则）：
//! - `{{key}}` 变量，未定义则原样保留（拼错立刻可见）；
//! - `{{#each endpoints}}…{{/each}}` 列表迭代，块内以当前项字段为变量；
//! - `{{#each tables}}…{{#each fields}}…{{/each}}…{{/each}}` 支持嵌套；
//! - 列表缺失/为空 → 渲染为空串；未闭合的 `{{#each` → 整段原样输出，不 panic。
//!
//! 变量查找沿作用域链由内向外：先最内层块的当前项，未命中回落到外层块，再到顶层
//! `title`/`description`；全链未命中才原样保留。
//!
//! 变量表（spec 之外，字段级另补 3 个派生变量，让冻结的 if/变换语法能输出合法
//! SQL——派生值即成品字符串，不新增任何语法）：
//! - 顶层：`title`（config.title）/ `description`（None 时为空串）；
//! - endpoints 项：`title`/`url`/`method`/`group`/`desc`/`author`，
//!   加派生 `url_js` / `method_js`（JSON 字符串字面量，可直接落进 JS 代码位）；
//! - tables 项：`key`/`title`，加派生 `key_q`（反引号标识符）；
//! - fields 项：`name`/`ty`/`required`(true|false)/`default`/`desc`/`mock`，
//!   加派生 `not_null`（`" NOT NULL"`|空）、`default_clause`（`" DEFAULT x"`|空，
//!   值按 SQL 字面量处理：数字/NULL 原样，其余单引号包裹）与 `name_q`（反引号标识符）；
//! - 任意 each 块内：`comma`（非末项为 `","`，末项为空串，SQL 用）。
//!
//! 转义策略（三类危险位各归各的派生变量，模板本身不做上下文感知转义）：
//! 标识符 → `*_q`；JS 字符串字面量 → `*_js`；行式结构里的裸换行 → 所有变量统一
//! 折叠（见 [`line_safe`]）。注释放 `//` 或 `--` 行注释，所以 `*/` 不构成出口。
//!
//! - 嵌套深度上限 [`MAX_DEPTH`]（32 层，防栈溢出）与展开预算 [`MAX_EXPANSIONS`]
//!   （10 万次 item 迭代，防自嵌套指数放大）：超限的块连同在展开中的祖先链一并
//!   回退为原文，原子不半展开（`/apidoc/generate` 每请求重渲染，不能卡死进程）。
//!
//! 已知边界（冻结语法的直接后果，模板内无法规避）：
//! - 无字符串变换，`/api/user` 变不成合法标识符——`api.ts` 用计算键
//!   `[method_js + " " + url_js]` 承载 method+url（合法且唯一），`handler.rs`
//!   函数名只能拼 `{{method}}_{{group}}`（同组同方法可能重名，骨架模板可接受）；
//! - 无 if，`api.ts`/`handler.rs` 的 group 只能逐条以注释标注，做不出真正的分节头。

use crate::{ApiDoc, DocEndpoint, TableDef, TableField};

/// 嵌套深度上限（作用域链长度）。自嵌套模板按 `|列表|^层数` 放大，且深递归会
/// 耗尽栈（实测 ~2 万层溢出）：超限的块整段原样保留（同未闭合策略），不展开、
/// 不递归。32 远小于栈耗尽量级，正常文档嵌套不超过 3 层（如 tables×fields）。
const MAX_DEPTH: usize = 32;

/// 展开预算（累计 item 迭代次数，一次迭代计 1）。深度上限挡不住深度 ≤32 的指数
/// 放大（`|列表|^层数`），预算兜底：整块装不下就直接整块原样保留；展开中途耗尽
/// 的，当前块连同在展开中的祖先链一并回退为原文（原子，不半展开）。正常文档
/// （≤3 层 × 数十接口）远低于此；纯计数，不依赖时间/线程，确定性。
const MAX_EXPANSIONS: usize = 100_000;

/// 内置模板 `(name, 源码)`：前端 Api 文件、Rust 接口骨架、建表 SQL。
/// `ApidocConfig.codegen` 中的同名模板覆盖这里的版本。
pub const TEMPLATES: &[(&str, &str)] = &[
    (
        "api.ts",
        r#"// {{title}} —— 前端 Api 文件
// {{description}}
// 由 apidoc-rust codegen 自动生成，请勿手改。

export const api = {
{{#each endpoints}}
  // {{title}} | {{method}} {{url}} | 分组: {{group}} | 作者: {{author}}
  // {{desc}}
  [{{method_js}} + " " + {{url_js}}]: async (): Promise<unknown> => {
    const res = await fetch({{url_js}}, { method: {{method_js}} });
    return res.json();
  },
{{/each}}
};
"#,
    ),
    (
        "handler.rs",
        r#"// {{title}} —— 接口骨架
// {{description}}
// 由 apidoc-rust codegen 自动生成，请勿手改。
{{#each endpoints}}
// {{title}} {{method}} {{url}}
fn {{fn_name}}() {}
{{/each}}
"#,
    ),
    (
        "schema.sql",
        r#"-- {{title}} —— 建表 SQL
-- {{description}}
-- 由 apidoc-rust codegen 自动生成，请勿手改。
{{#each tables}}
CREATE TABLE IF NOT EXISTS {{key_q}} ( -- {{title}}
{{#each fields}}  {{name_q}} {{ty}}{{not_null}}{{default_clause}}{{comma}} -- {{desc}}
{{/each}});
{{/each}}
"#,
    ),
];

/// 按名字取模板：先 `config.codegen` 自定义，再内置。
pub fn find<'a>(name: &'a str, custom: &'a [crate::CodegenTemplate]) -> Option<&'a str> {
    if let Some(t) = custom.iter().find(|t| t.name == name) {
        return Some(&t.template);
    }
    TEMPLATES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| *t)
}

/// 渲染模板。确定性：同一 (template, doc) 恒得同一输出（纯字符串拼接，
/// 预算计数与遍历顺序确定，不依赖时间/线程）。
pub fn render(template: &str, doc: &ApiDoc) -> String {
    render_with(template, doc, None)
}

/// 只渲染 `selected` 里的端点（UIC-1 代码模板的勾选语义；`{{#each endpoints}}`
/// 之外的变量照常取文档级值）。
pub fn render_selected(template: &str, doc: &ApiDoc, selected: &[&DocEndpoint]) -> String {
    render_with(template, doc, Some(selected))
}

fn render_with(template: &str, doc: &ApiDoc, selected: Option<&[&DocEndpoint]>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut stack = Vec::new();
    let mut used = 0usize;
    render_into(template, doc, selected, &mut stack, &mut out, &mut used);
    out
}

/// 一层模板作用域 = 当前迭代项 + 位置（`{{comma}}` 需要知道是不是末项）。
#[derive(Clone, Copy)]
struct Frame<'a> {
    item: Item<'a>,
    last: bool,
}

/// 迭代项类型：三种列表共用作用域链，嵌套 each 自然成立。
#[derive(Clone, Copy)]
enum Item<'a> {
    Endpoint(&'a DocEndpoint),
    Table(&'a TableDef),
    Field(&'a TableField),
}

/// 递归渲染 `t` 到 `out`，`stack` 为当前作用域链（由外到内），`used` 为已用
/// item 迭代次数（MAX_EXPANSIONS 预算，跨递归共享）。
/// 复杂度：每层只扫一遍块体，展开总量受 MAX_DEPTH / MAX_EXPANSIONS 双重钳制。
fn render_into<'a>(
    t: &str,
    doc: &'a ApiDoc,
    selected: Option<&[&'a DocEndpoint]>,
    stack: &mut Vec<Frame<'a>>,
    out: &mut String,
    used: &mut usize,
) {
    let mut i = 0;
    loop {
        let Some(rel) = t[i..].find("{{") else {
            out.push_str(&t[i..]);
            return;
        };
        let p = i + rel;
        out.push_str(&t[i..p]);
        let Some(close) = t[p + 2..].find("}}") else {
            // 标签未闭合：剩余原样输出
            out.push_str(&t[p..]);
            return;
        };
        let end = p + 2 + close + 2;
        let content = &t[p + 2..p + 2 + close];
        if let Some(name) = each_name(content) {
            let Some((body_end, after)) = match_each(t, end) else {
                // 未闭合的 each：从开标签起整段原样输出，不 panic
                out.push_str(&t[p..]);
                return;
            };
            let items = each_items(name, stack, doc, selected);
            let mark = out.len();
            // 超嵌套上限，或剩余预算不够展开整块：直接整块原样保留（零展开）
            let mut cut = stack.len() >= MAX_DEPTH || items.len() > MAX_EXPANSIONS - *used;
            if !cut {
                for frame in items {
                    if *used >= MAX_EXPANSIONS {
                        // 嵌套块把预算耗尽：当前块连同已渲染部分整块作废（原子性，
                        // 不半展开）；上层下一个 item 会检测到同样的耗尽继续向上回退
                        cut = true;
                        break;
                    }
                    *used += 1; // 一次 item 迭代计 1
                    stack.push(frame);
                    render_into(&t[end..body_end], doc, selected, stack, out, used);
                    stack.pop();
                }
            }
            if cut {
                // 回退到块首并整块原样输出，让超限滥用立刻可见
                out.truncate(mark);
                out.push_str(&t[p..after]);
            }
            i = after;
        } else if content.trim() == "/each" {
            // 无配对开标签的游离 /each：原样输出
            out.push_str(&t[p..end]);
            i = end;
        } else if let Some(v) = lookup(content.trim(), stack, doc) {
            out.push_str(&v);
            i = end;
        } else {
            // 未定义变量：原样保留，拼错立刻可见
            out.push_str(&t[p..end]);
            i = end;
        }
    }
}

/// `{{#each <name>}}` → Some(name)；其余（含 `{{#eachfoo}}`、`{{#each}}`）→ None。
fn each_name(content: &str) -> Option<&str> {
    let rest = content.trim().strip_prefix("#each")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let name = rest.trim();
    (!name.is_empty()).then_some(name)
}

/// 找与已开启的 `{{#each …}}` 配对的 `{{/each}}`（`from` 为块体起点）。
/// 返回 `(块体结束位置, 闭合标签之后的位置)`；未闭合返回 None。
/// 单趟前扫，区域互不重叠 → 线性。
fn match_each(t: &str, from: usize) -> Option<(usize, usize)> {
    let mut depth = 1usize;
    let mut i = from;
    while let Some(rel) = t[i..].find("{{") {
        let p = i + rel;
        let close = t[p + 2..].find("}}")?; // 标签没闭合 → 整块按未闭合处理
        let end = p + 2 + close + 2;
        let content = &t[p + 2..p + 2 + close];
        if content.trim() == "/each" {
            depth -= 1;
            if depth == 0 {
                return Some((p, end));
            }
        } else if each_name(content).is_some() {
            depth += 1;
        }
        i = end;
    }
    None
}

/// 解析列表名 → 迭代项（带末项标记）。
/// `fields` 取作用域链上最近一张表的字段；未知名字或列表不可得 → 空，渲染为空串。
fn each_items<'a>(
    name: &str,
    stack: &[Frame<'a>],
    doc: &'a ApiDoc,
    selected: Option<&[&'a DocEndpoint]>,
) -> Vec<Frame<'a>> {
    let items: Vec<Item<'a>> = match name {
        "endpoints" => match selected {
            Some(endpoints) => endpoints.iter().copied().map(Item::Endpoint).collect(),
            None => doc.endpoints.iter().map(Item::Endpoint).collect(),
        },
        "tables" => doc.config.tables.iter().map(Item::Table).collect(),
        "fields" => stack
            .iter()
            .rev()
            .find_map(|f| match f.item {
                Item::Table(t) => Some(t.fields.iter().map(Item::Field).collect()),
                _ => None,
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let n = items.len();
    items
        .into_iter()
        .enumerate()
        .map(|(i, item)| Frame { item, last: i + 1 == n })
        .collect()
}

/// 查一个变量名：作用域链由内向外，再顶层。未命中返回 None（调用方原样保留）。
/// 所有文本值统一过 [`line_safe`] —— 这是唯一的统一出口，派生变量也不例外。
fn lookup(key: &str, stack: &[Frame<'_>], doc: &ApiDoc) -> Option<String> {
    // comma 是位置派生量，只由最内层块决定
    if key == "comma" {
        return stack
            .last()
            .map(|f| if f.last { String::new() } else { ",".to_string() });
    }
    for f in stack.iter().rev() {
        if let Some(v) = item_var(&f.item, key) {
            return Some(line_safe(v));
        }
    }
    match key {
        "title" => Some(line_safe(doc.config.title.clone())),
        "description" => Some(line_safe(doc.config.description.clone().unwrap_or_default())),
        _ => None,
    }
}

/// 折叠换行：三份内置产物（行注释 / 字符串字面量 / SQL 行）都是行式结构，值里的
/// 裸换行只会把一行拆成两行（注释越界、SQL 断句）。需要多行文本就不该走 codegen。
/// ponytail: 一刀切折叠，没有上下文感知；真要多行文本的模板再说。
fn line_safe(s: String) -> String {
    if s.contains(['\n', '\r']) {
        s.replace('\r', "").replace('\n', " ")
    } else {
        s
    }
}

/// SQL 标识符：反引号包裹（MySQL/SQLite），内部反引号翻倍转义。
/// 表/字段名来自配置，`order` / `select` / `key` 这类保留字不加引号必产出非法 SQL。
fn quote_ident(s: &str) -> String {
    format!("`{}`", s.replace('`', "``"))
}

/// SQL 字面量：数字与 NULL 原样，其余单引号包裹、内部单引号翻倍。
/// `DEFAULT hello` 是语法错误，字符串默认值必须带引号。
fn sql_literal(s: &str) -> String {
    let t = s.trim();
    let numeric =
        !t.is_empty() && !t.chars().any(|c| c.is_ascii_alphabetic()) && t.parse::<f64>().is_ok();
    if numeric || t.eq_ignore_ascii_case("null") {
        t.to_string()
    } else {
        format!("'{}'", t.replace('\'', "''"))
    }
}

/// JS 字符串字面量：JSON 转义（双引号包裹）。JS 字符串与 JSON 字面量同构。
fn js_str(s: &str) -> String {
    serde_json::to_string(s).expect("a string always serializes")
}

/// Rust 函数名：`{method}_{group}` 过标识符清洗（非标识符字符 → `_`），
/// 空或数字开头补 `_`。group 带空格/符号时原样拼会产出非法 Rust。
/// 同组同方法仍可能重名——模板没有去重能力，骨架模板可接受。
fn fn_ident(method: &str, group: &str) -> String {
    let mut out = String::with_capacity(method.len() + group.len() + 1);
    for c in format!("{method}_{group}").chars() {
        out.push(if c.is_alphanumeric() || c == '_' { c } else { '_' });
    }
    if out.is_empty() || out.chars().next().is_some_and(|c| c.is_numeric()) {
        out.insert(0, '_');
    }
    out
}

/// 当前项自己的字段：定义 → Some(值)；不属于该类型 → None（继续往外找）。
fn item_var(item: &Item<'_>, key: &str) -> Option<String> {
    Some(match item {
        Item::Endpoint(e) => match key {
            "title" => e.title.clone(),
            "url" => e.url.clone(),
            "method" => e.method.clone(),
            "group" => e.group.clone(),
            "desc" => e.desc.clone(),
            "author" => e.author.clone(),
            // 落进 JS 代码位（字符串字面量）的 url/method 必须带引号转义
            "url_js" => js_str(&e.url),
            "method_js" => js_str(&e.method),
            // 落进 Rust 代码位的函数名：必须过标识符清洗（group 可能带空格/符号）
            "fn_name" => fn_ident(&e.method, &e.group),
            _ => return None,
        },
        Item::Table(t) => match key {
            "key" => t.key.clone(),
            "title" => t.title.clone(),
            "key_q" => quote_ident(&t.key),
            _ => return None,
        },
        Item::Field(f) => match key {
            "name" => f.name.clone(),
            "name_q" => quote_ident(&f.name),
            "ty" => f.ty.clone(),
            // 布尔按文本渲染（spec）：true / false
            "required" => f.required.to_string(),
            "default" => f.default.clone().unwrap_or_default(),
            "desc" => f.desc.clone().unwrap_or_default(),
            "mock" => f.mock.clone().unwrap_or_default(),
            // 派生：值自带前导空格，缺失即整段消失，SQL 不会出现悬空关键字
            "not_null" => (if f.required { " NOT NULL" } else { "" }).to_string(),
            "default_clause" => f
                .default
                .as_ref()
                .map(|d| format!(" DEFAULT {}", sql_literal(d)))
                .unwrap_or_default(),
            _ => return None,
        },
    })
}
