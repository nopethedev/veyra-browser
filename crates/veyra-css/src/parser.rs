//! CSS stylesheet parsing: rules, declarations, at-rules and media queries.

use std::time::Instant;

use veyra_common::Url;
use veyra_dom::Selector;

use crate::values::MediaEnv;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    UserAgent,
    User,
    Author,
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub name: String,
    pub value: String,
    pub important: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    All,
    Screen,
    Print,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaCondition {
    MinWidth(f32),
    MaxWidth(f32),
    MinHeight(f32),
    MaxHeight(f32),
    Landscape,
    Portrait,
    PrefersDark,
    PrefersLight,
    ReducedMotion,
    NoPreferenceReducedMotion,
    Hover,
    AnyHover,
    FinePointer,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct MediaQuery {
    pub negated: bool,
    pub kind: MediaKind,
    pub conditions: Vec<MediaCondition>,
}

impl MediaQuery {
    pub fn matches(&self, env: &MediaEnv) -> bool {
        let kind_ok = match self.kind {
            MediaKind::All => true,
            MediaKind::Screen => !env.print,
            MediaKind::Print => env.print,
        };
        let cond_ok = self.conditions.iter().all(|c| match c {
            MediaCondition::MinWidth(v) => env.viewport.w >= *v,
            MediaCondition::MaxWidth(v) => env.viewport.w <= *v,
            MediaCondition::MinHeight(v) => env.viewport.h >= *v,
            MediaCondition::MaxHeight(v) => env.viewport.h <= *v,
            MediaCondition::Landscape => env.viewport.w > env.viewport.h,
            MediaCondition::Portrait => env.viewport.h >= env.viewport.w,
            MediaCondition::PrefersDark => env.prefers_dark,
            MediaCondition::PrefersLight => !env.prefers_dark,
            MediaCondition::ReducedMotion => env.reduced_motion,
            MediaCondition::NoPreferenceReducedMotion => !env.reduced_motion,
            MediaCondition::Hover | MediaCondition::AnyHover | MediaCondition::FinePointer => true,
            MediaCondition::Unknown => false,
        });
        let result = kind_ok && cond_ok;
        if self.negated {
            !result
        } else {
            result
        }
    }
}

pub fn media_matches(queries: &[MediaQuery], env: &MediaEnv) -> bool {
    queries.is_empty() || queries.iter().any(|q| q.matches(env))
}

#[derive(Debug, Clone)]
pub struct StyleRule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    pub order: u32,
    pub media: Vec<MediaQuery>,
    pub selector_text: String,
}

#[derive(Debug, Clone)]
pub struct ImportRule {
    pub url: String,
    pub media: Vec<MediaQuery>,
}

#[derive(Debug, Clone)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
    pub imports: Vec<ImportRule>,
    pub origin: Origin,
    pub base_url: Url,
    pub errors: Vec<String>,
    pub bytes: usize,
    pub parse_ms: u64,
}

impl Stylesheet {
    pub fn empty(origin: Origin) -> Stylesheet {
        Stylesheet {
            rules: Vec::new(),
            imports: Vec::new(),
            origin,
            base_url: Url::parse("about:blank").unwrap(),
            errors: Vec::new(),
            bytes: 0,
            parse_ms: 0,
        }
    }
}

pub fn parse_stylesheet(text: &str, base_url: &Url, origin: Origin) -> Stylesheet {
    let started = Instant::now();
    let mut sheet = Stylesheet {
        rules: Vec::new(),
        imports: Vec::new(),
        origin,
        base_url: base_url.clone(),
        errors: Vec::new(),
        bytes: text.len(),
        parse_ms: 0,
    };
    let mut scanner = Scanner::new(text);
    scanner.parse_rules(&[], 0, &mut sheet);
    sheet.rules.sort_by_key(|r| r.order);
    for (i, rule) in sheet.rules.iter_mut().enumerate() {
        rule.order = i as u32;
    }
    sheet.parse_ms = started.elapsed().as_millis() as u64;
    sheet
}

/// Parses a `style="..."` attribute.
pub fn parse_declarations(text: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == b'\\' {
                    i += 1;
                } else if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'(' => depth += 1,
                b')' => depth -= 1,
                b';' if depth <= 0 => {
                    if let Some(d) = parse_one(&text[start..i]) {
                        out.push(d);
                    }
                    start = i + 1;
                }
                _ => {}
            },
        }
        i += 1;
    }
    if let Some(d) = parse_one(&text[start..]) {
        out.push(d);
    }
    out
}

