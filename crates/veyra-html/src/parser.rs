//! HTML tree construction.
//!
//! This is a pragmatic implementation of the HTML5 tree-construction
//! algorithm covering document scaffolding, implied elements, common
//! auto-closing rules, tables (with basic foster parenting) and raw-text
//! elements. It deliberately does not implement the adoption agency algorithm
//! or the full "in foreign content" rules; those are tracked as compatibility
//! gaps rather than silently ignored.

use std::time::Instant;

use veyra_common::Url;
use veyra_dom::{Document, NodeId, NodeType};

use crate::tokenizer::{Token, Tokenizer};

#[derive(Debug, Clone, Default)]
pub struct ParserStats {
    pub tokens: usize,
    pub elements: usize,
    pub text_nodes: usize,
    pub comments: usize,
    pub bytes: usize,
    pub errors: Vec<String>,
    pub parse_ms: u64,
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

const P_CLOSERS: &[&str] = &[
    "address", "article", "aside", "blockquote", "details", "div", "dl", "fieldset", "figcaption",
    "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hgroup", "hr", "main",
    "menu", "nav", "ol", "p", "pre", "section", "table", "ul",
];

const HEAD_ELEMENTS: &[&str] = &["base", "basefont", "bgsound", "link", "meta", "title", "style", "script", "noscript"];

const BLOCK_ELEMENTS: &[&str] = &[
    "address", "article", "aside", "blockquote", "center", "details", "dialog", "dir", "div", "dl",
    "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6",
    "header", "hgroup", "hr", "main", "menu", "nav", "ol", "p", "pre", "section", "summary", "ul",
];

pub fn parse_document(html: &str, base_url: Url) -> (Document, ParserStats) {
    let started = Instant::now();
    let mut doc = Document::new(base_url.clone());
    let mut builder = TreeBuilder::new(&mut doc, base_url, false);
    builder.run(html);
    let mut stats = builder.stats;
    stats.bytes = html.len();
    stats.parse_ms = started.elapsed().as_millis() as u64;
    doc.base_url = doc.effective_base();
    doc.set_title();
    doc.ready_state = "loading".into();
    (doc, stats)
}

/// Parses a fragment using `context` as the insertion context, returning the
/// new top-level nodes (still attached to a temporary container which the
/// caller may discard).
pub fn parse_fragment(doc: &mut Document, context: NodeId, html: &str) -> Vec<NodeId> {
    let base = doc.base_url.clone();
    let tag = doc.tag_name(context).to_string();
    let mut builder = TreeBuilder::new(doc, base, true);
    let container = builder.doc.create_element(if tag.is_empty() { "div" } else { &tag });
    builder.fragment_root = Some(container);
    builder.open.push(container);
    builder.run(html);
    builder.doc.children(container).to_vec()
}

struct TreeBuilder<'a> {
    doc: &'a mut Document,
    base_url: Url,
    open: Vec<NodeId>,
    html: NodeId,
    head: NodeId,
    body: Option<NodeId>,
    saw_doctype: bool,
    quirks: bool,
    fragment: bool,
    fragment_root: Option<NodeId>,
    stats: ParserStats,
    form_open: bool,
    skip_newline: bool,
}

