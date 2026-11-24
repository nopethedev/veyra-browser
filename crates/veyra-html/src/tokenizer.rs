//! HTML tokenizer: a state machine over bytes producing tags, text, comments
//! and doctypes, with raw-text handling for script/style and HTML character
//! references.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    StartTag {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
        line: u32,
    },
    EndTag {
        name: String,
        line: u32,
    },
    Text(String),
    Comment(String),
    Doctype {
        name: String,
        force_quirks: bool,
    },
    Eof,
}

impl Token {
    pub fn name(&self) -> Option<&str> {
        match self {
            Token::StartTag { name, .. } | Token::EndTag { name, .. } => Some(name),
            _ => None,
        }
    }
}

pub struct Tokenizer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    pub line: u32,
    raw: Option<String>,
    pub errors: Vec<String>,
}

const RAW_TEXT_TAGS: &[&str] = &[
    "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "noscript",
    "plaintext",
];

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a str) -> Tokenizer<'a> {
        Tokenizer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            line: 1,
            raw: None,
            errors: Vec::new(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'\x0c')) {
            self.bump();
        }
    }

    fn starts_with_ci(&self, needle: &str) -> bool {
        self.src
            .as_bytes()
            .get(self.pos..)
            .and_then(|s| s.get(..needle.len()))
            .map(|s| s.eq_ignore_ascii_case(needle.as_bytes()))
            .unwrap_or(false)
    }

    pub fn next_token(&mut self) -> Token {
        if let Some(tag) = self.raw.clone() {
            return self.raw_text(&tag);
        }
        if self.pos >= self.bytes.len() {
            return Token::Eof;
        }
        if self.peek() != Some(b'<') {
            return Token::Text(self.decode_data());
        }
        // Comment / doctype / bogus comment.
        if self.starts_with_ci("<!--") {
            return self.comment();
        }
        if self.starts_with_ci("<!doctype") {
            return self.doctype();
        }
        if self.starts_with_ci("<![cdata[") {
            // CDATA is not recognised in HTML content: treat as a comment.
            self.pos += 9;
            let rest = self.take_until("]]>");
            return Token::Comment(rest);
        }
        if self.starts_with_ci("<!") || self.starts_with_ci("<?") {
            return self.bogus_comment();
        }
        if self.starts_with_ci("</") {
            if self.bytes.get(self.pos + 2).map(|b| b.is_ascii_alphabetic()).unwrap_or(false) {
                return self.end_tag();
            }
            // `</>` is ignored; `</ x>` is a bogus comment.
            self.pos += 2;
            let rest = self.take_until(">");
            return Token::Comment(rest);
        }
        if self.bytes.get(self.pos + 1).map(|b| b.is_ascii_alphabetic()).unwrap_or(false) {
            return self.start_tag();
        }
        // A literal '<'.
        self.bump();
        Token::Text("<".into())
    }

    fn take_until(&mut self, needle: &str) -> String {
        match self.src[self.pos..].find(needle) {
            Some(idx) => {
                let s = self.src[self.pos..self.pos + idx].to_string();
                self.advance(idx + needle.len());
                s
            }
            None => {
                let s = self.src[self.pos..].to_string();
                self.advance(self.src.len() - self.pos);
                s
            }
        }
    }

    fn advance(&mut self, n: usize) {
        let end = (self.pos + n).min(self.bytes.len());
        for b in &self.bytes[self.pos..end] {
            if *b == b'\n' {
                self.line += 1;
            }
        }
        self.pos = end;
    }

    fn comment(&mut self) -> Token {
        self.pos += 4;
        // `<!-->` and `<!--->` are empty comments.
        if self.peek() == Some(b'>') {
            self.pos += 1;
            return Token::Comment(String::new());
        }
        if self.starts_with_ci("->") {
            self.pos += 2;
            return Token::Comment(String::new());
        }
        let mut text = self.take_until("-->");
        if self.pos >= self.bytes.len() {
            // Unterminated comment: strip a trailing '-' emitted by `--!>`.
            text = text.trim_end_matches('!').to_string();
        }
        Token::Comment(text)
    }

    fn bogus_comment(&mut self) -> Token {
        if self.peek() == Some(b'<') {
            self.pos += 1;
        }
        if self.peek() == Some(b'!') || self.peek() == Some(b'?') {
            self.pos += 1;
        }
        let text = self.take_until(">");
        Token::Comment(text)
    }

    fn doctype(&mut self) -> Token {
        self.pos += 9;
        let body = self.take_until(">");
        let mut parts = body.split_whitespace();
        let name = parts.next().unwrap_or("").to_ascii_lowercase();
        let rest = parts.collect::<Vec<_>>().join(" ").to_ascii_lowercase();
        let force_quirks = name != "html"
            || rest.contains("html 4.01")
            || rest.contains("xhtml 1.0 transitional")
            || (!rest.is_empty() && !rest.starts_with("public") && !rest.starts_with("system"));
        Token::Doctype { name, force_quirks }
    }

    fn raw_text(&mut self, tag: &str) -> Token {
        if tag == "plaintext" {
            let text = self.src[self.pos..].to_string();
            self.advance(self.src.len() - self.pos);
            self.raw = None;
            return if text.is_empty() { Token::Eof } else { Token::Text(text) };
        }
        let closing = format!("</{tag}");
        let hay = &self.src[self.pos..];
        let lower = hay.to_ascii_lowercase();
        match lower.find(&closing) {
            Some(idx) => {
                let boundary = lower.as_bytes().get(idx + closing.len()).copied();
                let valid = match boundary {
                    None => true,
                    Some(b) => matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/' | b'>'),
                };
                if !valid {
                    // Not a real end tag; emit the whole thing as text.
                    let text = hay.to_string();
                    self.advance(hay.len());
                    self.raw = None;
                    return Token::Text(text);
                }
                let text = hay[..idx].to_string();
                self.advance(idx);
                self.raw = None;
                if text.is_empty() {
                    self.next_token()
                } else {
                    Token::Text(text)
                }
            }
            None => {
                let text = hay.to_string();
                self.advance(hay.len());
                self.raw = None;
                if text.is_empty() {
                    Token::Eof
                } else {
                    Token::Text(text)
                }
            }
        }
    }

    /// Reads a run of character data up to the next `<`, decoding references.
    fn decode_data(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b == b'<' {
                break;
            }
            self.bump();
        }
        decode_entities(&self.src[start..self.pos])
    }

    fn start_tag(&mut self) -> Token {
        let line = self.line;
        self.pos += 1;
        let name = self.tag_name();
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut self_closing = false;
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                Some(b'/') => {
                    self.pos += 1;
                    if self.peek() == Some(b'>') {
                        self.pos += 1;
                        self_closing = true;
                        break;
                    }
                    continue;
                }
                _ => {}
            }
            let attr_name = self.attr_name();
            if attr_name.is_empty() {
                // Defensive: skip an unexpected byte.
                self.bump();
                continue;
            }
            self.skip_ws();
            let mut value = String::new();
            if self.peek() == Some(b'=') {
                self.pos += 1;
                self.skip_ws();
                match self.peek() {
                    Some(q @ (b'"' | b'\'')) => {
                        self.pos += 1;
                        let start = self.pos;
                        while let Some(b) = self.peek() {
                            if b == q {
                                break;
                            }
                            self.bump();
                        }
                        value = decode_entities(&self.src[start..self.pos]);
                        if self.peek() == Some(q) {
                            self.pos += 1;
                        } else {
                            self.errors.push(format!("line {}: unterminated attribute value", self.line));
                        }
                    }
                    _ => {
                        let start = self.pos;
                        while let Some(b) = self.peek() {
                            if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'>') {
                                break;
                            }
                            self.bump();
                        }
                        value = decode_entities(&self.src[start..self.pos]);
                    }
                }
            }
            if !attrs.iter().any(|(k, _)| *k == attr_name) {
                attrs.push((attr_name, value));
            }
        }
        if RAW_TEXT_TAGS.contains(&name.as_str()) && !self_closing {
            self.raw = Some(name.clone());
        }
        Token::StartTag { name, attrs, self_closing, line }
    }

    fn end_tag(&mut self) -> Token {
        let line = self.line;
        self.pos += 2;
        let name = self.tag_name();
        // Ignore attributes on end tags.
        while let Some(b) = self.peek() {
            if b == b'>' {
                self.pos += 1;
                break;
            }
            self.bump();
        }
        Token::EndTag { name, line }
    }

    fn tag_name(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() || matches!(b, b'>' | b'/') {
                break;
            }
            self.bump();
        }
        self.src[start..self.pos].to_ascii_lowercase()
    }

    fn attr_name(&mut self) -> String {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() || matches!(b, b'=' | b'>' | b'/') {
                break;
            }
            self.bump();
        }
        self.src[start..self.pos].to_ascii_lowercase()
    }
}