fn parse_one(text: &str) -> Option<Declaration> {
    let text = text.trim();
    if text.is_empty() || text.starts_with("/*") {
        return None;
    }
    let colon = find_top_level_colon(text)?;
    let raw_name = text[..colon].trim();
    // Custom property names are case-sensitive; normal properties are not.
    let name = if raw_name.starts_with("--") {
        raw_name.to_string()
    } else {
        raw_name.to_ascii_lowercase()
    };
    if name.is_empty() || !(name.starts_with("--") || name.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)) {
        return None;
    }
    let mut value = text[colon + 1..].trim().to_string();
    let mut important = false;
    let lower = value.to_ascii_lowercase();
    if let Some(idx) = lower.rfind("!important") {
        if lower[idx + 10..].trim().is_empty() {
            important = true;
            value = value[..idx].trim().to_string();
        }
    }
    if value.is_empty() {
        return None;
    }
    Some(Declaration { name, value: normalize_whitespace(&value), important })
}

fn find_top_level_colon(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    for (i, b) in bytes.iter().enumerate() {
        match quote {
            Some(q) => {
                if *b == q {
                    quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' => quote = Some(*b),
                b'(' => depth += 1,
                b')' => depth -= 1,
                b':' if depth == 0 => return Some(i),
                _ => {}
            },
        }
    }
    None
}

fn normalize_whitespace(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut quote: Option<char> = None;
    let mut last_space = false;
    for c in value.chars() {
        match quote {
            Some(q) => {
                out.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                    out.push(c);
                    last_space = false;
                } else if c.is_whitespace() {
                    if !last_space {
                        out.push(' ');
                        last_space = true;
                    }
                } else {
                    out.push(c);
                    last_space = false;
                }
            }
        }
    }
    out.trim().to_string()
}

struct Scanner<'a> {
    s: &'a str,
    b: &'a [u8],
    pos: usize,
    order: u32,
}

impl<'a> Scanner<'a> {
    fn new(s: &'a str) -> Scanner<'a> {
        Scanner { s, b: s.as_bytes(), pos: 0, order: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn skip_ws_comments(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'\x0c')) {
                self.pos += 1;
            }
            if self.b[self.pos..].starts_with(b"/*") {
                if let Some(end) = self.s[self.pos + 2..].find("*/") {
                    self.pos += 2 + end + 2;
                    continue;
                }
                self.pos = self.b.len();
                return;
            }
            break;
        }
    }

    fn read_until(&mut self, stop: &[u8]) -> String {
        let start = self.pos;
        let mut depth = 0i32;
        let mut quote: Option<u8> = None;
        while self.pos < self.b.len() {
            let b = self.b[self.pos];
            match quote {
                Some(q) => {
                    if b == b'\\' {
                        self.pos += 2;
                        continue;
                    }
                    if b == q {
                        quote = None;
                    }
                }
                None => {
                    // CSS escapes (Tailwind emits class names such as `.content-\[\'\\{\'\]`)
                    // must not affect brace depth.
                    if b == b'\\' {
                        self.pos += 2;
                        continue;
                    }
                    if b == b'/' && self.b.get(self.pos + 1) == Some(&b'*') {
                        if let Some(end) = self.s[self.pos + 2..].find("*/") {
                            self.pos += 2 + end + 2;
                            continue;
                        }
                        self.pos = self.b.len();
                        break;
                    }
                    if b == b'"' || b == b'\'' {
                        quote = Some(b);
                    } else if b == b'(' || b == b'[' {
                        depth += 1;
                    } else if b == b')' || b == b']' {
                        depth -= 1;
                    } else if depth <= 0 && stop.contains(&b) {
                        break;
                    }
                }
            }
            self.pos += 1;
        }
        self.s[start..self.pos].trim().to_string()
    }

