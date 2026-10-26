//! YAML subset ported from `php/src/protocol/Support/Yaml.php`.
//!
//! Supports mapping/sequence/scalars/inline list+map/comments/block scalars.
//! Anchors, aliases, tags, multiple documents and tab indentation are rejected
//! with the same `YAML: <message>` diagnostics as PHP.

use super::node::Node;

#[derive(Clone, Debug)]
pub struct YamlError {
    pub message: String,
    pub line: i64,
    pub column: i64,
}

impl YamlError {
    fn new(message: &str, line: usize) -> Self {
        Self {
            message: format!("YAML: {message}"),
            line: line as i64,
            column: 0,
        }
    }
}

type Result<T> = std::result::Result<T, YamlError>;

#[derive(Clone, Debug)]
struct Line {
    indent: usize,
    content: String,
    line: usize,
}

pub fn parse(source: &str) -> Result<Node> {
    let lines = split_lines(source)?;
    if lines.is_empty() {
        return Ok(Node::object());
    }
    let mut index = 0;
    let result = parse_block(&lines, &mut index, lines[0].indent, 0)?;
    if index < lines.len() {
        return Err(YamlError::new("unexpected content", lines[index].line));
    }
    Ok(match result {
        Some(node @ (Node::Object(_) | Node::Array(_))) => node,
        _ => Node::object(),
    })
}

fn split_lines(source: &str) -> Result<Vec<Line>> {
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = Vec::new();
    for (number, raw) in source.split('\n').enumerate() {
        let spaces = raw.len() - raw.trim_start_matches(' ').len();
        if raw[spaces..].starts_with('\t') {
            return Err(YamlError::new(
                "tab indentation is not supported",
                number + 1,
            ));
        }
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == "---" || trimmed == "..." {
            return Err(YamlError::new(
                "multi-document YAML is not supported",
                number + 1,
            ));
        }
        lines.push(Line {
            indent: spaces,
            content: raw[spaces..].trim_end().to_string(),
            line: number + 1,
        });
    }
    Ok(lines)
}

fn parse_block(
    lines: &[Line],
    index: &mut usize,
    indent: usize,
    depth: usize,
) -> Result<Option<Node>> {
    if depth > 512 {
        let line = lines.get(*index).map_or(0, |l| l.line);
        return Err(YamlError::new("nesting limit reached", line));
    }
    if *index >= lines.len() || lines[*index].indent < indent {
        return Ok(None);
    }
    if lines[*index].indent > indent {
        return Err(YamlError::new("bad indentation", lines[*index].line));
    }
    if is_sequence(&lines[*index].content) {
        parse_sequence(lines, index, indent, depth).map(Some)
    } else {
        parse_mapping(lines, index, indent, depth).map(Some)
    }
}

fn parse_mapping(lines: &[Line], index: &mut usize, indent: usize, depth: usize) -> Result<Node> {
    let mut result = Node::object();
    while *index < lines.len() && lines[*index].indent == indent {
        let line = lines[*index].clone();
        if is_sequence(&line.content) {
            return Err(YamlError::new("unexpected sequence item", line.line));
        }
        let (key, value) = split_key(&line.content, line.line)?;
        *index += 1;
        let node = if value.is_empty() {
            if *index < lines.len() && lines[*index].indent > indent {
                let child_indent = lines[*index].indent;
                parse_block(lines, index, child_indent, depth + 1)?.unwrap_or(Node::Null)
            } else {
                Node::Null
            }
        } else if value == "|" || value == ">" {
            Node::Str(parse_block_scalar(lines, index, indent, &value))
        } else {
            parse_scalar(&value, line.line, depth + 1)?
        };
        result.set(key, node);
    }
    if *index < lines.len() && lines[*index].indent > indent {
        return Err(YamlError::new("bad indentation", lines[*index].line));
    }
    Ok(result)
}

