//! JavaScript lexer. Produces tokens for the recursive-descent parser,
//! including template literals and regular-expression literals.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Eof,
    Ident(String),
    Num(f64),
    BigInt(i128),
    Str(String),
    Template { raw: Vec<String>, exprs: Vec<String> },
    Regex { source: String, flags: String },
    Punct(String),
    Keyword(String),
}

impl Tok {
    pub fn is_punct(&self, p: &str) -> bool {
        matches!(self, Tok::Punct(s) if s == p)
    }
    pub fn is_keyword(&self, k: &str) -> bool {
        matches!(self, Tok::Keyword(s) if s == k)
    }
    pub fn describe(&self) -> String {
        match self {
            Tok::Eof => "end of input".into(),
            Tok::Ident(s) => format!("identifier `{s}`"),
            Tok::Num(n) => format!("number `{n}`"),
            Tok::BigInt(n) => format!("bigint `{n}n`"),
            Tok::Str(s) => format!("string `{s}`"),
            Tok::Template { .. } => "template literal".into(),
            Tok::Regex { source, flags } => format!("regex /{source}/{flags}"),
            Tok::Punct(p) => format!("`{p}`"),
            Tok::Keyword(k) => format!("`{k}`"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: Tok,
    pub line: u32,
    pub offset: usize,
    /// True when a line break occurred before this token.
    pub newline_before: bool,
}

#[derive(Debug, Clone)]
pub struct LexError {
    pub message: String,
    pub line: u32,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

const KEYWORDS: &[&str] = &[
    "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do",
    "else", "export", "extends", "finally", "for", "function", "if", "import", "in", "instanceof",
    "let", "new", "return", "super", "switch", "this", "throw", "try", "typeof", "var", "void",
    "while", "with", "yield", "true", "false", "null", "await",
];

/// Words that may appear before a regex literal rather than division.
fn regex_allowed_after_keyword(k: &str) -> bool {
    matches!(
        k,
        "return" | "typeof" | "instanceof" | "in" | "of" | "new" | "delete" | "void" | "throw"
            | "case" | "do" | "else" | "yield" | "await"
    )
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    tokens: Vec<Token>,
    /// Whether each open `(` follows a control keyword, so its `)` can be
    /// followed by a regex literal: `if (x) /re/.test(y)`.
    paren_ctrl: Vec<bool>,
    last_paren_ctrl: bool,
    /// Whether each open `{` starts a statement block rather than an object
    /// literal, so a `}` can be followed by a regex literal.
    brace_block: Vec<bool>,
    last_brace_block: bool,
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    let mut lx = Lexer {
        src: src.as_bytes(),
        pos: 0,
        line: 1,
        tokens: Vec::new(),
        paren_ctrl: Vec::new(),
        last_paren_ctrl: false,
        brace_block: Vec::new(),
        last_brace_block: false,
    };
    lx.run()?;
    Ok(lx.tokens)
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> u8 {
        *self.src.get(self.pos).unwrap_or(&0)
    }
    fn peek_at(&self, n: usize) -> u8 {
        *self.src.get(self.pos + n).unwrap_or(&0)
    }
    fn bump(&mut self) -> u8 {
        let c = self.peek();
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
        }
        c
    }

    /// Decodes the UTF-8 scalar at the cursor without scanning the rest of the
    /// source. `std::str::from_utf8(&src[pos..])` validated the whole tail for
    /// every non-ASCII character, which made lexing quadratic on localized
    /// scripts; this only looks at the 1-4 bytes the character can occupy.
    fn peek_char(&self) -> Option<(char, usize)> {
        if self.pos >= self.src.len() {
            return None;
        }
        for len in 1..=4usize {
            let end = self.pos + len;
            if end > self.src.len() {
                break;
            }
            if let Ok(s) = std::str::from_utf8(&self.src[self.pos..end]) {
                if let Some(ch) = s.chars().next() {
                    return Some((ch, len));
                }
            }
        }
        // Malformed UTF-8: consume a byte so lexing always makes progress.
        Some((char::REPLACEMENT_CHARACTER, 1))
    }
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, LexError> {
        Err(LexError { message: msg.into(), line: self.line })
    }

    fn run(&mut self) -> Result<(), LexError> {
        let mut newline = false;
        loop {
            let c = self.peek();
            if c == 0 {
                self.tokens.push(Token { kind: Tok::Eof, line: self.line, offset: self.pos, newline_before: newline });
                return Ok(());
            }
            if c == b'\n' {
                newline = true;
                self.bump();
                continue;
            }
            if c.is_ascii_whitespace() {
                self.bump();
                continue;
            }
            if c == b'/' && self.peek_at(1) == b'/' {
                while self.peek() != 0 && self.peek() != b'\n' {
                    self.bump();
                }
                continue;
            }
            if c == b'/' && self.peek_at(1) == b'*' {
                self.bump();
                self.bump();
                loop {
                    if self.peek() == 0 {
                        return self.err("unterminated block comment");
                    }
                    if self.peek() == b'*' && self.peek_at(1) == b'/' {
                        self.bump();
                        self.bump();
                        break;
                    }
                    self.bump();
                }
                newline = true;
                continue;
            }

            let line = self.line;
            let token_start = self.pos;
            let kind = if c == b'"' || c == b'\'' {
                Tok::Str(self.read_string(c)?)
            } else if c == b'`' {
                self.read_template()?
            } else if c.is_ascii_digit() || (c == b'.' && self.peek_at(1).is_ascii_digit()) {
                self.read_number()?
            } else if c == b'#' && is_ident_start(self.peek_at(1)) {
                self.bump();
                let name = self.read_ident()?;
                Tok::Ident(format!("#{name}"))
            } else if is_ident_start(c) || c == b'\\' || c >= 0x80 {
                let name = self.read_ident()?;
                if KEYWORDS.contains(&name.as_str()) {
                    Tok::Keyword(name)
                } else {
                    Tok::Ident(name)
                }
            } else if c == b'/' && self.regex_allowed() {
                self.read_regex()?
            } else {
                Tok::Punct(self.read_punct()?)
            };
            match &kind {
                Tok::Punct(p) if p == "(" => {
                    let ctrl = matches!(self.tokens.last().map(|t| &t.kind), Some(Tok::Keyword(k)) if matches!(k.as_str(), "if" | "for" | "while" | "with"));
                    self.paren_ctrl.push(ctrl);
                    self.last_paren_ctrl = false;
                }
                Tok::Punct(p) if p == ")" => {
                    self.last_paren_ctrl = self.paren_ctrl.pop().unwrap_or(false);
                    self.last_brace_block = false;
                }
                Tok::Punct(p) if p == "{" => {
                    let block = match self.tokens.last().map(|t| &t.kind) {
                        None => true,
                        Some(Tok::Punct(q)) => {
                            self.last_paren_ctrl
                                || matches!(q.as_str(), ")" | ";" | "{" | "}" | "=>")
                        }
                        Some(Tok::Keyword(k)) => {
                            matches!(k.as_str(), "else" | "do" | "try" | "finally")
                        }
                        _ => false,
                    };
                    self.brace_block.push(block);
                    self.last_paren_ctrl = false;
                    self.last_brace_block = false;
                }
                Tok::Punct(p) if p == "}" => {
                    self.last_paren_ctrl = false;
                    self.last_brace_block = self.brace_block.pop().unwrap_or(false);
                }
                _ => {
                    self.last_paren_ctrl = false;
                    self.last_brace_block = false;
                }
            }
            self.tokens.push(Token { kind, line, offset: token_start, newline_before: newline });
            newline = false;
        }
    }

    fn regex_allowed(&self) -> bool {
        match self.tokens.last() {
            None => true,
            Some(t) => match &t.kind {
                Tok::Punct(p) if p == ")" => self.last_paren_ctrl,
                Tok::Punct(p) if p == "}" => self.last_brace_block,
                Tok::Punct(p) => matches!(
                    p.as_str(),
                    "(" | "," | ";" | ":" | "[" | "{" | "=" | "!" | "&" | "|" | "?" | "+"
                        | "-" | "*" | "/" | "%" | "^" | "~" | "<" | ">" | "<=" | ">=" | "=="
                        | "!=" | "===" | "!==" | "&&" | "||" | "??" | "=>" | "+=" | "-=" | "*="
                        | "/=" | "%=" | "&=" | "|=" | "^=" | "return"
                ),
                Tok::Keyword(k) => regex_allowed_after_keyword(k),
                _ => false,
            },
        }
    }

    fn read_string(&mut self, quote: u8) -> Result<String, LexError> {
        self.bump();
        let mut out = String::new();
        loop {
            if self.peek() == 0 {
                return self.err("unterminated string literal");
            }
            let c = self.bump();
            if c == quote {
                return Ok(out);
            }
            if c == b'\n' {
                return self.err("unterminated string literal");
            }
            if c == b'\\' {
                self.read_escape(&mut out)?;
            } else if c < 0x80 {
                out.push(c as char);
            } else {
                self.pos -= 1;
                match self.peek_char() {
                    Some((ch, len)) => {
                        out.push(ch);
                        self.pos += len;
                    }
                    None => break,
                }
            }
        }
        self.err("unterminated string literal")
    }

    fn read_escape(&mut self, out: &mut String) -> Result<(), LexError> {
        let c = self.bump();
        match c {
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'v' => out.push('\u{b}'),
            b'0' if !self.peek().is_ascii_digit() => out.push('\0'),
            b'\n' => {}
            b'\r' => {
                if self.peek() == b'\n' {
                    self.bump();
                }
            }
            b'x' => {
                let v = self.read_hex(2)?;
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            b'u' => {
                let v = if self.peek() == b'{' {
                    self.bump();
                    let start = self.pos;
                    while self.peek() != b'}' && self.peek() != 0 {
                        self.bump();
                    }
                    let hex = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
                    if self.peek() == b'}' {
                        self.bump();
                    }
                    u32::from_str_radix(hex, 16).map_err(|_| LexError {
                        message: "invalid unicode escape".into(),
                        line: self.line,
                    })?
                } else {
                    self.read_hex(4)?
                };
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            }
            other => out.push(other as char),
        }
        Ok(())
    }

    fn read_hex(&mut self, n: usize) -> Result<u32, LexError> {
        let start = self.pos;
        for _ in 0..n {
            if !self.peek().is_ascii_hexdigit() {
                return self.err("invalid hex escape");
            }
            self.bump();
        }
        let s = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("0");
        u32::from_str_radix(s, 16).or_else(|_| self.err("invalid hex escape"))
    }

    fn read_number(&mut self) -> Result<Tok, LexError> {
        let start = self.pos;
        if self.peek() == b'0' && matches!(self.peek_at(1), b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
            let radix = match self.peek_at(1) {
                b'x' | b'X' => 16,
                b'b' | b'B' => 2,
                _ => 8,
            };
            self.bump();
            self.bump();
            let digits_start = self.pos;
            while self.peek().is_ascii_alphanumeric() || self.peek() == b'_' {
                self.bump();
            }
            let digits: String = std::str::from_utf8(&self.src[digits_start..self.pos])
                .unwrap_or("0")
                .chars()
                .filter(|c| *c != '_')
                .collect();
            if self.peek() == b'n' {
                self.bump();
                return i128::from_str_radix(&digits, radix)
                    .map(Tok::BigInt)
                    .or_else(|_| self.err("invalid BigInt literal"));
            }
            return i64::from_str_radix(&digits, radix)
                .map(|v| Tok::Num(v as f64))
                .or_else(|_| self.err("invalid numeric literal"));
        }
        while self.peek().is_ascii_digit() || self.peek() == b'_' {
            self.bump();
        }
        let mut fractional = false;
        if self.peek() == b'.' {
            fractional = true;
            self.bump();
            while self.peek().is_ascii_digit() || self.peek() == b'_' {
                self.bump();
            }
        }
        if matches!(self.peek(), b'e' | b'E') {
            fractional = true;
            self.bump();
            if matches!(self.peek(), b'+' | b'-') {
                self.bump();
            }
            while self.peek().is_ascii_digit() {
                self.bump();
            }
        }
        if self.peek() == b'n' {
            if fractional {
                return self.err("invalid BigInt literal");
            }
            let raw: String = std::str::from_utf8(&self.src[start..self.pos])
                .unwrap_or("0")
                .chars()
                .filter(|c| *c != '_')
                .collect();
            self.bump();
            return raw
                .parse::<i128>()
                .map(Tok::BigInt)
                .or_else(|_| self.err("invalid BigInt literal"));
        }
        let raw: String = std::str::from_utf8(&self.src[start..self.pos])
            .unwrap_or("0")
            .chars()
            .filter(|c| *c != '_')
            .collect();
        raw.parse::<f64>()
            .map(Tok::Num)
            .or_else(|_| self.err("invalid numeric literal"))
    }

    fn read_ident(&mut self) -> Result<String, LexError> {
        let mut out = String::new();
        loop {
            let c = self.peek();
            if c == 0 {
                break;
            }
            if c == b'\\' && self.peek_at(1) == b'u' {
                self.bump();
                self.bump();
                let v = if self.peek() == b'{' {
                    self.bump();
                    let start = self.pos;
                    while self.peek() != b'}' && self.peek() != 0 {
                        self.bump();
                    }
                    let hex = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
                    if self.peek() == b'}' {
                        self.bump();
                    }
                    u32::from_str_radix(hex, 16).unwrap_or(0xfffd)
                } else {
                    self.read_hex(4)?
                };
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                continue;
            }
            if is_ident_continue(c) || c >= 0x80 {
                if c < 0x80 {
                    out.push(c as char);
                    self.bump();
                } else {
                    match self.peek_char() {
                        Some((ch, len)) if is_unicode_ident(ch) => {
                            out.push(ch);
                            self.pos += len;
                        }
                        _ => break,
                    }
                }
                continue;
            }
            break;
        }
        if out.is_empty() {
            return self.err("unexpected character");
        }
        Ok(out)
    }

    fn read_template(&mut self) -> Result<Tok, LexError> {
        self.bump();
        let mut raw = Vec::new();
        let mut exprs = Vec::new();
        let mut current = String::new();
        loop {
            let c = self.peek();
            if c == 0 {
                return self.err("unterminated template literal");
            }
            if c == b'`' {
                self.bump();
                raw.push(current);
                break;
            }
            if c == b'\\' {
                self.bump();
                self.read_escape(&mut current)?;
                continue;
            }
            if c == b'$' && self.peek_at(1) == b'{' {
                self.bump();
                self.bump();
                raw.push(std::mem::take(&mut current));
                let expr = self.read_template_expr()?;
                exprs.push(expr);
                continue;
            }
            if c < 0x80 {
                current.push(c as char);
                self.bump();
            } else {
                match self.peek_char() {
                    Some((ch, len)) => {
                        current.push(ch);
                        self.pos += len;
                    }
                    None => break,
                }
            }
        }
        Ok(Tok::Template { raw, exprs })
    }

    fn read_template_expr(&mut self) -> Result<String, LexError> {
        let start = self.pos;
        let mut depth = 1i32;
        while self.peek() != 0 {
            let c = self.peek();
            if c == b'\'' || c == b'"' {
                let q = c;
                self.bump();
                while self.peek() != 0 && self.peek() != q {
                    if self.peek() == b'\\' {
                        self.bump();
                    }
                    self.bump();
                }
                if self.peek() == q {
                    self.bump();
                }
                continue;
            }
            if c == b'`' {
                // Skip a nested template, including its expressions.
                self.bump();
                let mut nested = 0;
                while self.peek() != 0 {
                    let d = self.peek();
                    if d == b'\\' {
                        self.bump();
                        self.bump();
                        continue;
                    }
                    if d == b'`' && nested == 0 {
                        self.bump();
                        break;
                    }
                    if d == b'$' && self.peek_at(1) == b'{' {
                        nested += 1;
                        self.bump();
                        self.bump();
                        continue;
                    }
                    if d == b'}' && nested > 0 {
                        nested -= 1;
                    }
                    self.bump();
                }
                continue;
            }
            if c == b'{' {
                depth += 1;
                self.bump();
                continue;
            }
            if c == b'}' {
                depth -= 1;
                if depth == 0 {
                    let src = String::from_utf8_lossy(&self.src[start..self.pos]).to_string();
                    self.bump();
                    return Ok(src);
                }
                self.bump();
                continue;
            }
            self.bump();
        }
        self.err("unterminated template expression")
    }

    fn read_regex(&mut self) -> Result<Tok, LexError> {
        self.bump();
        let mut source = String::new();
        let mut in_class = false;
        loop {
            let c = self.peek();
            if c == 0 || c == b'\n' {
                return self.err("unterminated regular expression");
            }
            self.bump();
            if c == b'\\' {
                source.push('\\');
                let n = self.peek();
                if n == 0 {
                    return self.err("unterminated regular expression");
                }
                self.bump();
                source.push(n as char);
                continue;
            }
            if c == b'[' {
                in_class = true;
            } else if c == b']' {
                in_class = false;
            } else if c == b'/' && !in_class {
                break;
            }
            source.push(c as char);
        }
        let mut flags = String::new();
        while self.peek().is_ascii_alphabetic() {
            flags.push(self.bump() as char);
        }
        Ok(Tok::Regex { source, flags })
    }

    fn read_punct(&mut self) -> Result<String, LexError> {
        const PUNCTS: &[&str] = &[
            ">>>=", "...", "===", "!==", "**=", "&&=", "||=", "??=", ">>>", "<<=", ">>=", "=>",
            "==", "!=", "<=", ">=", "&&", "||", "??", "++", "--", "+=", "-=", "*=", "/=", "%=",
            "&=", "|=", "^=", "<<", ">>", "**", "?.",
        ];
        let rest = &self.src[self.pos..];
        for p in PUNCTS {
            if rest.starts_with(p.as_bytes()) {
                // `?.` followed by a digit is actually `?` then `.`
                if *p == "?." && self.peek_at(2).is_ascii_digit() {
                    continue;
                }
                self.pos += p.len();
                return Ok((*p).to_string());
            }
        }
        let c = self.peek();
        if b"+-*/%&|^~!<>=?:;,.()[]{}".contains(&c) {
            self.pos += 1;
            return Ok((c as char).to_string());
        }
        self.err(format!("unexpected character `{}`", c as char))
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}
fn is_ident_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}
fn is_unicode_ident(c: char) -> bool {
    c.is_alphabetic() || c == '\u{200c}' || c == '\u{200d}'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_basics() {
        let toks = tokenize("let x = 1 + 2.5e3; // hi\n/*x*/ 'a\\n'").unwrap();
        assert_eq!(toks[0].kind, Tok::Keyword("let".into()));
        assert_eq!(toks[1].kind, Tok::Ident("x".into()));
        assert_eq!(toks[3].kind, Tok::Num(1.0));
        assert_eq!(toks[5].kind, Tok::Num(2500.0));
        assert_eq!(toks[7].kind, Tok::Str("a\n".into()));
    }

    #[test]
    fn lexes_regex_after_block() {
        let toks = tokenize("if (x) {} /re/.test(y); const o = {a:1} / 2;").unwrap();
        let rx = toks.iter().position(|t| matches!(t.kind, Tok::Regex { .. })).unwrap();
        assert!(matches!(&toks[rx].kind, Tok::Regex { source, .. } if source == "re"));
        assert!(toks.iter().any(|t| matches!(&t.kind, Tok::Punct(p) if p == "/")));
    }

    #[test]
    fn lexes_templates_and_regex() {
        let toks = tokenize("`a${x + 1}b${`n${y}`}`; const r = /ab+c/gi; x / y").unwrap();
        match &toks[0].kind {
            Tok::Template { raw, exprs } => {
                assert_eq!(raw.len(), 3);
                assert_eq!(exprs.len(), 2);
                assert_eq!(exprs[0], "x + 1");
            }
            other => panic!("{other:?}"),
        }
        assert!(toks.iter().any(|t| matches!(&t.kind, Tok::Regex { flags, .. } if flags == "gi")));
        // After an identifier, `/` is division.
        assert!(toks.iter().any(|t| t.kind.is_punct("/")));
    }

    #[test]
    fn lexes_arrows_and_optional_chaining() {
        let toks = tokenize("const f = (a, b) => a?.b ?? b;").unwrap();
        assert!(toks.iter().any(|t| t.kind.is_punct("=>")));
        assert!(toks.iter().any(|t| t.kind.is_punct("?.")));
        assert!(toks.iter().any(|t| t.kind.is_punct("??")));
    }
}
