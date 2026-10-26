//! Ordered JSON/YAML value used by the definition parsers.
//!
//! `serde_json::Map` is a `BTreeMap` in this workspace (the `preserve_order`
//! feature would change `api.json` output globally), so definition parsing
//! keeps its own insertion-ordered object instead. Field order affects the UIC
//! `messages[].fields[]` arrays and must match the PHP reference.

use std::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

impl Node {
    pub fn object() -> Self {
        Node::Object(Vec::new())
    }

    pub fn array() -> Self {
        Node::Array(Vec::new())
    }

    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Node> {
        match self {
            Node::Object(entries) => entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// PHP array assignment: replace in place, otherwise append.
    pub fn set(&mut self, key: impl Into<String>, value: Node) {
        let key = key.into();
        if let Node::Object(entries) = self {
            if let Some((_, existing)) = entries.iter_mut().find(|(k, _)| *k == key) {
                *existing = value;
            } else {
                entries.push((key, value));
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Node> {
        if let Node::Object(entries) = self {
            if let Some(index) = entries.iter().position(|(k, _)| k == key) {
                return Some(entries.remove(index).1);
            }
        }
        None
    }

    pub fn push(&mut self, value: Node) {
        if let Node::Array(items) = self {
            items.push(value);
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Node)]> {
        match self {
            Node::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Node]> {
        match self {
            Node::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Node::Object(_))
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Node::Array(_))
    }

    /// PHP `is_array`: object or list.
    pub fn is_array_like(&self) -> bool {
        matches!(self, Node::Object(_) | Node::Array(_))
    }

    /// PHP list check (`$data === [] || array_keys(...) === range(...)`).
    pub fn is_list(&self) -> bool {
        match self {
            Node::Array(_) => true,
            Node::Object(entries) => {
                entries.is_empty()
                    || entries
                        .iter()
                        .enumerate()
                        .all(|(index, (key, _))| key == &index.to_string())
            }
            _ => false,
        }
    }

    pub fn keys(&self) -> Vec<&str> {
        match self {
            Node::Object(entries) => entries.iter().map(|(k, _)| k.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn first_key(&self) -> Option<&str> {
        match self {
            Node::Object(entries) => entries.first().map(|(k, _)| k.as_str()),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Node::Object(entries) => entries.len(),
            Node::Array(items) => items.len(),
            Node::Str(s) => s.len(),
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// PHP `(string)$value` for the scalar cases the adapters rely on.
    pub fn scalar_string(&self) -> String {
        match self {
            Node::Null => String::new(),
            Node::Bool(true) => "1".to_string(),
            Node::Bool(false) => String::new(),
            Node::Int(n) => n.to_string(),
            // PHP (string)3.0 is "3"; version_string keeps the ".0" for OpenAPI.
            Node::Float(f) => format!("{f}"),
            Node::Str(s) => s.clone(),
            Node::Array(_) => "Array".to_string(),
            Node::Object(_) => "Array".to_string(),
        }
    }

    /// PHP `var_export(float, true)` for version numbers: 2.0 stays "2.0".
    pub fn version_string(&self) -> String {
        match self {
            Node::Float(f) => format_float(*f),
            _ => self.scalar_string(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Node::Null => serde_json::Value::Null,
            Node::Bool(b) => serde_json::Value::Bool(*b),
            Node::Int(n) => serde_json::Value::Number((*n).into()),
            Node::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Node::Str(s) => serde_json::Value::String(s.clone()),
            Node::Array(items) => {
                serde_json::Value::Array(items.iter().map(Node::to_json).collect())
            }
            Node::Object(entries) => {
                let mut map = serde_json::Map::new();
                for (key, value) in entries {
                    map.insert(key.clone(), value.to_json());
                }
                serde_json::Value::Object(map)
            }
        }
    }

    /// `array_merge` for string-keyed objects: later keys overwrite, new keys append.
    pub fn merge(base: &Node, overlay: &Node) -> Node {
        let mut out = match base {
            Node::Object(entries) => Node::Object(entries.clone()),
            _ => Node::object(),
        };
        if let Node::Object(entries) = overlay {
            for (key, value) in entries {
                out.set(key.clone(), value.clone());
            }
        }
        out
    }

    /// PHP `empty()` used throughout the adapters.
    pub fn truthy(&self) -> bool {
        match self {
            Node::Null => false,
            Node::Bool(b) => *b,
            Node::Int(n) => *n != 0,
            Node::Float(f) => *f != 0.0,
            Node::Str(s) => !s.is_empty() && s != "0",
            Node::Array(items) => !items.is_empty(),
            Node::Object(entries) => !entries.is_empty(),
        }
    }

    pub fn strict_eq(&self, other: &Node) -> bool {
        match (self, other) {
            (Node::Null, Node::Null) => true,
            (Node::Bool(a), Node::Bool(b)) => a == b,
            (Node::Int(a), Node::Int(b)) => a == b,
            (Node::Float(a), Node::Float(b)) => a == b,
            (Node::Str(a), Node::Str(b)) => a == b,
            (Node::Array(a), Node::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.strict_eq(y))
            }
            (Node::Object(a), Node::Object(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|((ka, va), (kb, vb))| ka == kb && va.strict_eq(vb))
            }
            _ => false,
        }
    }

    /// PHP loose equality for `const` checks (`$data != $schema['const']`).
    pub fn loose_eq(&self, other: &Node) -> bool {
        match (self, other) {
            (Node::Int(a), Node::Float(b)) | (Node::Float(b), Node::Int(a)) => *a as f64 == *b,
            (Node::Str(a), Node::Int(b)) | (Node::Int(b), Node::Str(a)) => {
                a.trim().parse::<i64>().is_ok_and(|n| n == *b)
            }
            (Node::Str(a), Node::Float(b)) | (Node::Float(b), Node::Str(a)) => {
                a.trim().parse::<f64>().is_ok_and(|n| n == *b)
            }
            (Node::Null, Node::Str(s)) | (Node::Str(s), Node::Null) => s.is_empty(),
            _ => self.strict_eq(other),
        }
    }
}

pub fn format_float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_json())
    }
}

/// PHP `json_last_error_msg()` for the cases serde_json distinguishes.
pub fn php_json_error_message(err: &serde_json::Error) -> &'static str {
    let text = err.to_string();
    if text.contains("control character") {
        "Control character error, possibly incorrectly encoded"
    } else if text.contains("invalid unicode") || text.contains("surrogate") {
        "Malformed UTF-8 characters, possibly incorrectly encoded"
    } else {
        "Syntax error"
    }
}

pub fn parse_json(content: &str) -> Result<Node, String> {
    serde_json::from_str::<Node>(content).map_err(|e| php_json_error_message(&e).to_string())
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NodeVisitor;

        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = Node;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any JSON value")
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(Node::Bool(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(Node::Int(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(i64::try_from(v).map_or_else(|_| Node::Float(v as f64), Node::Int))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(Node::Float(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Node::Str(v.to_string()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(Node::Str(v))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Node::Null)
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Node::Null)
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
                Node::deserialize(d)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element::<Node>()? {
                    items.push(item);
                }
                Ok(Node::Array(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut node = Node::object();
                while let Some((key, value)) = map.next_entry::<String, Node>()? {
                    node.set(key, value);
                }
                Ok(node)
            }
        }

        deserializer.deserialize_any(NodeVisitor)
    }
}
