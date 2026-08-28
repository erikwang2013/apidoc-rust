//! apidoc runtime core: data model, distributed-slice fragment registry and
//! endpoint aggregation. Attribute macros are re-exported from apidoc-macros.
//!
//! Two things to know when consuming this crate:
//! - Registration happens via `linkme::distributed_slice`, so crates that only
//!   register documentation (no other use of this crate) must be linked, not
//!   merely built; call any exported item from the crate root to force it.
//! - The macros expand to paths like `apidoc::DocFragment`, so consumers must
//!   depend on `linkme` directly and re-export `distributed_slice` (this crate
//!   already re-exports it for convenience).

pub use apidoc_macros::*;
pub use linkme::distributed_slice;

use serde::Serialize;

/// Collects every `#[apidoc::*]` annotation from all linked crates.
#[distributed_slice]
pub static DOC_FRAGMENTS: [DocFragmentEntry];

/// One annotation: the endpoint id plus the annotated piece of documentation.
/// `seq` is assigned by the macro at expansion time (source order) and is used
/// by `DocRegistry::collect` to restore declaration order, since linkme's
/// linker-section iteration order is not source order.
pub struct DocFragmentEntry {
    pub id: &'static str,
    pub seq: u32,
    pub frag: DocFragment,
}

/// A single annotation payload.
pub enum DocFragment {
    Title(&'static str),
    Desc(&'static str),
    Method(&'static str),
    Url(&'static str),
    Param(DocParam),
    Query(DocParam),
    Returned(DocParam),
    // M3: single-value fragments (later mounts win), list fragments (append),
    // flag fragments (OR), and the ref reference.
    Tag(&'static str),
    Group(&'static str),
    Author(&'static str),
    Header(DocHeader),
    RouteParam(DocParam),
    ResponseStatus(&'static str),
    Success(DocExample),
    Error(DocExample),
    NotDebug,
    Md(&'static str),
    Sort(i32),
    Ref(&'static str),
}

/// A documented example response body (shared by success / error).
#[derive(Clone, Serialize)]
pub struct DocExample {
    /// HTTP status code, kept as a string to align with the PHP version.
    pub code: &'static str,
    /// Raw response body; stored verbatim, not validated as JSON.
    pub example: &'static str,
}

/// A documented parameter (body param, query string field or return field).
#[derive(Clone, Serialize)]
pub struct DocParam {
    pub name: &'static str,
    #[serde(rename = "type")]
    pub ty: &'static str,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mock: Option<&'static str>,
    #[serde(skip_serializing_if = "slice_is_empty")]
    pub children: &'static [DocParam],
}

fn slice_is_empty<T>(s: &[T]) -> bool {
    s.is_empty()
}

/// One documented HTTP endpoint, built by merging fragments with the same id.
#[derive(Serialize)]
pub struct DocEndpoint {
    pub title: String,
    pub desc: String,
    pub url: String,
    pub method: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<DocHeader>,
    pub params: Vec<DocParam>,
    pub querys: Vec<DocParam>,
    pub returned: Vec<DocParam>,
    // M3 fields: all optional, all omitted from JSON when at their default
    // value, so api.json output is unchanged for endpoints that use none of
    // the new annotations.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub route_params: Vec<DocParam>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub response_status: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub success: Vec<DocExample>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub error: Vec<DocExample>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub not_debug: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub md: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub sort: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

impl Default for DocEndpoint {
    fn default() -> Self {
        DocEndpoint {
            title: String::new(),
            desc: String::new(),
            url: String::new(),
            method: "GET".to_string(),
            headers: Vec::new(),
            params: Vec::new(),
            querys: Vec::new(),
            returned: Vec::new(),
            group: String::new(),
            tags: Vec::new(),
            author: String::new(),
            route_params: Vec::new(),
            response_status: Vec::new(),
            success: Vec::new(),
            error: Vec::new(),
            not_debug: false,
            md: String::new(),
            sort: 0,
            r#ref: None,
        }
    }
}

/// Request header documentation, e.g. `#[apidoc::header(name = "X-Token")]`.
#[derive(Clone, Serialize)]
pub struct DocHeader {
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<&'static str>,
}

/// Project-level configuration, combined with endpoints into the final output.
#[derive(Serialize)]
pub struct ApidocConfig {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Final aggregated document (the shape of api.json).
#[derive(Serialize)]
pub struct ApiDoc {
    pub config: ApidocConfig,
    pub endpoints: Vec<DocEndpoint>,
}

/// Collects and merges all registered fragments into per-endpoint documents.
pub struct DocRegistry;

impl DocRegistry {
    pub fn collect() -> Vec<DocEndpoint> {
        // Sort by seq first: linkme's iteration order is linker-defined, not
        // source order. Cross-crate ordering stays linker-arbitrary; seq ties
        // between crates keep linkme's stable order. ponytail: acceptable for
        // M1, revisit if multi-crate endpoint ordering ever matters.
        let mut entries: Vec<&DocFragmentEntry> = DOC_FRAGMENTS.iter().collect();
        entries.sort_by_key(|e| e.seq);
        let mut ids: Vec<&'static str> = Vec::new();
        let mut endpoints: Vec<DocEndpoint> = Vec::new();
        for entry in entries {
            // ponytail: O(n²) linear id lookup; fine for doc-sized inputs, swap
            // to a HashMap<&str, usize> if thousands of endpoints ever appear.
            let idx = match ids.iter().position(|id| *id == entry.id) {
                Some(i) => i,
                None => {
                    ids.push(entry.id);
                    endpoints.push(DocEndpoint::default());
                    endpoints.len() - 1
                }
            };
            let ep = &mut endpoints[idx];
            match &entry.frag {
                DocFragment::Title(t) => ep.title = t.to_string(),
                DocFragment::Desc(d) => ep.desc = d.to_string(),
                DocFragment::Method(m) => ep.method = m.to_string(),
                DocFragment::Url(u) => ep.url = u.to_string(),
                DocFragment::Param(p) => ep.params.push(p.clone()),
                DocFragment::Query(q) => ep.querys.push(q.clone()),
                DocFragment::Returned(r) => ep.returned.push(r.clone()),
                // M3: single-value fields overwrite (later mount wins), lists
                // append, response_status dedups, not_debug ORs, ref overwrites.
                DocFragment::Tag(t) => ep.tags.push(t.to_string()),
                DocFragment::Group(g) => ep.group = g.to_string(),
                DocFragment::Author(a) => ep.author = a.to_string(),
                DocFragment::Header(h) => ep.headers.push(h.clone()),
                DocFragment::RouteParam(p) => ep.route_params.push(p.clone()),
                DocFragment::ResponseStatus(s) => {
                    if !ep.response_status.iter().any(|x| x == s) {
                        ep.response_status.push(s.to_string());
                    }
                }
                DocFragment::Success(e) => ep.success.push(e.clone()),
                DocFragment::Error(e) => ep.error.push(e.clone()),
                DocFragment::NotDebug => ep.not_debug = true,
                DocFragment::Md(m) => ep.md = m.to_string(),
                DocFragment::Sort(n) => ep.sort = *n,
                DocFragment::Ref(r) => ep.r#ref = Some(r.to_string()),
            }
        }
        // Second pass: resolve ref chains (copy the target's `returned` into
        // the referencing endpoint). Runs after every endpoint exists so the
        // target may be declared anywhere, and recursively so chains A→B→C
        // resolve; cycles are cut by the visited set (warned, not copied).
        for i in 0..endpoints.len() {
            if endpoints[i].r#ref.is_some() {
                resolve_ref(i, &mut Vec::new(), &ids, &mut endpoints);
            }
        }
        endpoints
    }
}

/// Copies the ref target's `returned` into `endpoints[idx].returned`.
/// Returns false (no copy) when the target is missing or a cycle is hit.
fn resolve_ref(
    idx: usize,
    visited: &mut Vec<usize>,
    ids: &[&'static str],
    endpoints: &mut [DocEndpoint],
) -> bool {
    if visited.contains(&idx) {
        eprintln!("apidoc: ref cycle at `{}`, skipping", ids[idx]);
        return false;
    }
    visited.push(idx);
    let Some(target) = endpoints[idx].r#ref.as_deref() else {
        return true;
    };
    let Some(j) = find_ref_target(target, ids) else {
        eprintln!("apidoc: ref target `{target}` not found for `{}`", ids[idx]);
        return false;
    };
    if !resolve_ref(j, visited, ids, endpoints) {
        return false;
    }
    endpoints[idx].returned = endpoints[j].returned.clone();
    true
}

/// Locates a ref target: exact id match first, then `::fn_name` suffix match
/// (the PHP-style global reference by function name).
fn find_ref_target(target: &str, ids: &[&'static str]) -> Option<usize> {
    if let Some(j) = ids.iter().position(|id| *id == target) {
        return Some(j);
    }
    let suffix = format!("::{target}");
    let mut hits = ids.iter().enumerate().filter(|(_, id)| id.ends_with(&suffix));
    let (j, _) = hits.next()?;
    if hits.next().is_some() {
        eprintln!("apidoc: ref `{target}` matches multiple endpoints, using `{}`", ids[j]);
    }
    Some(j)
}
