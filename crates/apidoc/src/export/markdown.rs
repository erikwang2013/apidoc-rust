//! markdown 导出：`# 标题` → `## group` 分节 → 每接口 `### 标题`。
//! 参数表合并 route_param + param + query，children 用点路径展平。

use crate::{ApiDoc, DocEndpoint, DocParam};

use super::{flatten, group_endpoints};

pub fn render(doc: &ApiDoc) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", doc.config.title));
    for (group, eps) in group_endpoints(doc) {
        out.push_str(&format!("## {group}\n\n"));
        for ep in eps {
            render_endpoint(ep, &mut out);
        }
    }
    out
}

fn render_endpoint(ep: &DocEndpoint, out: &mut String) {
    out.push_str(&format!("### {}\n\n", ep.title));
    out.push_str(&format!("```\n{} {}\n```\n\n", ep.method, ep.url));
    if !ep.desc.is_empty() {
        out.push_str(&format!("{}\n\n", ep.desc));
    }
    let mut meta = Vec::new();
    if !ep.author.is_empty() {
        meta.push(format!("作者：{}", ep.author));
    }
    if !ep.tags.is_empty() {
        meta.push(format!("标签：{}", ep.tags.join("、")));
    }
    if !meta.is_empty() {
        out.push_str(&format!("{}\n\n", meta.join("　")));
    }
    let mut rows: Vec<(String, &DocParam)> = Vec::new();
    for list in [&ep.route_params, &ep.params, &ep.querys] {
        rows.extend(flatten(list));
    }
    if !rows.is_empty() {
        out.push_str("| 名称 | 类型 | 必填 | 默认 | 描述 |\n|---|---|---|---|---|\n");
        for (path, p) in rows {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                cell(&path),
                p.ty,
                if p.required { "是" } else { "否" },
                cell(p.default.unwrap_or("-")),
                cell(p.desc.unwrap_or("-")),
            ));
        }
        out.push('\n');
    }
/// 表格单元格转义：`|` 会拆列，换行吞掉。
fn cell(s: &str) -> String {
    s.replace('|', "\\|")
}

    for (label, examples) in [("成功响应", &ep.success), ("错误响应", &ep.error)] {
        for ex in examples {
            out.push_str(&format!("**{label} {}**\n\n```json\n{}\n```\n\n", ex.code, ex.example));
        }
    }
    if !ep.md.is_empty() {
        out.push_str(&ep.md);
        out.push_str("\n\n");
    }
}