    fn read_block(&mut self) -> String {
        // Consumes an opening '{' and returns the inner text.
        if self.peek() == Some(b'{') {
            self.pos += 1;
        }
        let start = self.pos;
        let mut depth = 1;
        let mut quote: Option<u8> = None;
        while self.pos < self.b.len() {
            let b = self.b[self.pos];
            match quote {
                Some(q) => {
                    if b == b'\\' {
                        self.pos += 2;
                        continue;
                    }
                    if b == q {
                        quote = None;
                    }
                }
                None => {
                    // CSS escapes (Tailwind emits class names such as `.content-\[\'\\{\'\]`)
                    // must not affect brace depth.
                    if b == b'\\' {
                        self.pos += 2;
                        continue;
                    }
                    if b == b'/' && self.b.get(self.pos + 1) == Some(&b'*') {
                        if let Some(end) = self.s[self.pos + 2..].find("*/") {
                            self.pos += 2 + end + 2;
                            continue;
                        }
                        self.pos = self.b.len();
                        break;
                    }
                    if b == b'"' || b == b'\'' {
                        quote = Some(b);
                    } else if b == b'{' {
                        depth += 1;
                    } else if b == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            let inner = self.s[start..self.pos].to_string();
                            self.pos += 1;
                            return inner;
                        }
                    }
                }
            }
            self.pos += 1;
        }
        self.s[start..].to_string()
    }

    fn parse_rules(&mut self, media: &[MediaQuery], depth: usize, sheet: &mut Stylesheet) {
        if depth > 8 {
            return;
        }
        loop {
            self.skip_ws_comments();
            if self.pos >= self.b.len() {
                return;
            }
            if self.peek() == Some(b'}') {
                self.pos += 1;
                return;
            }
            if self.peek() == Some(b'@') {
                self.at_rule(media, depth, sheet);
                continue;
            }
            let prelude = self.read_until(b"{");
            if self.peek() != Some(b'{') {
                return;
            }
            let block = self.read_block();
            let selector_text = prelude.clone();
            let selectors = match Selector::parse_list(&prelude) {
                Some(list) => list,
                None => {
                    sheet.errors.push(format!("unsupported selector: {}", truncate(&prelude, 80)));
                    continue;
                }
            };
            let declarations = parse_declarations(&block);
            if declarations.is_empty() {
                continue;
            }
            sheet.rules.push(StyleRule {
                selectors,
                declarations,
                order: self.order,
                media: media.to_vec(),
                selector_text,
            });
            self.order += 1;
        }
    }

    fn at_rule(&mut self, media: &[MediaQuery], depth: usize, sheet: &mut Stylesheet) {
        self.pos += 1; // '@'
        let name_start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let name = self.s[name_start..self.pos].to_ascii_lowercase();
        let prelude = self.read_until(b"{;");
        match self.peek() {
            Some(b';') => {
                self.pos += 1;
                if name == "import" {
                    let (url, media_text) = split_import(&prelude);
                    if !url.is_empty() {
                        let mq = parse_media_list(&media_text);
                        sheet.imports.push(ImportRule { url, media: mq });
                    }
                }
                return;
            }
            Some(b'{') => {
                let block = self.read_block();
                match name.as_str() {
                    "media" => {
                        let mut combined = media.to_vec();
                        combined.extend(parse_media_list(&prelude));
                        let mut inner = Scanner::new(&block);
                        inner.order = self.order;
                        inner.parse_rules(&combined, depth + 1, sheet);
                        self.order = inner.order;
                    }
                    "supports" => {
                        if supports_condition(&prelude) {
                            let mut inner = Scanner::new(&block);
                            inner.order = self.order;
                            inner.parse_rules(media, depth + 1, sheet);
                            self.order = inner.order;
                        }
                    }
                    "layer" => {
                        let mut inner = Scanner::new(&block);
                        inner.order = self.order;
                        inner.parse_rules(media, depth + 1, sheet);
                        self.order = inner.order;
                    }
                    // @font-face, @keyframes, @page, @property, @container and
                    // other at-rules are recorded as unsupported.
                    "font-face" | "keyframes" | "-webkit-keyframes" | "page" | "property" | "container" | "counter-style" => {
                        sheet.errors.push(format!("unsupported at-rule: @{}", name));
                    }
                    _ => {}
                }
                return;
            }
            _ => return,
        }
    }
}

/// Conservative `@supports` evaluation. We advertise only features the style
/// engine can actually paint, so modern fallback stacks keep their simpler
/// declarations in Veyra.
fn supports_condition(text: &str) -> bool {
    let t = strip_outer_parens(text.trim());
    if let Some(rest) = t.strip_prefix("not ") {
        return !supports_condition(rest);
    }
    if let Some(parts) = split_support_op(t, " or ") {
        return parts.iter().any(|p| supports_condition(p));
    }
    if let Some(parts) = split_support_op(t, " and ") {
        return parts.iter().all(|p| supports_condition(p));
    }
    let Some((prop, value)) = split_support_decl(t) else {
        return false;
    };
    let prop = prop.trim().to_ascii_lowercase();
    let value = value.trim().to_ascii_lowercase();
    if ["margin-trim", "-webkit-hyphens", "-moz-orient", "-webkit-box-orient"]
        .contains(&prop.as_str())
    {
        return false;
    }
    for unsupported in [
        "lab(", "lch(", "oklab(", "oklch(", "color-mix(", "light-dark(", "color(",
        "rgb(from ", "hsl(from ", "anchor(", "calc-size(", "linear-gradient(in ",
        "radial-gradient(in ", "conic-gradient(in ", "interpolate-size",
    ] {
        if value.contains(unsupported) {
            return false;
        }
    }
    true
}

