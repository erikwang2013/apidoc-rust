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
//! - endpoints 项：`title`/`url`/`method`/`group`/`desc`/`author`；
//! - tables 项：`key`/`title`；
//! - fields 项：`name`/`ty`/`required`(true|false)/`default`/`desc`/`mock`，
//!   加派生 `not_null`（`" NOT NULL"`|空）与 `default_clause`（`" DEFAULT x"`|空）；
//! - 任意 each 块内：`comma`（非末项为 `","`，末项为空串，SQL 用）。
//!
//! - 嵌套深度上限 [`MAX_DEPTH`]（32 层，防栈溢出）与展开预算 [`MAX_EXPANSIONS`]
//!   （10 万次 item 迭代，防自嵌套指数放大）：超限的块连同在展开中的祖先链一并
//!   回退为原文，原子不半展开（`/apidoc/generate` 每请求重渲染，不能卡死进程）。
//!
//! 已知边界（冻结语法的直接后果，模板内无法规避）：
//! - 无字符串变换，`/api/user` 变不成合法标识符——`api.ts` 用对象键 `"METHOD url"`
//!   承载 method+url（合法且唯一），`handler.rs` 函数名只能拼 `{{method}}_{{group}}`
//!   （同组同方法可能重名，骨架模板可接受）；
//! - 无 if，`api.ts`/`handler.rs` 的 group 只能逐条以注释标注，做不出真正的分节头；
//! - 不做转义：模板来源可信（编译期注解/项目配置），需要时再加。

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
        r#"/**
 * {{title}} —— 前端 Api 文件
 * {{description}}
 * 由 apidoc-rust codegen 自动生成，请勿手改。
 */

export const api = {
{{#each endpoints}}
  /**
   * {{title}}
   * {{method}} {{url}} | 分组: {{group}} | 作者: {{author}}
   * {{desc}}
   */
  "{{method}} {{url}}": async (): Promise<unknown> => {
    const res = await fetch("{{url}}", { method: "{{method}}" });
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
fn {{method}}_{{group}}() {}
{{/each}}
"#,
    ),
    (
        "schema.sql",
        r#"-- {{title}} —— 建表 SQL
-- {{description}}
-- 由 apidoc-rust codegen 自动生成，请勿手改。
{{#each tables}}
CREATE TABLE IF NOT EXISTS {{key}} ( -- {{title}}
{{#each fields}}  {{name}} {{ty}}{{not_null}}{{default_clause}}{{comma}} -- {{desc}}
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
    let mut out = String::with_capacity(template.len());
    let mut stack = Vec::new();
    let mut used = 0usize;
    render_into(template, doc, &mut stack, &mut out, &mut used);
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
            let items = each_items(name, stack, doc);
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
                    render_into(&t[end..body_end], doc, stack, out, used);
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
fn each_items<'a>(name: &str, stack: &[Frame<'a>], doc: &'a ApiDoc) -> Vec<Frame<'a>> {
    let items: Vec<Item<'a>> = match name {
        "endpoints" => doc.endpoints.iter().map(Item::Endpoint).collect(),
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
fn lookup(key: &str, stack: &[Frame<'_>], doc: &ApiDoc) -> Option<String> {
    // comma 是位置派生量，只由最内层块决定
    if key == "comma" {
        return stack
            .last()
            .map(|f| if f.last { String::new() } else { ",".to_string() });
    }
    for f in stack.iter().rev() {
        if let Some(v) = item_var(&f.item, key) {
            return Some(v);
        }
    }
    match key {
        "title" => Some(doc.config.title.clone()),
        "description" => Some(doc.config.description.clone().unwrap_or_default()),
        _ => None,
    }
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
            _ => return None,
        },
        Item::Table(t) => match key {
            "key" => t.key.clone(),
            "title" => t.title.clone(),
            _ => return None,
        },
        Item::Field(f) => match key {
            "name" => f.name.clone(),
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
                .map(|d| format!(" DEFAULT {d}"))
                .unwrap_or_default(),
            _ => return None,
        },
    })
}