/// Decodes HTML character references in text and attribute values.
pub fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'&' {
                i += 1;
            }
            out.push_str(&input[start..i]);
            continue;
        }
        // Numeric reference.
        if let Some(rest) = input[i..].strip_prefix("&#") {
            let (radix, skip) = if rest.starts_with(['x', 'X']) { (16, 1) } else { (10, 0) };
            let digits: String = rest[skip..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            if !digits.is_empty() {
                if let Ok(mut code) = u32::from_str_radix(&digits, radix) {
                    if (0xD800..0xE000).contains(&code) || code == 0 {
                        code = 0xFFFD;
                    }
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                        i += 2 + skip + digits.len();
                        if bytes.get(i) == Some(&b';') {
                            i += 1;
                        }
                        continue;
                    }
                }
            }
        }
        // Named reference (longest match first).
        let semicolon = input[i + 1..].find(';').map(|p| i + 1 + p);
        let mut matched = false;
        if let Some(sc) = semicolon {
            if sc > i + 1 && sc - (i + 1) <= 32 {
                if let Some(ch) = named_entity(&input[i + 1..sc]) {
                    out.push(ch);
                    i = sc + 1;
                    matched = true;
                }
            }
        }
        if !matched {
            // Legacy entities without a trailing semicolon.
            let tail: String = input[i + 1..].chars().take(6).collect();
            let char_count = tail.chars().count();
            for len in (2..=char_count).rev() {
                if let Some(ch) = named_entity(&tail.chars().take(len).collect::<String>()) {
                    out.push(ch);
                    i += 1 + len;
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            out.push('&');
            i += 1;
        }
    }
    out
}

