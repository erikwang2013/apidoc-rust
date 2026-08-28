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
        }
    }
}

/// Request header documentation. Not produced by any M1 macro; reserved.
#[derive(Serialize)]
pub struct DocHeader {
    pub name: &'static str,
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
            }
        }
        endpoints
    }
}