impl<'a> TreeBuilder<'a> {
    fn new(doc: &'a mut Document, base_url: Url, fragment: bool) -> TreeBuilder<'a> {
        let html = doc.create_element("html");
        let head = doc.create_element("head");
        if !fragment {
            doc.append(doc.root, html);
            doc.append(html, head);
        }
        TreeBuilder {
            doc,
            base_url,
            open: if fragment { Vec::new() } else { vec![html] },
            html,
            head,
            body: None,
            saw_doctype: false,
            quirks: false,
            fragment,
            fragment_root: None,
            stats: ParserStats::default(),
            form_open: false,
            skip_newline: false,
        }
    }

    fn run(&mut self, html: &str) {
        let mut tokenizer = Tokenizer::new(html);
        loop {
            let token = tokenizer.next_token();
            if token == Token::Eof {
                break;
            }
            self.stats.tokens += 1;
            self.handle(token);
        }
        self.stats.errors.extend(tokenizer.errors.clone());
        if !self.fragment {
            if !self.saw_doctype {
                self.quirks = true;
            }
            self.doc.quirks_mode = self.quirks;
            self.doc.html = Some(self.html);
            self.doc.head = Some(self.head);
            if self.body.is_none() {
                let body = self.doc.create_element("body");
                self.doc.append(self.html, body);
                self.body = Some(body);
            }
            self.doc.body = self.body;
        }
    }

    fn current(&self) -> NodeId {
        self.open.last().copied().unwrap_or(self.html)
    }

    fn stack_has(&self, tag: &str) -> bool {
        self.open.iter().any(|n| self.doc.tag_name(*n) == tag)
    }

    fn in_scope(&self, tag: &str) -> Option<usize> {
        for (i, n) in self.open.iter().enumerate().rev() {
            let t = self.doc.tag_name(*n);
            if t == tag {
                return Some(i);
            }
            if matches!(t, "table" | "td" | "th" | "caption" | "html") {
                return None;
            }
        }
        None
    }

    fn close_in_scope(&mut self, tag: &str) {
        if let Some(idx) = self.in_scope(tag) {
            self.open.truncate(idx);
            if self.open.is_empty() {
                self.open.push(self.html);
            }
        }
    }

    /// Pops children of `tag`, keeping `tag` itself open.
    fn pop_children_of(&mut self, tag: &str) {
        for i in (0..self.open.len()).rev() {
            if self.doc.tag_name(self.open[i]) == tag {
                self.open.truncate(i + 1);
                return;
            }
        }
    }

    fn pop_until(&mut self, tag: &str) {
        for i in (0..self.open.len()).rev() {
            if self.doc.tag_name(self.open[i]) == tag {
                self.open.truncate(i);
                if self.open.is_empty() {
                    self.open.push(self.html);
                }
                return;
            }
        }
    }

    fn ensure_body(&mut self) -> NodeId {
        if let Some(b) = self.body {
            if !self.open.iter().any(|n| *n == b) && !self.fragment {
                self.open.push(b);
            }
            return b;
        }
        if self.fragment {
            return self.current();
        }
        let body = self.doc.create_element("body");
        self.doc.append(self.html, body);
        self.body = Some(body);
        self.open.clear();
        self.open.push(self.html);
        self.open.push(body);
        body
    }

    fn insert_element(&mut self, name: &str, attrs: Vec<(String, String)>) -> NodeId {
        let node = self.doc.create_element(name);
        for (k, v) in attrs {
            self.doc.set_attr(node, &k, &v);
        }
        self.insert_node(node);
        self.stats.elements += 1;
        node
    }

    fn insert_node(&mut self, node: NodeId) {
        let parent = self.current();
        self.doc.append(parent, node);
    }

    /// Inserts content at the "appropriate place", fostering it out of an
    /// open table when it is not table content.
    fn insert_node_fostered(&mut self, node: NodeId) {
        if let Some(table) = self.in_table_foster() {
            let parent = self.doc.parent(table).unwrap_or(self.html);
            if self.doc.children(parent).contains(&table) {
                self.doc.insert_before(parent, node, Some(table));
            } else {
                self.doc.append(parent, node);
            }
            return;
        }
        self.insert_node(node);
    }

    /// Returns the table element when the insertion point is in a table
    /// structure outside a cell (requires foster parenting).
    fn in_table_foster(&self) -> Option<NodeId> {
        let mut table_idx = None;
        for (i, n) in self.open.iter().enumerate().rev() {
            let t = self.doc.tag_name(*n);
            if matches!(t, "td" | "th" | "caption") {
                return None;
            }
            if t == "table" {
                table_idx = Some(i);
                break;
            }
        }
        let idx = table_idx?;
        // Elements directly inside <table> that are legal do not need fostering.
        self.open.get(idx).copied()
    }

    fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let current = self.current();
        if !self.fragment && self.body.is_none() && !self.in_head_element(current) {
            if text.trim().is_empty() {
                return;
            }
            self.ensure_body();
        }
        let current = self.current();
        let foster = self.in_table_foster().is_some();
        // Merge with the previous text node when possible.
        if !foster {
            if let Some(last) = self.doc.children(current).last().copied() {
                if let Some(NodeType::Text(existing)) = self.doc.node(last).map(|n| n.node_type.clone()) {
                    let merged = format!("{}{}", existing, text);
                    if let Some(n) = self.doc.node_mut(last) {
                        n.node_type = NodeType::Text(merged);
                    }
                    return;
                }
            }
        }
        if self.in_table_foster().is_some() && text.trim().is_empty() {
            return;
        }
        let node = self.doc.create_text(text);
        self.stats.text_nodes += 1;
        self.insert_node_fostered(node);
    }