fn named_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "shy" => '\u{ad}',
        "zwnj" => '\u{200c}',
        "zwj" => '\u{200d}',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "deg" => '°',
        "plusmn" => '±',
        "micro" => 'µ',
        "para" => '¶',
        "sect" => '§',
        "middot" => '·',
        "bull" => '•',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "times" => '×',
        "divide" => '÷',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "harr" => '↔',
        "hearts" => '♥',
        "euro" => '€',
        "pound" => '£',
        "yen" => '¥',
        "cent" => '¢',
        "curren" => '¤',
        "brvbar" => '¦',
        "uml" => '¨',
        "ordf" => 'ª',
        "ordm" => 'º',
        "not" => '¬',
        "macr" => '¯',
        "acute" => '´',
        "cedil" => '¸',
        "sup1" => '¹',
        "sup2" => '²',
        "sup3" => '³',
        "frac14" => '¼',
        "frac12" => '½',
        "frac34" => '¾',
        "iquest" => '¿',
        "agrave" => 'à',
        "aacute" => 'á',
        "acirc" => 'â',
        "atilde" => 'ã',
        "auml" => 'ä',
        "aring" => 'å',
        "aelig" => 'æ',
        "ccedil" => 'ç',
        "egrave" => 'è',
        "eacute" => 'é',
        "ecirc" => 'ê',
        "euml" => 'ë',
        "igrave" => 'ì',
        "iacute" => 'í',
        "icirc" => 'î',
        "iuml" => 'ï',
        "ntilde" => 'ñ',
        "ograve" => 'ò',
        "oacute" => 'ó',
        "ocirc" => 'ô',
        "otilde" => 'õ',
        "ouml" => 'ö',
        "oslash" => 'ø',
        "ugrave" => 'ù',
        "uacute" => 'ú',
        "ucirc" => 'û',
        "uuml" => 'ü',
        "yacute" => 'ý',
        "yuml" => 'ÿ',
        "szlig" => 'ß',
        "Alpha" => 'Α',
        "Beta" => 'Β',
        "Gamma" => 'Γ',
        "Delta" => 'Δ',
        "Omega" => 'Ω',
        "alpha" => 'α',
        "beta" => 'β',
        "gamma" => 'γ',
        "delta" => 'δ',
        "pi" => 'π',
        "sigma" => 'σ',
        "omega" => 'ω',
        "check" => '✓',
        // Common named entities used by web pages.
        "star" => '☆',
        "starf" => '★',
        "spades" => '♠',
        "clubs" => '♣',
        "diams" => '♦',
        "loz" => '◊',
        "frasl" => '⁄',
        "sbquo" => '‚',
        "bdquo" => '„',
        "dagger" => '†',
        "Dagger" => '‡',
        "permil" => '‰',
        "prime" => '′',
        "Prime" => '″',
        "lsaquo" => '‹',
        "rsaquo" => '›',
        "oline" => '‾',
        "lrm" => '\u{200e}',
        "rlm" => '\u{200f}',
        "ne" => '≠',
        "le" => '≤',
        "ge" => '≥',
        "asymp" => '≈',
        "infin" => '∞',
        "sum" => '∑',
        "prod" => '∏',
        "radic" => '√',
        "int" => '∫',
        "minus" => '−',
        "lowast" => '∗',
        "cap" => '∩',
        "cup" => '∪',
        "and" => '∧',
        "or" => '∨',
        "there4" => '∴',
        "sim" => '∼',
        "cong" => '≅',
        "sub" => '⊂',
        "sup" => '⊃',
        "nsub" => '⊄',
        "sube" => '⊆',
        "supe" => '⊇',
        "oplus" => '⊕',
        "otimes" => '⊗',
        "perp" => '⊥',
        "sdot" => '⋅',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> Vec<Token> {
        let mut t = Tokenizer::new(src);
        let mut out = Vec::new();
        loop {
            let tok = t.next_token();
            if tok == Token::Eof {
                break;
            }
            out.push(tok);
        }
        out
    }

    #[test]
    fn simple_document() {
        let ts = tokens("<p class=\"a\" id=b>Hi &amp; bye</p>");
        assert_eq!(ts.len(), 3);
        match &ts[0] {
            Token::StartTag { name, attrs, .. } => {
                assert_eq!(name, "p");
                assert_eq!(attrs.len(), 2);
                assert_eq!(attrs[0].1, "a");
                assert_eq!(attrs[1].1, "b");
            }
            _ => panic!("expected start tag"),
        }
        assert_eq!(ts[1], Token::Text("Hi & bye".into()));
    }

    #[test]
    fn raw_text_script() {
        let ts = tokens("<script>if (a < b) x = \"</p>\";</script><p>x</p>");
        assert!(matches!(&ts[0], Token::StartTag { name, .. } if name == "script"));
        assert_eq!(ts[1], Token::Text("if (a < b) x = \"</p>\";".into()));
        assert!(matches!(&ts[2], Token::EndTag { name, .. } if name == "script"));
        assert!(matches!(&ts[3], Token::StartTag { name, .. } if name == "p"));
    }

    #[test]
    fn comments_and_doctype() {
        let ts = tokens("<!doctype html><!-- hello --><div></div>");
        assert!(matches!(&ts[0], Token::Doctype { name, force_quirks: false } if name == "html"));
        assert_eq!(ts[1], Token::Comment(" hello ".into()));
        assert!(matches!(&ts[2], Token::StartTag { name, .. } if name == "div"));
    }

    #[test]
    fn numeric_entities() {
        assert_eq!(decode_entities("&#65;&#x42;&amp;"), "AB&");
        assert_eq!(decode_entities("&unknown;"), "&unknown;");
        assert_eq!(decode_entities("&amp"), "&");
        assert_eq!(decode_entities("&hellip;"), "…");
    }

    #[test]
    fn self_closing() {
        let ts = tokens("<img src=x /><br/>");
        match &ts[0] {
            Token::StartTag { self_closing, .. } => assert!(*self_closing),
            _ => panic!(),
        }
    }
}