fn strip_outer_parens(mut s: &str) -> &str {
    loop {
        if !(s.starts_with('(') && s.ends_with(')')) {
            return s;
        }
        let mut depth = 0i32;
        let mut wraps = true;
        for (i, ch) in s.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && i + 1 != s.len() {
                        wraps = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if wraps {
            s = s[1..s.len() - 1].trim();
        } else {
            return s;
        }
    }
}

fn split_support_op<'a>(text: &'a str, op: &str) -> Option<Vec<&'a str>> {
    let mut depth = 0i32;
    let mut parts = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && text[i..].starts_with(op) {
            parts.push(text[start..i].trim());
            i += op.len();
            start = i;
            continue;
        }
        i += 1;
    }
    if parts.is_empty() {
        None
    } else {
        parts.push(text[start..].trim());
        Some(parts)
    }
}

fn split_support_decl(text: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ':' if depth == 0 => return Some((&text[..i], &text[i + 1..])),
            _ => {}
        }
    }
    None
}

fn split_import(prelude: &str) -> (String, String) {
    let trimmed = prelude.trim();
    let (url, rest) = if trimmed.to_ascii_lowercase().starts_with("url(") {
        let open = trimmed.find('(').unwrap_or(0);
        let close = trimmed[open..].find(')').map(|i| i + open).unwrap_or(trimmed.len());
        let inner = trimmed[open + 1..close].trim().trim_matches(|c| c == '"' || c == '\'');
        (inner.to_string(), trimmed.get(close + 1..).unwrap_or("").trim().to_string())
    } else if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        let quote = trimmed.chars().next().unwrap();
        let body = &trimmed[1..];
        match body.find(quote) {
            Some(end) => (body[..end].to_string(), body[end + 1..].trim().to_string()),
            None => (body.to_string(), String::new()),
        }
    } else {
        let end = trimmed.find([' ', '\t']).unwrap_or(trimmed.len());
        (trimmed[..end].to_string(), trimmed[end..].trim().to_string())
    };
    (url, rest)
}

pub fn parse_media_list(text: &str) -> Vec<MediaQuery> {
    let mut out = Vec::new();
    for part in split_top_level_commas(text) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let lower = part.to_ascii_lowercase();
        let mut negated = false;
        let mut rest = lower.as_str();
        if let Some(r) = rest.strip_prefix("not ") {
            negated = true;
            rest = r.trim();
        }
        if let Some(r) = rest.strip_prefix("only ") {
            rest = r.trim();
        }
        let mut kind = MediaKind::All;
        if rest.starts_with("screen") {
            kind = MediaKind::Screen;
        } else if rest.starts_with("print") {
            kind = MediaKind::Print;
        } else if rest.starts_with("all") {
            kind = MediaKind::All;
        } else if !rest.starts_with('(') && !rest.is_empty() {
            out.push(MediaQuery { negated, kind: MediaKind::All, conditions: vec![MediaCondition::Unknown] });
            continue;
        }
        let mut conditions = Vec::new();
        let mut idx = 0;
        let bytes = rest.as_bytes();
        while idx < bytes.len() {
            if bytes[idx] == b'(' {
                let close = match rest[idx..].find(')') {
                    Some(c) => idx + c,
                    None => break,
                };
                let cond = &rest[idx + 1..close];
                conditions.push(parse_media_condition(cond));
                idx = close + 1;
            } else {
                idx += 1;
            }
        }
        out.push(MediaQuery { negated, kind, conditions });
    }
    out
}

fn parse_media_condition(cond: &str) -> MediaCondition {
    let c = cond.trim();
    let (name, value) = match c.split_once(':') {
        Some((n, v)) => (n.trim(), v.trim()),
        None => (c, ""),
    };
    let px = |v: &str| -> f32 {
        v.trim()
            .trim_end_matches("px")
            .trim()
            .parse::<f32>()
            .unwrap_or(0.0)
    };
    match name {
        "min-width" => MediaCondition::MinWidth(px(value)),
        "max-width" => MediaCondition::MaxWidth(px(value)),
        "min-height" => MediaCondition::MinHeight(px(value)),
        "max-height" => MediaCondition::MaxHeight(px(value)),
        "orientation" => {
            if value == "landscape" {
                MediaCondition::Landscape
            } else {
                MediaCondition::Portrait
            }
        }
        "prefers-color-scheme" => {
            if value == "dark" {
                MediaCondition::PrefersDark
            } else if value == "light" {
                MediaCondition::PrefersLight
            } else {
                MediaCondition::Unknown
            }
        }
        "prefers-reduced-motion" => {
            if value == "reduce" {
                MediaCondition::ReducedMotion
            } else {
                MediaCondition::NoPreferenceReducedMotion
            }
        }
        "hover" => MediaCondition::Hover,
        "any-hover" => MediaCondition::AnyHover,
        "pointer" => MediaCondition::FinePointer,
        _ => MediaCondition::Unknown,
    }
}

