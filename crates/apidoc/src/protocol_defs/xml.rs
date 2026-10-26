//! Minimal safe XML DOM on top of quick-xml.
//!
//! Equivalent to `php/src/protocol/Support/SafeXml.php`: DOCTYPE is rejected,
//! entities are never expanded (quick-xml only decodes predefined/numeric
//! references), nesting and node counts are capped.

use quick_xml::events::Event;
use quick_xml::Reader;

const MAX_DEPTH: usize = 256;
const MAX_NODES: usize = 200_000;

#[derive(Clone, Debug)]
pub struct XmlError {
    pub message: String,
    pub line: i64,
    pub column: i64,
}

#[derive(Clone, Debug, Default)]
pub struct XmlNode {
    /// Local name (namespace prefix stripped), matching DOM `localName`.
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlNode>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct XmlDoc {
    pub root: XmlNode,
}

impl XmlNode {
    pub fn attr(&self, name: &str) -> &str {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map_or("", |(_, value)| value.as_str())
    }

    pub fn first(&self, local_name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|child| child.name == local_name)
    }

    pub fn all(&self, local_name: &str) -> Vec<&XmlNode> {
        self.children
            .iter()
            .filter(|child| child.name == local_name)
            .collect()
    }

    pub fn first_element(&self) -> Option<&XmlNode> {
        self.children.first()
    }

    pub fn elements(&self) -> impl Iterator<Item = &XmlNode> {
        self.children.iter()
    }

    pub fn text_content(&self) -> String {
        let mut text = self.text.clone();
        for child in &self.children {
            text.push_str(&child.text_content());
        }
        text
    }
}

pub fn load(xml: &str) -> Result<XmlDoc, XmlError> {
    if xml.to_ascii_lowercase().contains("<!doctype") {
        return Err(XmlError {
            message: "DOCTYPE is not allowed".to_string(),
            line: 0,
            column: 0,
        });
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root: Option<XmlNode> = None;
    let mut nodes = 0usize;
    loop {
        let position = reader.buffer_position();
        let event = reader
            .read_event()
            .map_err(|err| xml_error(xml, position, err.to_string()))?;
        match event {
            Event::Start(ref start) => {
                nodes += 1;
                if nodes > MAX_NODES {
                    return Err(xml_error(
                        xml,
                        position,
                        "XML node limit exceeded".to_string(),
                    ));
                }
                if stack.len() >= MAX_DEPTH {
                    return Err(xml_error(
                        xml,
                        position,
                        "XML nesting limit exceeded".to_string(),
                    ));
                }
                let node = element(start, xml, position)?;
                if stack.is_empty() && root.is_some() {
                    return Err(xml_error(
                        xml,
                        position,
                        "multiple root elements".to_string(),
                    ));
                }
                stack.push(node);
            }
            Event::Empty(ref start) => {
                nodes += 1;
                if nodes > MAX_NODES {
                    return Err(xml_error(
                        xml,
                        position,
                        "XML node limit exceeded".to_string(),
                    ));
                }
                let node = element(start, xml, position)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else if root.is_none() {
                    root = Some(node);
                } else {
                    return Err(xml_error(
                        xml,
                        position,
                        "multiple root elements".to_string(),
                    ));
                }
            }
            Event::End(_) => {
                let node = stack.pop().ok_or_else(|| {
                    xml_error(xml, position, "unexpected closing tag".to_string())
                })?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    root = Some(node);
                }
            }
            Event::Text(ref text) => {
                let value = text
                    .unescape()
                    .map_err(|_| xml_error(xml, position, "undefined entity".to_string()))?;
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&value);
                } else if !value.trim().is_empty() {
                    return Err(xml_error(
                        xml,
                        position,
                        "content outside the root element".to_string(),
                    ));
                }
            }
            Event::CData(ref text) => {
                let raw = text.clone().into_inner();
                let value = String::from_utf8_lossy(&raw);
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&value);
                }
            }
            Event::Eof => break,
            Event::DocType(_) => {
                return Err(XmlError {
                    message: "DOCTYPE is not allowed".to_string(),
                    line: 0,
                    column: 0,
                });
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(xml_error(
            xml,
            reader.buffer_position(),
            "unclosed element".to_string(),
        ));
    }
    let root = root.ok_or_else(|| XmlError {
        message: "XML parse error: Document is empty".to_string(),
        line: 1,
        column: 1,
    })?;
    Ok(XmlDoc { root })
}

fn element(
    start: &quick_xml::events::BytesStart<'_>,
    xml: &str,
    position: u64,
) -> Result<XmlNode, XmlError> {
    let qualified = String::from_utf8_lossy(start.name().as_ref()).into_owned();
    let name = local_name(&qualified).to_string();
    let mut attrs = Vec::new();
    for attr in start.attributes() {
        let attr = attr.map_err(|err| xml_error(xml, position, err.to_string()))?;
        let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
        let value = attr
            .unescape_value()
            .map_err(|_| xml_error(xml, position, "undefined entity".to_string()))?;
        attrs.push((key, value.into_owned()));
    }
    Ok(XmlNode {
        name,
        attrs,
        children: Vec::new(),
        text: String::new(),
    })
}

pub fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn xml_error(xml: &str, position: u64, message: String) -> XmlError {
    let index = (position as usize).min(xml.len());
    let index = floor_char_boundary(xml, index);
    let prefix = &xml[..index];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() as i64 + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .map_or(1, |last| last.chars().count() as i64 + 1);
    XmlError {
        message: format!("XML parse error: {message}"),
        line,
        column,
    }
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}