    fn in_head_element(&self, node: NodeId) -> bool {
        matches!(self.doc.tag_name(node), "title" | "style" | "script" | "noscript" | "template")
    }

    fn handle(&mut self, token: Token) {
        match token {
            Token::Doctype { name, force_quirks } => {
                if self.saw_doctype {
                    self.stats.errors.push("duplicate doctype".into());
                    return;
                }
                self.saw_doctype = true;
                self.quirks = force_quirks || name != "html";
                let node = self.doc.alloc(NodeType::Doctype(name));
                if !self.fragment {
                    self.doc.insert_before(self.doc.root, node, Some(self.html));
                }
            }
            Token::Comment(text) => {
                self.stats.comments += 1;
                let node = self.doc.create_comment(&text);
                if self.fragment || self.body.is_some() {
                    self.insert_node(node);
                } else {
                    self.doc.append(self.head, node);
                }
            }
            Token::Text(text) => self.insert_text(&text),
            Token::StartTag { name, attrs, self_closing, .. } => {
                self.start_tag(&name, attrs, self_closing)
            }
            Token::EndTag { name, .. } => self.end_tag(&name),
            Token::Eof => {}
        }
    }

    fn start_tag(&mut self, name: &str, mut attrs: Vec<(String, String)>, self_closing: bool) {
        match name {
            "html" => {
                for (k, v) in attrs {
                    if self.doc.attr(self.html, &k).is_none() {
                        self.doc.set_attr(self.html, &k, &v);
                    }
                }
                return;
            }
            "head" => {
                if self.body.is_none() && !self.fragment {
                    for (k, v) in attrs {
                        self.doc.set_attr(self.head, &k, &v);
                    }
                }
                return;
            }
            "body" => {
                let body = self.ensure_body();
                for (k, v) in attrs {
                    self.doc.set_attr(body, &k, &v);
                }
                return;
            }
            _ => {}
        }

        // Head-only elements before the body.
        if HEAD_ELEMENTS.contains(&name) && self.body.is_none() && !self.fragment {
            if name == "noscript" {
                // Treated as raw text by the tokenizer; contents are inert.
            }
            let node = self.doc.create_element(name);
            for (k, v) in attrs {
                self.doc.set_attr(node, &k, &v);
            }
            self.doc.append(self.head, node);
            self.stats.elements += 1;
            if !VOID.contains(&name) {
                self.open.push(node);
            }
            return;
        }

        if self.fragment {
            self.start_tag_fragment(name, attrs, self_closing);
            return;
        }

        // Table structure.
        match name {
            "table" => {
                self.close_in_scope("p");
                self.ensure_body();
                let node = self.insert_element(name, attrs);
                self.open.push(node);
                return;
            }
            "caption" | "colgroup" | "thead" | "tbody" | "tfoot" => {
                self.ensure_body();
                if name == "colgroup" {
                    if self.doc.tag_name(self.current()) == "table" {
                        self.close_in_scope("table");
                    }
                } else {
                    self.pop_table_group();
                }
                let node = self.insert_element(name, attrs);
                self.open.push(node);
                return;
            }
            "col" => {
                self.insert_element(name, attrs);
                return;
            }
            "tr" => {
                self.ensure_body();
                if self.stack_has("tr") {
                    self.pop_until("tr");
                }
                self.ensure_row_group();
                let node = self.insert_element(name, attrs);
                self.open.push(node);
                return;
            }
            "td" | "th" => {
                self.ensure_body();
                if !self.stack_has("tr") {
                    self.ensure_row_group();
                    let tr = self.insert_element("tr", Vec::new());
                    self.open.push(tr);
                } else if self.doc.tag_name(self.current()) != "tr" {
                    self.pop_children_of("tr");
                }
                let cur_tag = self.doc.tag_name(self.current()).to_string();
                if matches!(cur_tag.as_str(), "td" | "th") {
                    self.pop_until(&cur_tag);
                }
                let node = self.insert_element(name, attrs);
                self.open.push(node);
                return;
            }
            "select" | "option" | "optgroup" => {
                self.ensure_body();
                if name == "option" {
                    loop {
                        let t = self.doc.tag_name(self.current()).to_string();
                        if matches!(t.as_str(), "option" | "optgroup") {
                            self.pop_until(&t);
                        } else {
                            break;
                        }
                    }
                }
                if name == "optgroup" && self.doc.tag_name(self.current()) == "option" {
                    self.pop_until("option");
                }
                let node = self.insert_element(name, attrs);
                self.open.push(node);
                return;
            }
            _ => {}
        }

        // Inline formatting / common content.
        self.ensure_body();
        match name {
            "li" => {
                if self.stack_has("li") {
                    self.close_in_scope("li");
                }
                self.close_in_scope("p");
            }
            "dt" | "dd" => {
                for t in ["dt", "dd"] {
                    if self.stack_has(t) {
                        self.close_in_scope(t);
                    }
                }
                self.close_in_scope("p");
            }
            "p" => self.close_in_scope("p"),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                for t in ["h1", "h2", "h3", "h4", "h5", "h6"] {
                    if self.stack_has(t) {
                        self.close_in_scope(t);
                    }
                }
                self.close_in_scope("p");
            }
            "a" => {
                if self.stack_has("a") {
                    self.close_in_scope("a");
                }
            }
            "button" => {
                if self.stack_has("button") {
                    self.close_in_scope("button");
                }
            }
            "form" => {
                if self.form_open {
                    return;
                }
                self.form_open = true;
                self.close_in_scope("p");
            }
            "pre" | "listing" | "textarea" => {
                self.skip_newline = true;
                self.close_in_scope("p");
            }
            "script" | "style" | "title" | "template" | "noscript" => {}
            _ if P_CLOSERS.contains(&name) => {
                self.close_in_scope("p");
            }
            _ => {}
        }