fn split_top_level_commas(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in text.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => {
                out.push(current.clone());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

/// Resolves `url(...)` values, which may be quoted.
pub fn parse_url_value(value: &str, base: &Url) -> Option<Url> {
    let v = value.trim();
    let inner = if let Some(rest) = v.strip_prefix("url(").or_else(|| v.strip_prefix("URL(")) {
        rest.trim_end_matches(')').trim().trim_matches(|c| c == '"' || c == '\'')
    } else {
        return None;
    };
    if inner.is_empty() {
        return None;
    }
    let url = base.resolve(inner);
    if url.scheme == "data" || url.is_http() {
        Some(url)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/css/site.css").unwrap()
    }

    #[test]
    fn parses_rules_and_media() {
        let css = r#"
            /* comment */
            body { margin: 8px; color: #333 }
            @media screen and (min-width: 600px) {
              .wide { display: flex; gap: 10px }
            }
            a:hover, a:focus { text-decoration: underline !important }
        "#;
        let sheet = parse_stylesheet(css, &base(), Origin::Author);
        assert_eq!(sheet.rules.len(), 3);
        assert_eq!(sheet.rules[0].selectors.len(), 1);
        assert_eq!(sheet.rules[1].media.len(), 1);
        assert!(sheet.rules[2].declarations[0].important);
        let env = MediaEnv { viewport: veyra_common::Size::new(800.0, 600.0), ..Default::default() };
        assert!(media_matches(&sheet.rules[1].media, &env));
        let env_small = MediaEnv { viewport: veyra_common::Size::new(400.0, 600.0), ..Default::default() };
        assert!(!media_matches(&sheet.rules[1].media, &env_small));
    }

    #[test]
    fn parses_imports() {
        let css = "@import url(\"reset.css\"); @import 'base.css' screen; body{color:red}";
        let sheet = parse_stylesheet(css, &base(), Origin::Author);
        assert_eq!(sheet.imports.len(), 2);
        assert_eq!(sheet.imports[0].url, "reset.css");
        assert_eq!(sheet.imports[1].url, "base.css");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn declarations_handle_functions_and_important() {
        let decls = parse_declarations("background: rgb(1, 2, 3); font-family: \"A B\", sans-serif; color: red !important");
        assert_eq!(decls.len(), 3);
        assert_eq!(decls[0].value, "rgb(1, 2, 3)");
        assert_eq!(decls[1].value, "\"A B\", sans-serif");
        assert!(decls[2].important);
        assert_eq!(decls[2].value, "red");
    }

    #[test]
    fn skips_unsupported_selectors() {
        let sheet = parse_stylesheet("div::-moz-thing { color: red } p { color: blue }", &base(), Origin::Author);
        assert_eq!(sheet.rules.len(), 2);
    }

    #[test]
    fn supports_keeps_only_supported_features() {
        let css = r#"
            a { color: red }
            @supports (display: grid) { a { display: grid } }
            @supports (color: lab(50% 0 0)) { a { color: lab(50% 0 0) } }
            @supports not (color: lab(50% 0 0)) { a { color: blue } }
        "#;
        let sheet = parse_stylesheet(css, &base(), Origin::Author);
        assert_eq!(sheet.rules.len(), 3);
        let values: Vec<&str> = sheet.rules.iter().flat_map(|r| r.declarations.iter().map(|d| d.value.as_str())).collect();
        assert!(values.contains(&"grid"));
        assert!(values.contains(&"blue"));
        assert!(!values.iter().any(|v| v.starts_with("lab(")));
    }

    #[test]
    fn escaped_braces_do_not_swallow_following_rules() {
        let css = r#".content-\['\{'\] { content: "{" } .after { color: green }"#;
        let sheet = parse_stylesheet(css, &base(), Origin::Author);
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations[0].name, "color");
        assert_eq!(sheet.rules[0].declarations[0].value, "green");
    }

    #[test]
    fn urls() {
        let u = parse_url_value("url(../img/a.png)", &base()).unwrap();
        assert_eq!(u.to_string(), "https://example.com/img/a.png");
    }
}
