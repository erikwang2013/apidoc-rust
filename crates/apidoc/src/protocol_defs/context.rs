//! Parse context: limits, import resolution, diagnostics.
//!
//! Mirrors `php/src/protocol/ProtocolContext.php`. The context never touches
//! the filesystem itself; a [`Resolver`] supplied by the caller maps a
//! relative path to canonical key + content.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use super::node::{parse_json, Node};
use super::yaml;

#[derive(Clone, Debug)]
pub struct Options {
    pub operation: String,
    pub message: String,
    pub direction: String,
    pub sample: String,
    pub sample_encoding: String,
    /// Unsupported constructs are errors when strict (PHP default true).
    pub strict: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            operation: String::new(),
            message: String::new(),
            direction: String::new(),
            sample: String::new(),
            sample_encoding: String::new(),
            strict: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_file_bytes: usize,
    pub max_lines: usize,
    pub max_imports: usize,
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 2_097_152,
            max_lines: 50_000,
            max_imports: 64,
            max_depth: 32,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Diagnostic {
    pub path: String,
    pub line: i64,
    pub column: i64,
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Validation {
    pub ok: bool,
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
}

/// Caller-supplied document loader for `$ref` / import targets.
///
/// `path` is the normalized path relative to the defining file. Returning
/// `None` means the caller refused it (outside allowed roots, missing, or over
/// its own limits); the parser records the PHP-compatible diagnostic.
pub trait Resolver {
    fn read(&mut self, path: &str) -> Option<(String, String)>;

    /// Detail for the last `read` refusal when the path was valid but the file
    /// could not be loaded (size cap, IO, encoding). Cleared by every `read`.
    fn failure(&mut self) -> Option<String> {
        None
    }
}

pub struct NoResolver;

impl Resolver for NoResolver {
    fn read(&mut self, _path: &str) -> Option<(String, String)> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct LoadError {
    pub message: String,
    pub line: i64,
    pub column: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Loader {
    Json,
    JsonOrYaml,
}

impl Loader {
    /// `null` from the PHP callables means "not a document" (scalar or parse error).
    pub fn load(self, content: &str) -> Result<Node, LoadError> {
        match self {
            Loader::Json => parse_json(content)
                .map_err(|message| LoadError {
                    message,
                    line: 0,
                    column: 0,
                })
                .and_then(require_document),
            Loader::JsonOrYaml => {
                let trimmed = content.trim_start();
                let node = if trimmed.starts_with('{') || trimmed.starts_with('[') {
                    parse_json(content).map_err(|message| LoadError {
                        message,
                        line: 0,
                        column: 0,
                    })?
                } else {
                    yaml::parse(content).map_err(|e| LoadError {
                        message: e.message,
                        line: e.line,
                        column: e.column,
                    })?
                };
                require_document(node)
            }
        }
    }

    /// YAML/JSON root must be a mapping/sequence, as PHP `is_array` requires.
    pub fn load_document(self, content: &str) -> Option<Node> {
        self.load(content).ok()
    }
}

fn require_document(node: Node) -> Result<Node, LoadError> {
    if node.is_array_like() {
        Ok(node)
    } else {
        Err(LoadError {
            message: "document root must be an object".to_string(),
            line: 1,
            column: 1,
        })
    }
}

pub struct Context<'a> {
    file: String,
    options: Options,
    limits: Limits,
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
    imports: Vec<String>,
    visited: HashSet<String>,
    depth: usize,
    stack: Vec<(String, usize)>,
    resolver: &'a mut dyn Resolver,
    contents: HashMap<String, String>,
    documents: HashMap<String, Node>,
    expanded: HashMap<String, Node>,
}

impl<'a> Context<'a> {
    pub fn new(
        file: String,
        content: String,
        options: Options,
        limits: Limits,
        resolver: &'a mut dyn Resolver,
    ) -> Self {
        let mut contents = HashMap::new();
        contents.insert(file.clone(), content);
        let mut visited = HashSet::new();
        visited.insert(file.clone());
        Self {
            file,
            options,
            limits,
            errors: Vec::new(),
            warnings: Vec::new(),
            imports: Vec::new(),
            visited,
            depth: 0,
            stack: Vec::new(),
            resolver,
            contents,
            documents: HashMap::new(),
            expanded: HashMap::new(),
        }
    }

    pub fn file(&self) -> &str {
        &self.file
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn errors(&self) -> &[Diagnostic] {
        &self.errors
    }

    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// Read the current document under byte/line limits.
    pub fn read(&mut self) -> Option<String> {
        let content = self.contents.get(&self.file)?.clone();
        if content.len() > self.limits.max_file_bytes {
            self.error("file exceeds max_file_bytes", 0, 0);
            return None;
        }
        if content.bytes().filter(|b| *b == b'\n').count() + 1 > self.limits.max_lines {
            self.error("file exceeds max_lines", 0, 0);
            return None;
        }
        Some(content)
    }

    /// Resolve an import path relative to the current file; resolver owns root checks.
    pub fn resolve(&mut self, import_path: &str) -> Option<String> {
        if import_path.is_empty() || import_path.contains('\0') {
            return None;
        }
        if import_path.starts_with('/') {
            return None;
        }
        let bytes = import_path.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/')
        {
            return None;
        }
        let dir = match self.file.rfind('/') {
            Some(index) => &self.file[..index],
            None => ".",
        };
        let candidate = normalize_path(&format!("{dir}/{import_path}"))?;
        let Some((key, content)) = self.resolver.read(&candidate) else {
            if let Some(message) = self.resolver.failure() {
                self.error(&message, 0, 0);
            }
            return None;
        };
        self.contents.insert(key.clone(), content);
        Some(key)
    }

    pub fn push(&mut self, realpath: String) -> bool {
        if self.visited.contains(&realpath) {
            self.warning("import cycle skipped", 0, 0);
            return false;
        }
        if self.imports.len() >= self.limits.max_imports {
            self.error("max_imports exceeded", 0, 0);
            return false;
        }
        if self.depth >= self.limits.max_depth {
            self.error("max_depth exceeded", 0, 0);
            return false;
        }
        self.visited.insert(realpath.clone());
        self.imports.push(realpath.clone());
        self.stack.push((self.file.clone(), self.depth));
        self.file = realpath;
        self.depth += 1;
        true
    }

    pub fn pop(&mut self) {
        if let Some((file, depth)) = self.stack.pop() {
            self.file = file;
            self.depth = depth;
        }
    }

    pub fn enter_file(&mut self, file: String) {
        self.stack.push((self.file.clone(), self.depth));
        self.file = file;
    }

    pub fn leave_file(&mut self) {
        if let Some((file, depth)) = self.stack.pop() {
            self.file = file;
            self.depth = depth;
        }
    }

    pub fn error(&mut self, message: &str, line: i64, column: i64) {
        self.errors.push(self.entry("error", message, line, column));
    }

    pub fn warning(&mut self, message: &str, line: i64, column: i64) {
        self.warnings
            .push(self.entry("warning", message, line, column));
    }

    /// Unsupported syntax: error in strict mode, warning otherwise.
    pub fn unsupported(&mut self, message: &str, line: i64, column: i64) {
        if self.options.strict {
            self.error(message, line, column);
        } else {
            self.warning(message, line, column);
        }
    }

    fn entry(&self, severity: &str, message: &str, line: i64, column: i64) -> Diagnostic {
        Diagnostic {
            path: String::new(),
            line,
            column,
            severity: severity.to_string(),
            message: message.to_string(),
        }
    }

    pub fn imports(&self) -> &[String] {
        &self.imports
    }

    pub fn document(&self, key: &str) -> Option<&Node> {
        self.documents.get(key)
    }

    pub fn remember_document(&mut self, key: String, document: Node) {
        self.documents.insert(key, document);
    }

    pub fn expanded_document(&self, key: &str) -> Option<&Node> {
        self.expanded.get(key)
    }

    pub fn remember_expanded(&mut self, key: String, document: Node) {
        self.expanded.insert(key, document);
    }
}

fn normalize_path(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if joined.is_empty() {
        return None;
    }
    Some(if absolute {
        format!("/{joined}")
    } else {
        joined
    })
}