        let node = if BLOCK_ELEMENTS.contains(&name) {
            let n = self.doc.create_element(name);
            for (k, v) in attrs {
                self.doc.set_attr(n, &k, &v);
            }
            self.insert_node_fostered(n);
            self.stats.elements += 1;
            n
        } else {
            self.insert_element(name, attrs)
        };
        self.skip_newline = false;
        if !VOID.contains(&name) && !self_closing {
            self.open.push(node);
        }
    }

    fn start_tag_fragment(&mut self, name: &str, attrs: Vec<(String, String)>, self_closing: bool) {
        match name {
            "html" | "head" | "body" => return,
            "tr" => {
                if !self.stack_has("tr") {
                    let node = self.insert_element("tr", Vec::new());
                    self.open.push(node);
                }
                if self.doc.tag_name(self.current()) == "tr" {
                    self.pop_until("tr");
                }
            }
            "td" | "th" => {
                if !self.stack_has("tr") {
                    let tr = self.insert_element("tr", Vec::new());
                    self.open.push(tr);
                }
            }
            _ => {}
        }
        let node = self.insert_element(name, attrs);
        if !VOID.contains(&name) && !self_closing {
            self.open.push(node);
        }
    }

    fn pop_table_group(&mut self) {
        loop {
            let t = self.doc.tag_name(self.current()).to_string();
            if matches!(t.as_str(), "thead" | "tbody" | "tfoot" | "tr" | "td" | "th") {
                self.pop_until(&t);
            } else {
                break;
            }
        }
    }

    fn ensure_row_group(&mut self) {
        let current = self.current();
        let tag = self.doc.tag_name(current);
        if matches!(tag, "thead" | "tbody" | "tfoot" | "table") {
            if tag == "table" {
                let tbody = self.insert_element("tbody", Vec::new());
                self.open.push(tbody);
            }
            return;
        }
        if matches!(tag, "tr") {
            return;
        }
        // Find a table ancestor and open a tbody inside it.
        if self.stack_has("table") {
            while let Some(cur) = self.open.last().copied() {
                if self.doc.tag_name(cur) == "table" {
                    let tbody = self.insert_element("tbody", Vec::new());
                    self.open.push(tbody);
                    return;
                }
                self.open.pop();
            }
        }
    }

    fn end_tag(&mut self, name: &str) {
        match name {
            "html" | "body" => return,
            "br" => {
                self.ensure_body();
                let node = self.insert_element("br", Vec::new());
                let _ = node;
                return;
            }
            "p" => {
                if !self.stack_has("p") {
                    self.ensure_body();
                    let p = self.insert_element("p", Vec::new());
                    let _ = p;
                    return;
                }
                self.close_in_scope("p");
                return;
            }
            "form" => {
                self.form_open = false;
            }
            _ => {}
        }
        if !self.stack_has(name) {
            return;
        }
        // Pop back to (but not including) the html element.
        for i in (0..self.open.len()).rev() {
            if self.doc.tag_name(self.open[i]) == name {
                self.open.truncate(i);
                if self.open.is_empty() {
                    self.open.push(self.html);
                }
                return;
            }
            if self.open[i] == self.html {
                return;
            }
        }
    }
}

