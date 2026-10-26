//! Token stream ported from `php/src/protocol/Support/TokenStream.php`.

use super::yaml::stripcslashes;

#[derive(Clone, Debug)]
pub struct ParseError {
    pub message: String,
    pub line: i64,
    pub column: i64,
}

pub type Result<T> = std::result::Result<T, ParseError>;

pub struct TokenStream {
    source: Vec<char>,
    pos: usize,
    line: i64,
    column: i64,
}

impl TokenStream {
    pub fn new(source: &str) -> Self {
        Self {
            source: source
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .chars()
                .collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn eof(&mut self) -> bool {
        self.skip_trivia();
        self.pos >= self.source.len()
    }

    pub fn next_char(&mut self) -> String {
        if self.pos >= self.source.len() {
            return String::new();
        }
        let ch = self.source[self.pos];
        self.advance();
        ch.to_string()
    }

    pub fn peek(&mut self, offset: usize) -> String {
        let (save_pos, save_line, save_column) = (self.pos, self.line, self.column);
        self.skip_trivia();
        for _ in 0..offset {
            self.advance();
        }
        let ch = self.source.get(self.pos).copied();
        self.pos = save_pos;
        self.line = save_line;
        self.column = save_column;
        ch.map(|c| c.to_string()).unwrap_or_default()
    }

    pub fn consume(&mut self, chars: &str) -> Option<char> {
        self.skip_trivia();
        let ch = *self.source.get(self.pos)?;
        if !chars.contains(ch) {
            return None;
        }
        self.advance();
        Some(ch)
    }

    pub fn expect(&mut self, expected: char) -> Result<()> {
        self.skip_trivia();
        if self.source.get(self.pos) != Some(&expected) {
            return Err(self.error(&format!("expected \"{expected}\"")));
        }
        self.advance();
        Ok(())
    }

    pub fn peek_identifier(&mut self) -> String {
        let (save_pos, save_line, save_column) = (self.pos, self.line, self.column);
        self.skip_trivia();
        let value = self.scan_identifier();
        self.pos = save_pos;
        self.line = save_line;
        self.column = save_column;
        value
    }

    pub fn identifier(&mut self) -> Result<String> {
        self.skip_trivia();
        let value = self.scan_identifier();
        if value.is_empty() {
            return Err(self.error("expected identifier"));
        }
        for _ in 0..value.chars().count() {
            self.advance();
        }
        Ok(value)
    }

    pub fn number(&mut self) -> Result<String> {
        self.skip_trivia();
        let start = self.pos;
        let len = self.source.len();
        if self.pos < len && self.source[self.pos] == '-' {
            self.advance();
        }
        let digit_start = self.pos;
        while self.pos < len && self.source[self.pos].is_ascii_digit() {
            self.advance();
        }
        if self.pos == digit_start {
            self.pos = start;
            self.line = 1;
            self.column = 1;
            // Rewind line/column correctly for multi-line leading trivia.
            self.reposition(start);
            return Err(self.error("expected number"));
        }
        if self.pos + 1 < len
            && self.source[self.pos] == '.'
            && self.source[self.pos + 1].is_ascii_digit()
        {
            self.advance();
            while self.pos < len && self.source[self.pos].is_ascii_digit() {
                self.advance();
            }
        }
        if self.pos < len && (self.source[self.pos] == 'e' || self.source[self.pos] == 'E') {
            let exponent_start = self.pos;
            self.advance();
            if self.pos < len && (self.source[self.pos] == '+' || self.source[self.pos] == '-') {
                self.advance();
            }
            let exponent_digits = self.pos;
            while self.pos < len && self.source[self.pos].is_ascii_digit() {
                self.advance();
            }
            if self.pos == exponent_digits {
                self.pos = exponent_start;
                self.reposition(exponent_start);
            }
        }
        Ok(self.source[start..self.pos].iter().collect())
    }

    pub fn string(&mut self) -> Result<String> {
        self.skip_trivia();
        let quote = *self.source.get(self.pos).unwrap_or(&'\0');
        if quote != '"' && quote != '\'' {
            return Err(self.error("expected string"));
        }
        self.advance();
        let mut value = String::new();
        while self.pos < self.source.len() {
            let ch = self.source[self.pos];
            if ch == '\\' {
                self.advance();
                if self.pos >= self.source.len() {
                    return Err(self.error("unterminated string escape"));
                }
                let escaped: String = std::iter::once('\\')
                    .chain(std::iter::once(self.source[self.pos]))
                    .collect();
                value.push_str(&stripcslashes(&escaped));
                self.advance();
                continue;
            }
            if ch == quote {
                self.advance();
                return Ok(value);
            }
            value.push(ch);
            self.advance();
        }
        Err(self.error("unterminated string"))
    }

    pub fn skip_trivia(&mut self) {
        while self.pos < self.source.len() {
            let ch = self.source[self.pos];
            if ch == ' ' || ch == '\t' || ch == '\n' {
                self.advance();
                continue;
            }
            if ch == '#' || (ch == '/' && self.peek_raw(1) == '/') {
                while self.pos < self.source.len() && self.source[self.pos] != '\n' {
                    self.advance();
                }
                continue;
            }
            if ch == '/' && self.peek_raw(1) == '*' {
                self.advance();
                self.advance();
                while self.pos < self.source.len()
                    && !(self.source[self.pos] == '*' && self.peek_raw(1) == '/')
                {
                    self.advance();
                }
                if self.pos >= self.source.len() {
                    // Truncated block comment: stop at EOF; the next token call
                    // reports the unexpected end instead of panicking.
                    return;
                }
                self.advance();
                self.advance();
                continue;
            }
            break;
        }
    }

    pub fn line(&self) -> i64 {
        self.line
    }

    pub fn column(&self) -> i64 {
        self.column
    }

    pub fn error(&self, message: &str) -> ParseError {
        ParseError {
            message: message.to_string(),
            line: self.line,
            column: self.column,
        }
    }

    fn scan_identifier(&self) -> String {
        let mut index = self.pos;
        if index >= self.source.len() {
            return String::new();
        }
        let first = self.source[index];
        if !(first.is_ascii_alphabetic() || first == '_') {
            return String::new();
        }
        index += 1;
        while index < self.source.len() {
            let ch = self.source[index];
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
                index += 1;
            } else {
                break;
            }
        }
        self.source[self.pos..index].iter().collect()
    }

    fn peek_raw(&self, offset: usize) -> char {
        self.source.get(self.pos + offset).copied().unwrap_or('\0')
    }

    fn advance(&mut self) {
        if self.pos >= self.source.len() {
            return;
        }
        if self.source[self.pos] == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        self.pos += 1;
    }

    /// Recompute line/column from the start after a failed speculative scan.
    fn reposition(&mut self, target: usize) {
        let mut line = 1;
        let mut column = 1;
        for index in 0..target.min(self.source.len()) {
            if self.source[index] == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        self.line = line;
        self.column = column;
        self.pos = target;
    }
}
