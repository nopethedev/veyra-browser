//! The cascade: matching rules to elements, ordering declarations by origin,
//! importance and specificity, inheritance, custom properties and `var()`.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use veyra_common::{Color, Url};
use veyra_dom::selector::SimpleSelector;
use veyra_dom::{matcher, Document, MatchContext, NodeId, Selector, Specificity};
use veyra_common::Size;

use crate::parser::{
    media_matches, parse_declarations, parse_stylesheet, Declaration, Origin, Stylesheet,
};
use crate::style::{ComputedStyle, StyleCtx};
use crate::ua::UA_CSS;
use crate::values::{Display, Length, MediaEnv};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PseudoElement {
    Before,
    After,
    Marker,
    Placeholder,
}

impl PseudoElement {
    pub fn css_name(&self) -> &'static str {
        match self {
            PseudoElement::Before => "before",
            PseudoElement::After => "after",
            PseudoElement::Marker => "marker",
            PseudoElement::Placeholder => "placeholder",
        }
    }
    pub fn parse(name: &str) -> Option<PseudoElement> {
        Some(match name {
            "before" => PseudoElement::Before,
            "after" => PseudoElement::After,
            "marker" => PseudoElement::Marker,
            "placeholder" => PseudoElement::Placeholder,
            _ => return None,
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct StyleStats {
    pub elements: usize,
    pub matched_rules: usize,
    pub candidates: usize,
    pub declarations: usize,
    pub pseudo_elements: usize,
    pub ms: u64,
    pub sheets: usize,
    pub css_bytes: usize,
}

#[derive(Default)]
struct RuleIndex {
    /// (sheet index, rule index, pseudo mask) where sheet 0 is the UA sheet.
    /// Mask bits: 1 normal, 2 `::before`, 4 `::after`, 8 other pseudo-element.
    entries: Vec<(usize, usize, u8)>,
    by_id: HashMap<String, Vec<u32>>,
    by_class: HashMap<String, Vec<u32>>,
    by_tag: HashMap<String, Vec<u32>>,
    universal: Vec<u32>,
}

impl RuleIndex {
    fn add(&mut self, sheet: usize, rule: usize, selectors: &[Selector]) {
        let eid = self.entries.len() as u32;
        let mut mask = 0u8;
        for sel in selectors {
            mask |= match sel.pseudo_element.as_deref() {
                None => 1,
                Some("before") => 2,
                Some("after") => 4,
                Some(_) => 8,
            };
        }
        self.entries.push((sheet, rule, mask));
        for sel in selectors {
            let Some(compound) = sel.compounds.last() else { continue };
            let mut id: Option<&String> = None;
            let mut class: Option<&String> = None;
            let mut tag: Option<&String> = None;
            for simple in &compound.simples {
                match simple {
                    SimpleSelector::Id(v) => id = Some(v),
                    SimpleSelector::Class(v) => class = Some(v),
                    SimpleSelector::Type(v) => tag = Some(v),
                    _ => {}
                }
            }
            if let Some(id) = id {
                self.by_id.entry(id.clone()).or_default().push(eid);
            } else if let Some(class) = class {
                self.by_class.entry(class.clone()).or_default().push(eid);
            } else if let Some(tag) = tag {
                self.by_tag.entry(tag.to_ascii_lowercase()).or_default().push(eid);
            } else {
                self.universal.push(eid);
            }
        }
    }

    fn candidates(&self, doc: &Document, node: NodeId, pseudo: Option<&str>) -> Vec<u32> {
        let want = match pseudo {
            None => 1u8,
            Some("before") => 2,
            Some("after") => 4,
            Some(_) => 8,
        };
        let mut out = self.universal.clone();
        if let Some(id) = doc.attr(node, "id") {
            if let Some(rules) = self.by_id.get(id) {
                out.extend_from_slice(rules);
            }
        }
        if let Some(classes) = doc.attr(node, "class") {
            for class in classes.split_ascii_whitespace() {
                if let Some(rules) = self.by_class.get(class) {
                    out.extend_from_slice(rules);
                }
            }
        }
        let tag = doc.tag_name(node).to_ascii_lowercase();
        if let Some(rules) = self.by_tag.get(&tag) {
            out.extend_from_slice(rules);
        }
        out.sort_unstable();
        out.dedup();
        out.retain(|eid| self.entries[*eid as usize].2 & want != 0);
        out
    }
}

pub struct StyleEngine {
    pub ua: Stylesheet,
    pub author: Vec<Stylesheet>,
    /// Shadow scope for each author sheet (`None` for document sheets).
    pub author_scopes: Vec<Option<NodeId>>,
    rule_index: RuleIndex,
    pub media: MediaEnv,
    pub root_font_size: f32,
    pub hover: Option<NodeId>,
    pub active: Option<NodeId>,
    pub focus: Option<NodeId>,
    pub target: Option<String>,
    pub visited: Vec<NodeId>,
    pub styles: HashMap<NodeId, Rc<ComputedStyle>>,
    pub pseudo: HashMap<(NodeId, PseudoElement), Rc<ComputedStyle>>,
    pub stats: StyleStats,
    pub fallback: ComputedStyle,
}

impl StyleEngine {
    pub fn new() -> StyleEngine {
        let base = Url::parse("about:blank").unwrap();
        let ua = parse_stylesheet(UA_CSS, &base, Origin::UserAgent);
        StyleEngine {
            ua,
            author: Vec::new(),
            author_scopes: Vec::new(),
            rule_index: RuleIndex::default(),
            media: MediaEnv::default(),
            root_font_size: 16.0,
            hover: None,
            active: None,
            focus: None,
            target: None,
            visited: Vec::new(),
            styles: HashMap::new(),
            pseudo: HashMap::new(),
            stats: StyleStats::default(),
            fallback: ComputedStyle::initial(),
        }
    }

    pub fn add_stylesheet(&mut self, css: &str, url: &Url) -> StyleStats {
        let sheet = parse_stylesheet(css, url, Origin::Author);
        self.author.push(sheet);
        self.author_scopes.push(None);
        self.collect_stats()
    }

    /// A stylesheet declared inside a shadow root. Its selectors only match
    /// that tree (plus `:host` and `::slotted(...)`).
    pub fn add_stylesheet_scoped(&mut self, css: &str, url: &Url, scope: NodeId) -> StyleStats {
        let sheet = parse_stylesheet(css, url, Origin::Author);
        self.author.push(sheet);
        self.author_scopes.push(Some(scope));
        self.collect_stats()
    }

    pub fn clear_author_sheets(&mut self) {
        self.author.clear();
        self.author_scopes.clear();
        self.styles.clear();
        self.pseudo.clear();
    }

    pub fn set_viewport(&mut self, w: f32, h: f32) {
        self.media.viewport = Size::new(w.max(1.0), h.max(1.0));
    }

    fn collect_stats(&mut self) -> StyleStats {
        let mut stats = StyleStats {
            sheets: self.author.len() + 1,
            css_bytes: self.ua.bytes,
            ..Default::default()
        };
        for s in &self.author {
            stats.css_bytes += s.bytes;
        }
        stats
    }

    pub fn style(&self, node: NodeId) -> &ComputedStyle {
        self.styles.get(&node).map(|s| s.as_ref()).unwrap_or(&self.fallback)
    }

    /// Returns a cheaply cloneable handle to an element's computed style.
    pub fn style_rc(&self, node: NodeId) -> Rc<ComputedStyle> {
        self.styles.get(&node).cloned().unwrap_or_else(|| Rc::new(self.fallback.clone()))
    }

    pub fn pseudo_style_rc(&self, node: NodeId, which: PseudoElement) -> Option<Rc<ComputedStyle>> {
        self.pseudo.get(&(node, which)).cloned()
    }

    /// `true` when an element has computed styles in the current pass.
    pub fn is_styled(&self, node: NodeId) -> bool {
        self.styles.contains_key(&node)
    }

    pub fn pseudo_style(&self, node: NodeId, which: PseudoElement) -> Option<&ComputedStyle> {
        self.pseudo.get(&(node, which)).map(|s| s.as_ref())
    }

    pub fn compute(&mut self, doc: &Document) {
        let started = Instant::now();
        self.rule_index = self.build_rule_index();
        self.styles.clear();
        self.pseudo.clear();
        self.stats = self.collect_stats();
        self.fallback = ComputedStyle::initial();
        let Some(html) = doc.document_element() else {
            self.stats.ms = started.elapsed().as_millis() as u64;
            return;
        };
        let root_parent = ComputedStyle::initial();
        let (root_style, matched, decls, candidates) = self.compute_style(doc, html, &root_parent, None);
        self.stats.matched_rules += matched;
        self.stats.candidates += candidates;
        self.stats.declarations += decls;
        self.root_font_size = root_style.font_size;
        let root_style = Rc::new(root_style);
        self.styles.insert(html, root_style.clone());
        self.stats.elements += 1;
        self.compute_pseudos(doc, html, &root_style);
        self.style_children(doc, html, &root_style);
        self.style_shadow_trees(doc);
        self.stats.ms = started.elapsed().as_millis() as u64;
        let _ = Display::Block;
        let _ = Length::Auto;
    }

    fn build_rule_index(&self) -> RuleIndex {
        let mut index = RuleIndex::default();
        for (si, sheet) in std::iter::once(&self.ua).chain(self.author.iter()).enumerate() {
            for (ri, rule) in sheet.rules.iter().enumerate() {
                index.add(si, ri, &rule.selectors);
            }
        }
        index
    }

    fn style_shadow_trees(&mut self, doc: &Document) {
        let hosts: Vec<(NodeId, NodeId)> = doc.shadow_roots.iter().map(|(h, r)| (*h, *r)).collect();
        for (host, root) in hosts {
            let Some(host_style) = self.styles.get(&host).cloned() else { continue };
            self.style_children(doc, root, &host_style);
        }
    }

    fn style_children(&mut self, doc: &Document, parent: NodeId, parent_style: &Rc<ComputedStyle>) {
        for child in doc.children(parent).to_vec() {
            if !doc.is_element(child) {
                continue;
            }
            let (style, matched, decls, candidates) = self.compute_style(doc, child, parent_style, None);
            self.stats.matched_rules += matched;
            self.stats.candidates += candidates;
            self.stats.declarations += decls;
            let style = Rc::new(style);
            self.styles.insert(child, style.clone());
            self.stats.elements += 1;
            self.compute_pseudos(doc, child, &style);
            self.style_children(doc, child, &style);
        }
    }

    fn compute_pseudos(&mut self, doc: &Document, node: NodeId, parent_style: &Rc<ComputedStyle>) {
        for pe in [PseudoElement::Before, PseudoElement::After] {
            let (mut style, _, _, _) = self.compute_style(doc, node, parent_style, Some(pe));
            if let Some(raw) = style.content.clone() {
                style.content = Some(resolve_content(&raw, doc, node));
                self.pseudo.insert((node, pe), Rc::new(style));
                self.stats.pseudo_elements += 1;
            }
        }
    }

    fn compute_style(
        &self,
        doc: &Document,
        node: NodeId,
        parent: &ComputedStyle,
        pseudo: Option<PseudoElement>,
    ) -> (ComputedStyle, usize, usize, usize) {
        let mut style = ComputedStyle::inherit_base(parent);
        let mut matched: Vec<(Declaration, Specificity, Origin, u32)> = Vec::new();
        let mut ctx = MatchContext {
            hover: self.hover,
            active: self.active,
            focus: self.focus,
            target: self.target.as_deref(),
            visited: &self.visited,
            scope: None,
        };
        let want_pseudo = pseudo.map(|p| p.css_name());
        let mut matched_rules = 0usize;
        // A single monotonically increasing sequence number spans all sheets.
        // `rule.order` restarts at zero in every stylesheet, so using it alone
        // let an early stylesheet beat a later one that should override it
        // (MediaWiki's site styles hid the logo icon that Vector re-enabled).
        let mut seq: u32 = 0;
        if pseudo.is_none() {
            for d in presentation_hints(doc, node) {
                matched.push((d, Specificity::default(), Origin::Author, seq));
                seq += 1;
            }
        }
        let candidates = self.rule_index.candidates(doc, node, want_pseudo);
        for eid in &candidates {
            let (sheet_idx, rule_idx, _) = self.rule_index.entries[*eid as usize];
            let scope = if sheet_idx == 0 {
                None
            } else {
                self.author_scopes.get(sheet_idx - 1).copied().flatten()
            };
            // Document styles do not pierce a shadow boundary; shadow styles
            // only reach their own tree, their host (`:host`) and slotted
            // light content (`::slotted`).
            let (in_scope, assigned) = if sheet_idx == 0 {
                // User-agent defaults apply inside shadow trees too.
                (true, false)
            } else {
                match scope {
                Some(root) => {
                    let host = doc.host_of(root);
                    let in_tree = doc.in_shadow_tree(node, root);
                    let assigned = doc.assigned_in_shadow(node, root);
                    if host == Some(node) {
                        (true, false)
                    } else if in_tree {
                        (true, false)
                    } else if assigned {
                        (true, true)
                    } else {
                        (false, false)
                    }
                }
                None => (!doc.is_in_shadow(node), false),
                }
            };
            if !in_scope {
                continue;
            }
            let sheet = if sheet_idx == 0 {
                &self.ua
            } else {
                &self.author[sheet_idx - 1]
            };
            let rule = &sheet.rules[rule_idx];
            if !media_matches(&rule.media, &self.media) {
                continue;
            }
            let host_node = scope.and_then(|root| doc.host_of(root));
            let mut best: Option<Specificity> = None;
            for sel in &rule.selectors {
                ctx.scope = scope;
                let pseudo_element = sel.pseudo_element.as_deref();
                let slotted = pseudo_element.and_then(parse_slotted);
                if let Some(inner) = slotted {
                    if !assigned {
                        continue;
                    }
                    if let Some(list) = Selector::parse_list(inner) {
                        if list.iter().any(|s| matcher(doc, node, s, &ctx)) {
                            best = Some(match best {
                                Some(b) => b.max(sel.specificity),
                                None => sel.specificity,
                            });
                        }
                    }
                    continue;
                }
                if pseudo_element != want_pseudo {
                    continue;
                }
                if host_node == Some(node) && scope.is_some() && !selector_has_host(sel) {
                    continue;
                }
                if matcher(doc, node, sel, &ctx) {
                    best = Some(match best {
                        Some(b) => b.max(sel.specificity),
                        None => sel.specificity,
                    });
                }
            }
            if let Some(spec) = best {
                matched_rules += 1;
                for d in &rule.declarations {
                    matched.push((d.clone(), spec, sheet.origin, seq));
                    seq += 1;
                }
            }
        }
        if pseudo.is_none() {
            if let Some(attr) = doc.attr(node, "style") {
                for d in parse_declarations(attr) {
                    matched.push((d, Specificity { a: 1000, b: 0, c: 0 }, Origin::Author, seq));
                    seq += 1;
                }
            }
        }
        let _ = seq;

        matched.sort_by_key(|(d, spec, origin, order)| {
            rank_key(*origin, d.important, *spec, *order)
        });

        // Custom properties first, in cascade order.
        for (d, ..) in &matched {
            if d.name.starts_with("--") {
                // Store the raw token stream: variables resolve against the final
                // cascade, so a later declaration of another custom property must
                // still be visible inside this value.
                style.custom_properties.insert(d.name.clone(), d.value.clone());
            }
        }
        // Font size before everything else so relative units resolve. Only the
        // winning declaration applies, and em/percent resolve against the
        // parent: applying every matching rule in turn would compound sizes
        // (a UA `h1 { font-size: 2em }` plus an author `188%` plus `1.8em`
        // turned 16px into 108px instead of 28.8px).
        if let Some((d, ..)) = matched.iter().rev().find(|(d, ..)| d.name == "font-size") {
            let value = resolve_vars(&d.value, &style.custom_properties, 0);
            let mut ctx = self.ctx(&style, doc);
            ctx.font_size = parent.font_size;
            apply_declaration(&mut style, parent, &d.name, &value, ctx);
        }
        for (d, ..) in &matched {
            if d.name == "font-size" {
                continue;
            }
            let value = resolve_vars(&d.value, &style.custom_properties, 0);
            let ctx = self.ctx(&style, doc);
            apply_declaration(&mut style, parent, &d.name, &value, ctx);
        }

        // HTML `align` attribute and other presentational hints.
        if pseudo.is_none() {
            if let Some(align) = doc.attr(node, "align") {
                if let Some(t) = crate::values::TextAlign::parse(align) {
                    style.text_align = t;
                }
            }
        }
        if style.display == Display::None {
            style.opacity = 1.0;
        }
        (style, matched_rules, matched.len(), candidates.len())
    }

    fn ctx(&self, style: &ComputedStyle, doc: &Document) -> StyleCtx {
        StyleCtx {
            font_size: style.font_size,
            root_font_size: self.root_font_size,
            viewport: self.media.viewport,
            current_color: style.color,
            base_url: doc.base_url.clone(),
        }
    }
}

impl Default for StyleEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn rank_key(origin: Origin, important: bool, spec: Specificity, order: u32) -> (u8, u8, u16, u16, u16, u32) {
    if important {
        let origin_rank = match origin {
            Origin::UserAgent => 3,
            Origin::User => 2,
            Origin::Author => 1,
        };
        (1, origin_rank, spec.a, spec.b, spec.c, order)
    } else {
        let origin_rank = match origin {
            Origin::UserAgent => 0,
            Origin::User => 1,
            Origin::Author => 2,
        };
        (0, origin_rank, spec.a, spec.b, spec.c, order)
    }
}

fn apply_declaration(style: &mut ComputedStyle, parent: &ComputedStyle, name: &str, value: &str, ctx: StyleCtx) {
    let lower = value.trim().to_ascii_lowercase();
    if lower == "inherit" {
        style.copy_property_from(parent, name);
        return;
    }
    if lower == "initial" || lower == "revert" || lower == "unset" {
        if is_inherited(name) || lower == "unset" {
            if is_inherited(name) {
                style.copy_property_from(parent, name);
                return;
            }
        }
        let initial = ComputedStyle::initial();
        style.copy_property_from(&initial, name);
        return;
    }
    style.apply(name, value, &ctx);
}

pub fn is_inherited(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "line-height"
            | "letter-spacing"
            | "word-spacing"
            | "text-align"
            | "text-transform"
            | "text-indent"
            | "white-space"
            | "word-break"
            | "overflow-wrap"
            | "direction"
            | "visibility"
            | "cursor"
            | "list-style-type"
            | "list-style-position"
            | "border-collapse"
            | "border-spacing"
            | "text-decoration"
            | "text-decoration-color"
    )
}

/// Substitutes `var(--name, fallback)` references.
pub fn resolve_vars(value: &str, props: &HashMap<String, String>, depth: usize) -> String {
    if depth > 8 || !value.contains("var(") {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if value[i..].starts_with("var(") {
            let start = i + 4;
            let mut level = 1;
            let mut j = start;
            let mut quote: Option<u8> = None;
            while j < bytes.len() {
                let b = bytes[j];
                match quote {
                    Some(q) => {
                        if b == q {
                            quote = None;
                        }
                    }
                    None => match b {
                        b'"' | b'\'' => quote = Some(b),
                        b'(' => level += 1,
                        b')' => {
                            level -= 1;
                            if level == 0 {
                                break;
                            }
                        }
                        _ => {}
                    },
                }
                j += 1;
            }
            if level != 0 {
                out.push_str(&value[i..]);
                break;
            }
            let inner = &value[start..j];
            let (name, fallback) = match inner.split_once(',') {
                Some((n, f)) => (n.trim(), Some(f.trim())),
                None => (inner.trim(), None),
            };
            let replacement = if let Some(v) = props.get(name) {
                resolve_vars(v, props, depth + 1)
            } else if let Some(f) = fallback {
                resolve_vars(f, props, depth + 1)
            } else {
                String::new()
            };
            out.push_str(&replacement);
            i = j + 1;
        } else {
            let ch = value[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn resolve_content(raw: &str, doc: &Document, node: NodeId) -> String {
    let t = raw.trim();
    if let Some(rest) = t.strip_prefix("attr(") {
        let name = rest.trim_end_matches(')').trim();
        return doc.attr(node, name).unwrap_or("").to_string();
    }
    t.trim_matches('"').to_string()
}

/// HTML presentational attributes, treated as low-priority author declarations.
fn presentation_hints(doc: &Document, node: NodeId) -> Vec<Declaration> {
    let mut out = Vec::new();
    let tag = doc.tag_name(node);
    let hint = |name: &str, value: String| Declaration {
        name: name.into(),
        value,
        important: false,
    };
    if let Some(bg) = doc.attr(node, "bgcolor") {
        if Color::parse(bg).is_some() {
            out.push(hint("background-color", bg.to_string()));
        }
    }
    if let Some(color) = doc.attr(node, "color") {
        if tag == "font" && Color::parse(color).is_some() {
            out.push(hint("color", color.to_string()));
        }
    }
    if tag == "font" {
        if let Some(face) = doc.attr(node, "face") {
            out.push(hint("font-family", face.to_string()));
        }
        if let Some(size) = doc.attr(node, "size") {
            let px = match size.trim() {
                "1" => 10.0,
                "2" => 13.0,
                "3" => 16.0,
                "4" => 18.0,
                "5" => 24.0,
                "6" => 32.0,
                "7" => 48.0,
                other => other.parse::<f32>().unwrap_or(16.0) * 1.5,
            };
            out.push(hint("font-size", format!("{px}px")));
        }
    }
    if tag == "body" {
        if let Some(text) = doc.attr(node, "text") {
            if Color::parse(text).is_some() {
                out.push(hint("color", text.to_string()));
            }
        }
        if let Some(bg) = doc.attr(node, "background") {
            let url = doc.base_url.resolve(bg);
            if url.is_http() || url.scheme == "data" {
                out.push(hint("background-image", format!("url({})", url)));
            }
        }
    }
    if let Some(dir) = doc.attr(node, "dir") {
        out.push(hint("direction", dir.to_ascii_lowercase()));
    }
    if tag == "hr" {
        if doc.attr(node, "noshade").is_none() {
            if let Some(size) = doc.attr(node, "size") {
                out.push(hint("border-width", format!("{}px", size)));
            }
        }
    }
    if tag == "table" || tag == "td" || tag == "th" {
        if let Some(border) = doc.attr(node, "border") {
            if border != "0" {
                let w = border.parse::<f32>().unwrap_or(1.0).max(1.0);
                out.push(hint("border-width", format!("{w}px")));
                out.push(hint("border-style", "outset".into()));
            }
        }
    }
    if let Some(width) = doc.attr(node, "width") {
        if matches!(tag, "table" | "td" | "th" | "col" | "colgroup") {
            out.push(hint("width", parse_html_dimension(width)));
        }
    }
    if let Some(height) = doc.attr(node, "height") {
        if matches!(tag, "table" | "td" | "th" | "tr") {
            out.push(hint("height", parse_html_dimension(height)));
        }
    }
    out
}

fn parse_html_dimension(value: &str) -> String {
    let v = value.trim();
    if v.ends_with('%') {
        v.to_string()
    } else if let Ok(n) = v.parse::<f32>() {
        format!("{n}px")
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veyra_common::Url;
    use veyra_html::parse_document;

    fn styled(html: &str, css: &str) -> (Document, StyleEngine) {
        let base = Url::parse("https://example.com/").unwrap();
        let (doc, _) = parse_document(html, base);
        let mut engine = StyleEngine::new();
        engine.media.viewport = Size::new(1000.0, 800.0);
        engine.add_stylesheet(css, &doc.base_url);
        engine.compute(&doc);
        (doc, engine)
    }

    #[test]
    fn ua_and_author_cascade() {
        let (doc, engine) = styled(
            "<div id=a><p class=x>hi</p></div>",
            "p { color: red } .x { font-size: 20px } #a p { color: blue !important }",
        );
        let p = doc.query_selector(doc.root, "p").unwrap();
        let style = engine.style(p);
        assert_eq!(style.color, Color::rgb(0, 0, 255));
        assert_eq!(style.font_size, 20.0);
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn root_custom_properties_inherit() {
        let (doc, engine) = styled(
            "<html><body><div class=RNNXgb>x</div></body></html>",
            ":root{--xhUGwc:#fff} body{background:var(--xhUGwc)} .RNNXgb{background:var(--xhUGwc)}",
        );
        let div = doc.query_selector(doc.root, ".RNNXgb").unwrap();
        assert_eq!(engine.style(div).background_color, Color::rgb(255, 255, 255));
    }

    #[test]
    fn inline_style_wins() {
        let (doc, engine) = styled("<p style=\"color: green\">x</p>", "p { color: red }");
        let p = doc.query_selector(doc.root, "p").unwrap();
        assert_eq!(engine.style(p).color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn inheritance_and_initial() {
        let (doc, engine) = styled(
            "<div id=d><span id=s>x</span></div>",
            "#d { color: #123456; font-size: 30px; border: 2px solid red } #s { color: inherit }",
        );
        let s = doc.get_element_by_id("s").unwrap();
        let style = engine.style(s);
        assert_eq!(style.color, Color::rgb(0x12, 0x34, 0x56));
        assert_eq!(style.font_size, 30.0);
        assert_eq!(style.border_width.top, Length::ZERO);
        assert_eq!(style.display, Display::Inline);
    }

    #[test]
    fn custom_properties() {
        let (doc, engine) = styled(
            "<div id=d><p id=p>x</p></div>",
            "#d { --brand: #ff0000 } #p { color: var(--brand) }",
        );
        let p = doc.get_element_by_id("p").unwrap();
        assert_eq!(engine.style(p).color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn media_queries_apply() {
        let (doc, engine) = styled(
            "<p id=p>x</p>",
            "@media (max-width: 500px) { #p { color: red } } @media (min-width: 900px) { #p { color: lime } }",
        );
        let p = doc.get_element_by_id("p").unwrap();
        assert_eq!(engine.style(p).color, Color::rgb(0, 255, 0));
    }

    #[test]
    fn later_stylesheet_overrides_earlier() {
        let base = Url::parse("https://example.com/").unwrap();
        let (doc, _) = parse_document("<span id=s class=icon>x</span>", base.clone());
        let mut engine = StyleEngine::new();
        engine.media.viewport = Size::new(1000.0, 800.0);
        engine.add_stylesheet("@media all and (min-width: 600px) { .icon { display: block } }", &base);
        engine.add_stylesheet(".icon { display: none }", &base);
        engine.compute(&doc);
        let s = doc.get_element_by_id("s").unwrap();
        assert_eq!(engine.style(s).display, Display::None);
    }

    #[test]
    fn pseudo_elements() {
        let (doc, engine) = styled("<div id=d class=x></div>", ".x::before { content: 'hello'; color: red }");
        let d = doc.get_element_by_id("d").unwrap();
        let before = engine.pseudo_style(d, PseudoElement::Before).unwrap();
        assert_eq!(before.content.as_deref(), Some("hello"));
        assert_eq!(before.color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn hidden_attribute() {
        let (doc, engine) = styled("<p id=p hidden>x</p>", "");
        let p = doc.get_element_by_id("p").unwrap();
        assert_eq!(engine.style(p).display, Display::None);
    }

    #[test]
    fn vars_with_fallback() {
        let mut props = HashMap::new();
        props.insert("--a".to_string(), "var(--b, 3px)".to_string());
        assert_eq!(resolve_vars("var(--a, 1px)", &props, 0), "3px");
        assert_eq!(resolve_vars("var(--nope, 1px)", &props, 0), "1px");
    }
}

fn parse_slotted(pseudo: &str) -> Option<&str> {
    let inner = pseudo.strip_prefix("slotted(")?.strip_suffix(')')?;
    if inner.trim().is_empty() {
        None
    } else {
        Some(inner.trim())
    }
}

fn selector_has_host(sel: &Selector) -> bool {
    sel.compounds.iter().any(|compound| {
        compound.simples.iter().any(|simple| {
            matches!(simple, SimpleSelector::PseudoClass(p) if matches!(**p, veyra_dom::selector::PseudoClass::Host(_)))
        })
    })
}