pub fn is_block_element(tag: &str) -> bool {
    BLOCK_ELEMENTS.contains(&tag) || tag == "table" || tag == "hr" || tag == "li"
}

#[cfg(test)]
mod tests {
    use super::*;
    use veyra_dom::Document;

    fn parse(html: &str) -> Document {
        parse_document(html, Url::parse("https://example.com/").unwrap()).0
    }

    #[test]
    fn builds_basic_document() {
        let doc = parse("<!doctype html><html><head><title>T</title></head><body><h1>Hi</h1><p>Hello <b>world</b></p></body></html>");
        assert!(!doc.quirks_mode);
        assert_eq!(doc.title, "T");
        let body = doc.body().unwrap();
        assert!(doc.inner_html(body).contains("<h1>Hi</h1>"));
        assert_eq!(doc.visible_text().trim(), "Hi\nHello world");
    }

    #[test]
    fn implies_structure() {
        let doc = parse("<p>one<p>two");
        assert_eq!(doc.element_children(doc.body().unwrap()).len(), 2);
        assert_eq!(doc.query_selector_all(doc.root, "p").len(), 2);
    }

    #[test]
    fn needs_doctype_for_standards_mode() {
        assert!(parse("<p>x").quirks_mode);
        assert!(!parse("<!DOCTYPE html><p>x").quirks_mode);
    }

    #[test]
    fn autocloses_list_items() {
        let doc = parse("<ul><li>a<li>b</ul>");
        let items = doc.query_selector_all(doc.root, "ul > li");
        assert_eq!(items.len(), 2);
        assert_eq!(doc.text_content(items[1]), "b");
    }

    #[test]
    fn builds_tables() {
        let doc = parse("<table><tr><td>a<td>b<tr><td>c</table>");
        assert_eq!(doc.query_selector_all(doc.root, "table tr").len(), 2);
        assert_eq!(doc.query_selector_all(doc.root, "td").len(), 3);
        assert_eq!(doc.query_selector_all(doc.root, "tbody").len(), 1);
    }

    #[test]
    fn parses_fragments() {
        let mut doc = parse("<div id=x></div>");
        let div = doc.get_element_by_id("x").unwrap();
        let nodes = parse_fragment(&mut doc, div, "<p>hi <b>there</b></p><span>!</span>");
        for n in nodes {
            doc.append(div, n);
        }
        assert_eq!(doc.inner_html(div), "<p>hi <b>there</b></p><span>!</span>");
    }

    #[test]
    fn raw_text_and_comments() {
        let doc = parse("<script>var x = 1 < 2;</script><style>p{color:red}</style><div>a<!--c-->b</div>");
        assert_eq!(doc.query_selector_all(doc.root, "script").len(), 1);
        assert_eq!(doc.text_content(doc.query_selector(doc.root, "script").unwrap()), "var x = 1 < 2;");
        assert_eq!(doc.visible_text().trim(), "ab");
    }

    #[test]
    fn attributes_entities_and_void() {
        let doc = parse("<div title=\"a &amp; b\"><img src=x><br></div>");
        let div = doc.query_selector(doc.root, "div").unwrap();
        assert_eq!(doc.attr(div, "title"), Some("a & b"));
        assert_eq!(doc.query_selector_all(doc.root, "img").len(), 1);
        assert_eq!(doc.inner_html(div), "<img src=\"x\"><br>");
    }
}
