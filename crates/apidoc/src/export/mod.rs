//! M5 导出：markdown / typescript / swagger（OpenAPI3）三种格式渲染。
//! 各格式一个子模块，共享 helper 集中在本文件。

pub mod markdown;
pub mod swagger;
pub mod typescript;

use crate::{ApiDoc, DocEndpoint, DocParam};
use std::collections::HashSet;

/// 按 group 分组（保持首次出现顺序），无 group 落「未分组」。
pub fn group_endpoints(doc: &ApiDoc) -> Vec<(&str, Vec<&DocEndpoint>)> {
    let mut groups: Vec<(&str, Vec<&DocEndpoint>)> = Vec::new();
    for ep in &doc.endpoints {
        let g = if ep.group.is_empty() { "未分组" } else { ep.group.as_str() };
        match groups.iter_mut().find(|(name, _)| *name == g) {
            Some((_, list)) => list.push(ep),
            None => groups.push((g, vec![ep])),
        }
    }
    groups
}

/// children 递归展平为 (点路径, 参数)：`user.profile.bio` 风格，供一维渲染。
pub fn flatten(list: &[DocParam]) -> Vec<(String, &DocParam)> {
    fn walk<'a>(prefix: &str, list: &'a [DocParam], out: &mut Vec<(String, &'a DocParam)>) {
        for p in list {
            let path = if prefix.is_empty() {
                p.name.to_string()
            } else {
                format!("{prefix}.{}", p.name)
            };
            out.push((path.clone(), p));
            if !p.children.is_empty() {
                walk(&path, p.children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk("", list, &mut out);
    out
}

/// snake_case → PascalCase；非字母数字剔除；空串或数字开头加 `_` 前缀。
pub fn pascal(name: &str) -> String {
    let mut out = String::new();
    for seg in name.split('_') {
        let mut chars = seg.chars();
        if let Some(c) = chars.next() {
            out.extend(c.to_uppercase());
            out.extend(chars);
        }
    }
    let out: String = out.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        format!("_{out}")
    } else {
        out
    }
}

/// PascalCase → camelCase（路径常量键用）。
pub fn camel(name: &str) -> String {
    let p = pascal(name);
    let mut chars = p.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => p,
    }
}

/// 类型名 / 键名去重分配器：重名加数字后缀（GetUser → GetUser2）。
#[derive(Default)]
pub struct NameAllocator {
    used: HashSet<String>,
}

impl NameAllocator {
    pub fn new() -> Self {
        NameAllocator { used: HashSet::new() }
    }
    pub fn alloc(&mut self, base: String) -> String {
        if self.used.insert(base.clone()) {
            return base;
        }
        let mut n = 2;
        loop {
            let cand = format!("{base}{n}");
            if self.used.insert(cand.clone()) {
                return cand;
            }
            n += 1;
        }
    }
}
