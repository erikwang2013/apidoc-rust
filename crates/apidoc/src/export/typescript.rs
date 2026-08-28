//! TypeScript 导出：按 group 命名空间对象存路径常量（as const），
//! 每端点 `{Name}Params`（params+querys 合并，children 嵌套对象类型）与
//! `{Name}Result`（returned 映射）。

use crate::{ApiDoc, DocEndpoint, DocParam};

use super::{camel, group_endpoints, pascal, NameAllocator};

pub fn render(doc: &ApiDoc) -> String {
    let mut out = String::new();
    // 类型名按「方法 + URL 末段」派生：DocEndpoint 不携带端点 id/fn 名，
    // 且红线禁止 collect() 补字段，URL 是唯一稳定可用来源。
    // ponytail: GET /api/user/:id → GetUser；将来有 id 字段再换 fn 名。
    let mut type_names = NameAllocator::new();
    for (group, eps) in group_endpoints(doc) {
        let ns = namespace_name(group);
        out.push_str(&format!("export const {ns} = {{\n"));
        let mut keys = NameAllocator::new();
        for ep in &eps {
            let key = keys.alloc(camel(&base_name(ep)));
            out.push_str(&format!("  {key}: '{}' as const,\n", ep.url));
        }
        out.push_str("}\n\n");
        for ep in &eps {
            let base = type_names.alloc(base_name(ep));
            render_interface(&base, "Params", &merge_params(ep), &mut out);
            render_interface(&base, "Result", &ep.returned, &mut out);
        }
    }
    out
}

fn namespace_name(group: &str) -> String {
    if group == "未分组" {
        // default 是 TS 保留字，不能作标识符
        return "defaultGroup".to_string();
    }
    if group.is_ascii() {
        pascal(group)
    } else {
        // 非 ASCII（中文）group 是合法 JS 标识符，原样保留
        group.to_string()
    }
}

/// 类型名基名：方法 + URL 末段（占位符段跳过）转 PascalCase。
fn base_name(ep: &DocEndpoint) -> String {
    let last = ep
        .url
        .rsplit('/')
        .find(|s| !s.is_empty() && !s.starts_with(':') && !s.starts_with('{'))
        .unwrap_or("");
    pascal(&format!("{}_{}", ep.method.to_lowercase(), last))
}

fn merge_params(ep: &DocEndpoint) -> Vec<DocParam> {
    let mut v = Vec::new();
    v.extend(ep.params.iter().cloned());
    v.extend(ep.querys.iter().cloned());
    v
}

fn render_interface(base: &str, suffix: &str, list: &[DocParam], out: &mut String) {
    if list.is_empty() {
        return;
    }
    out.push_str(&format!("export interface {base}{suffix} {{\n"));
    for p in list {
        let opt = if p.required { "" } else { "?" };
        out.push_str(&format!("  {}{}: {}\n", p.name, opt, ts_type(p)));
    }
    out.push_str("}\n\n");
}

fn ts_type(p: &DocParam) -> String {
    if !p.children.is_empty() {
        if p.ty == "array" {
            format!("{}[]", ts_object(p.children))
        } else {
            ts_object(p.children)
        }
    } else {
        match p.ty {
            "int" | "float" => "number".to_string(),
            "bool" => "boolean".to_string(),
            "object" => "object".to_string(),
            "array" => "any[]".to_string(),
            _ => "string".to_string(),
        }
    }
}

fn ts_object(children: &[DocParam]) -> String {
    let fields: Vec<String> = children
        .iter()
        .map(|c| {
            let opt = if c.required { "" } else { "?" };
            format!("{}{}: {}", c.name, opt, ts_type(c))
        })
        .collect();
    format!("{{ {} }}", fields.join("; "))
}
