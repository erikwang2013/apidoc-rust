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
    if ep.protocol == "http" {
        out.push_str(&format!("```\n{} {}\n```\n\n", ep.method, ep.url));
    } else {
        let target = ep.protocol_opts.get("target").map_or("", String::as_str);
        out.push_str(&format!(
            "```\n{} {}{}\n```\n\n",
            ep.protocol,
            ep.url,
            if target.is_empty() { String::new() } else { format!(" → {target}") }
        ));
    }
    if !ep.events.is_empty() {
        out.push_str("| 方向 | 消息 | 类型 | 描述 |\n|---|---|---|---|\n");
        for ev in &ep.events {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                cell(ev.direction),
                cell(ev.name),
                cell(ev.ty),
                cell(ev.desc.unwrap_or("-")),
            ));
        }
        out.push('\n');
    }
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
                cell(p.ty),
                if p.required { "是" } else { "否" },
                cell(p.default.unwrap_or("-")),
                cell(p.desc.unwrap_or("-")),
            ));
        }
        out.push('\n');
    }

    for (label, examples) in [("成功响应", &ep.success), ("错误响应", &ep.error)] {
        for ex in examples {
            // 围栏至少 3 个反引号，且要比正文里最长的一串更长，否则示例里的 ``` 会提前
            // 关掉代码块，后面的文档全被吞进代码块
            let fence = "`".repeat(longest_backtick_run(ex.example).max(2) + 1);
            out.push_str(&format!(
                "**{label} {}**\n\n{fence}json\n{}\n{fence}\n\n",
                ex.code, ex.example
            ));
        }
    }
    if !ep.md.is_empty() {
        out.push_str(&ep.md);
        out.push_str("\n\n");
    }
}

/// 表格单元格转义：`|` 会拆列，裸换行会把一行拆成两行（表格断在中间）。
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ").replace('\r', "")
}

/// 正文里最长的一串连续反引号长度（代码围栏至少要比它长 1）。
fn longest_backtick_run(s: &str) -> usize {
    s.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}