fn parse_sequence(lines: &[Line], index: &mut usize, indent: usize, depth: usize) -> Result<Node> {
    let mut result = Node::array();
    while *index < lines.len()
        && lines[*index].indent == indent
        && is_sequence(&lines[*index].content)
    {
        let line = lines[*index].clone();
        let rest = line.content[1..].trim_start().to_string();
        *index += 1;
        if rest.is_empty() {
            if *index < lines.len() && lines[*index].indent > indent {
                let child_indent = lines[*index].indent;
                result.push(
                    parse_block(lines, index, child_indent, depth + 1)?.unwrap_or(Node::Null),
                );
            } else {
                result.push(Node::Null);
            }
            continue;
        }
        if looks_like_key(&rest) {
            let (key, value) = split_key(&rest, line.line)?;
            let mut item = Node::object();
            let node = if value.is_empty() {
                if *index < lines.len() && lines[*index].indent > indent {
                    let child_indent = lines[*index].indent;
                    parse_block(lines, index, child_indent, depth + 1)?.unwrap_or(Node::Null)
                } else {
                    Node::Null
                }
            } else if value == "|" || value == ">" {
                Node::Str(parse_block_scalar(lines, index, indent, &value))
            } else {
                parse_scalar(&value, line.line, depth + 1)?
            };
            item.set(key, node);
            if *index < lines.len() && lines[*index].indent > indent {
                let extra_indent = lines[*index].indent;
                let extra = parse_mapping(lines, index, extra_indent, depth + 1)?;
                if let Node::Object(entries) = extra {
                    for (k, v) in entries {
                        item.set(k, v);
                    }
                }
            }
            result.push(item);
        } else {
            result.push(parse_scalar(&rest, line.line, depth + 1)?);
            if *index < lines.len() && lines[*index].indent > indent {
                return Err(YamlError::new("bad indentation", lines[*index].line));
            }
        }
    }
    Ok(result)
}

fn parse_block_scalar(
    lines: &[Line],
    index: &mut usize,
    parent_indent: usize,
    style: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut min_indent: Option<usize> = None;
    let start = *index;
    while *index < lines.len() && lines[*index].indent > parent_indent {
        let line = &lines[*index];
        let min = *min_indent.get_or_insert(line.indent);
        parts.push(format!(
            "{}{}",
            " ".repeat(line.indent.saturating_sub(min)),
            line.content
        ));
        *index += 1;
    }
    if *index == start {
        return String::new();
    }
    parts.join(if style == "|" { "\n" } else { " " })
}

fn is_sequence(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

fn looks_like_key(content: &str) -> bool {
    find_top_level_colon(content).is_some()
}

fn split_key(content: &str, line: usize) -> Result<(String, String)> {
    let pos = find_top_level_colon(content)
        .ok_or_else(|| YamlError::new("expected \"key: value\"", line))?;
    let raw_key = content[..pos].trim();
    if raw_key.is_empty() {
        return Err(YamlError::new("empty key", line));
    }
    let key = if is_quoted(raw_key) {
        parse_scalar(raw_key, line, 0)?.scalar_string()
    } else {
        raw_key.to_string()
    };
    if key.starts_with('&') || key.starts_with('*') || key.starts_with('!') {
        return Err(YamlError::new(
            "anchors, aliases and tags are not supported",
            line,
        ));
    }
    Ok((key, content[pos + 1..].trim().to_string()))
}

fn find_top_level_colon(content: &str) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut quote = 0u8;
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if quote != 0 {
            if ch == b'\\' {
                i += 2;
                continue;
            }
            if ch == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        match ch {
            b'"' | b'\'' => quote = ch,
            b'[' | b'{' => depth += 1,
            b']' | b'}' => depth = depth.saturating_sub(1),
            b':' if depth == 0 => {
                if i + 1 >= bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t' {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_scalar(value: &str, line: usize, depth: usize) -> Result<Node> {
    if depth > 256 {
        return Err(YamlError::new("nesting limit reached", line));
    }
    let value = strip_comment(value);
    let value = value.trim();
    if value.is_empty() {
        return Ok(Node::Str(String::new()));
    }
    if is_quoted(value) {
        return Ok(Node::Str(unquote(value)));
    }
    let first = value.as_bytes()[0];
    if first == b'&' || first == b'*' || first == b'!' {
        return Err(YamlError::new(
            "anchors, aliases and tags are not supported",
            line,
        ));
    }
    if first == b'[' {
        return parse_inline_list(value, line, depth + 1);
    }
    if first == b'{' {
        return parse_inline_map(value, line, depth + 1);
    }
    let lower = value.to_lowercase();
    if lower == "null" || value == "~" {
        return Ok(Node::Null);
    }
    if lower == "true" || lower == "false" {
        return Ok(Node::Bool(lower == "true"));
    }
    if is_integer(value) {
        if let Ok(n) = value.parse::<i64>() {
            return Ok(Node::Int(n));
        }
    }
    if is_float(value) {
        if let Ok(f) = value.parse::<f64>() {
            return Ok(Node::Float(f));
        }
    }
    Ok(Node::Str(value.to_string()))
}

/// PHP `substr($value, 1, -1)`: truncated input yields "" instead of panicking.
fn inline_inner(value: &str) -> &str {
    value
        .get(1..value.len().saturating_sub(1))
        .unwrap_or("")
        .trim()
}

fn parse_inline_list(value: &str, line: usize, depth: usize) -> Result<Node> {
    let inner = inline_inner(value);
    if inner.is_empty() {
        return Ok(Node::array());
    }
    let mut result = Node::array();
    for item in split_inline(inner) {
        result.push(parse_scalar(&item, line, depth)?);
    }
    Ok(result)
}

fn parse_inline_map(value: &str, line: usize, depth: usize) -> Result<Node> {
    let inner = inline_inner(value);
    if inner.is_empty() {
        return Ok(Node::object());
    }
    let mut result = Node::object();
    for pair in split_inline(inner) {
        let (key, item_value) = split_key(&pair, line)?;
        let node = if item_value.is_empty() {
            Node::Null
        } else {
            parse_scalar(&item_value, line, depth + 1)?
        };
        result.set(key, node);
    }
    Ok(result)
}

fn split_inline(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut buffer = String::new();
    let mut quote: Option<char> = None;
    let mut depth = 0usize;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            buffer.push(ch);
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    buffer.push(next);
                }
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                buffer.push(ch);
            }
            '[' | '{' => {
                depth += 1;
                buffer.push(ch);
            }
            ']' | '}' => {
                depth = depth.saturating_sub(1);
                buffer.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(buffer.trim().to_string());
                buffer.clear();
            }
            _ => buffer.push(ch),
        }
    }
    if !buffer.trim().is_empty() {
        parts.push(buffer.trim().to_string());
    }
    parts
}

fn is_quoted(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
}

fn unquote(value: &str) -> String {
    let quote = value.as_bytes()[0];
    let inner = &value[1..value.len() - 1];
    if quote == b'\'' {
        inner.replace("''", "'")
    } else {
        stripcslashes(inner)
    }
}

fn strip_comment(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut quote = 0u8;
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if quote != 0 {
            if ch == b'\\' {
                i += 2;
                continue;
            }
            if ch == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if ch == b'"' || ch == b'\'' {
            quote = ch;
        } else if ch == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return value[..i].trim_end().to_string();
        }
        i += 1;
    }
    value.to_string()
}

fn is_integer(value: &str) -> bool {
    let rest = value.strip_prefix('-').unwrap_or(value);
    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
}

fn is_float(value: &str) -> bool {
    let rest = value.strip_prefix('-').unwrap_or(value);
    let (mantissa, exponent) = match rest.find(['e', 'E']) {
        Some(index) => (&rest[..index], Some(&rest[index + 1..])),
        None => (rest, None),
    };
    let (whole, fraction) = match mantissa.find('.') {
        Some(index) => (&mantissa[..index], Some(&mantissa[index + 1..])),
        None => (mantissa, None),
    };
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    if let Some(exponent) = exponent {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if exponent.is_empty() || !exponent.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    // PHP only accepts decimals with a fraction or an exponent.
    fraction.is_some() || exponent.is_some()
}

/// PHP `stripcslashes` for the escape forms used by YAML/proto/graphql fixtures.
pub fn stripcslashes(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            break;
        };
        match next {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'v' => out.push('\u{0b}'),
            'f' => out.push('\u{0c}'),
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            '$' => out.push('$'),
            '0'..='7' => {
                let mut digits = String::from(next);
                for _ in 0..2 {
                    if matches!(chars.peek(), Some('0'..='7')) {
                        digits.push(chars.next().unwrap());
                    }
                }
                if let Ok(code) = u32::from_str_radix(&digits, 8) {
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                    }
                }
            }
            'x' => {
                let mut digits = String::new();
                for _ in 0..2 {
                    if matches!(chars.peek(), Some(c) if c.is_ascii_hexdigit()) {
                        digits.push(chars.next().unwrap());
                    }
                }
                if let Ok(code) = u32::from_str_radix(&digits, 16) {
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                    }
                }
            }
            other => out.push(other),
        }
    }
    out
}
