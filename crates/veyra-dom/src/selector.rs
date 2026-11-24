//! CSS selector parsing and matching.

use std::cmp::Ordering;

use crate::{Document, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Specificity {
    pub a: u16,
    pub b: u16,
    pub c: u16,
}

impl Specificity {
    pub fn add(self, other: Specificity) -> Specificity {
        Specificity {
            a: self.a + other.a,
            b: self.b + other.b,
            c: self.c + other.c,
        }
    }
    pub fn max(self, other: Specificity) -> Specificity {
        if self > other {
            self
        } else {
            other
        }
    }
    pub fn tuple(self) -> u32 {
        (self.a as u32) << 20 | (self.b as u32) << 10 | self.c as u32
    }
}

impl PartialOrd for Specificity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Specificity {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.a, self.b, self.c).cmp(&(other.a, other.b, other.c))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone)]
pub enum PseudoClass {
    FirstChild,
    LastChild,
    OnlyChild,
    NthChild(i32, i32),
    NthLastChild(i32, i32),
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthOfType(i32, i32),
    NthLastOfType(i32, i32),
    Root,
    Host(Option<Vec<Selector>>),
    Empty,
    Not(Vec<Selector>),
    Is(Vec<Selector>),
    Where(Vec<Selector>),
    Link,
    Visited,
    Checked,
    Disabled,
    Enabled,
    Required,
    Optional,
    Focus,
    Hover,
    Active,
    Target,
    Scope,
    Lang(String),
    PlaceholderShown,
    Any(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum SimpleSelector {
    Universal,
    Type(String),
    Id(String),
    Class(String),
    Attribute { name: String, op: AttrOp, value: String, case_insensitive: bool },
    PseudoClass(Box<PseudoClass>),
}

#[derive(Debug, Clone, Default)]
pub struct CompoundSelector {
    pub simples: Vec<SimpleSelector>,
    pub specificity: Specificity,
}

#[derive(Debug, Clone)]
pub struct Selector {
    pub compounds: Vec<CompoundSelector>,
    pub combinators: Vec<Combinator>,
    pub pseudo_element: Option<String>,
    pub specificity: Specificity,
}

impl Selector {
    pub fn parse(text: &str) -> Option<Selector> {
        let mut p = SelectorParser { s: text.as_bytes(), pos: 0 };
        let sel = p.complex()?;
        p.skip_ws();
        if p.pos != p.s.len() {
            return None;
        }
        Some(sel)
    }

    pub fn parse_list(text: &str) -> Option<Vec<Selector>> {
        let mut out = Vec::new();
        let mut depth = 0i32;
        let mut start = 0;
        let bytes = text.as_bytes();
        let mut in_string: Option<u8> = None;
        for (i, b) in bytes.iter().enumerate() {
            match in_string {
                Some(q) => {
                    if *b == q {
                        in_string = None;
                    }
                }
                None => match b {
                    b'"' | b'\'' => in_string = Some(*b),
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b',' if depth == 0 => {
                        let part = &text[start..i];
                        if !part.trim().is_empty() {
                            out.push(Selector::parse(part.trim())?);
                        }
                        start = i + 1;
                    }
                    _ => {}
                },
            }
        }
        let part = &text[start..];
        if !part.trim().is_empty() {
            out.push(Selector::parse(part.trim())?);
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    pub fn has_pseudo_element(&self) -> Option<&str> {
        self.pseudo_element.as_deref()
    }

    /// True when the selector's subject requires dynamic state that a static
    /// style pass cannot evaluate. Used by the cascade to style `:hover` etc.
    pub fn needs_state(&self) -> bool {
        self.compounds.iter().any(|c| {
            c.simples.iter().any(|s| {
                matches!(
                    s,
                    SimpleSelector::PseudoClass(p)
                        if matches!(**p, PseudoClass::Hover | PseudoClass::Active | PseudoClass::Focus | PseudoClass::Visited)
                )
            })
        })
    }
}

struct SelectorParser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> SelectorParser<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }
    fn skip_ws(&mut self) -> bool {
        let start = self.pos;
        while matches!(self.peek(), Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'\x0c')) {
            self.pos += 1;
        }
        self.pos > start
    }

    fn complex(&mut self) -> Option<Selector> {
        self.skip_ws();
        let mut compounds = vec![self.compound()?];
        let mut combinators = Vec::new();
        let mut pseudo_element = None;
        let mut specificity = compounds[0].specificity;
        loop {
            let had_ws = self.skip_ws();
            match self.peek() {
                None | Some(b',') | Some(b')') => break,
                Some(b'>') => {
                    self.pos += 1;
                    self.skip_ws();
                    let c = self.compound()?;
                    specificity = specificity.add(c.specificity);
                    compounds.push(c);
                    combinators.push(Combinator::Child);
                }
                Some(b'+') => {
                    self.pos += 1;
                    self.skip_ws();
                    let c = self.compound()?;
                    specificity = specificity.add(c.specificity);
                    compounds.push(c);
                    combinators.push(Combinator::NextSibling);
                }
                Some(b'~') => {
                    self.pos += 1;
                    self.skip_ws();
                    let c = self.compound()?;
                    specificity = specificity.add(c.specificity);
                    compounds.push(c);
                    combinators.push(Combinator::SubsequentSibling);
                }
                Some(_) if had_ws => {
                    let c = self.compound()?;
                    specificity = specificity.add(c.specificity);
                    compounds.push(c);
                    combinators.push(Combinator::Descendant);
                }
                Some(_) => return None,
            }
        }
        // A pseudo-element belongs to the last compound.
        if let Some(CompoundSelector { simples, .. }) = compounds.last_mut() {
            if let Some(SimpleSelector::PseudoClass(p)) = simples.last() {
                if let PseudoClass::Any(names) = &**p {
                    if let Some(name) = names.first() {
                        pseudo_element = Some(name.clone());
                        simples.pop();
                    }
                }
            }
        }
        // Recompute specificity without the pseudo-element marker (it was
        // already counted as a type selector by the parser).
        Some(Selector { compounds, combinators, pseudo_element, specificity })
    }

    fn compound(&mut self) -> Option<CompoundSelector> {
        let mut simples = Vec::new();
        let mut specificity = Specificity::default();
        while let Some(b) = self.peek() {
            match b {
                b'*' => {
                    self.pos += 1;
                    simples.push(SimpleSelector::Universal);
                }
                b'#' => {
                    self.pos += 1;
                    let name = self.ident()?;
                    specificity.a += 1;
                    simples.push(SimpleSelector::Id(name));
                }
                b'.' => {
                    self.pos += 1;
                    let name = self.ident()?;
                    specificity.b += 1;
                    simples.push(SimpleSelector::Class(name));
                }
                b'[' => {
                    self.pos += 1;
                    let sel = self.attribute()?;
                    specificity.b += 1;
                    simples.push(sel);
                }
                b':' => {
                    self.pos += 1;
                    let double = self.peek() == Some(b':');
                    if double {
                        self.pos += 1;
                        let name = self.ident()?.to_ascii_lowercase();
                        specificity.c += 1;
                        if name == "slotted" && self.peek() == Some(b'(') {
                            let inner = self.argument()?;
                            simples.push(SimpleSelector::PseudoClass(Box::new(PseudoClass::Any(vec![format!(
                                "slotted({})",
                                inner.trim()
                            )]))));
                        } else {
                            simples.push(SimpleSelector::PseudoClass(Box::new(PseudoClass::Any(vec![name]))));
                        }
                    } else {
                        let name = self.ident()?.to_ascii_lowercase();
                        let (pseudo, spec) = self.pseudo(&name)?;
                        specificity = specificity.add(spec);
                        simples.push(SimpleSelector::PseudoClass(Box::new(pseudo)));
                    }
                }
                b if is_ident_start(b) || b == b'\\' => {
                    let name = self.ident()?;
                    if self.peek() == Some(b'|') {
                        // Ignore namespace prefixes; match on the local name.
                        self.pos += 1;
                        let local = self.ident()?;
                        specificity.c += 1;
                        simples.push(SimpleSelector::Type(local.to_ascii_lowercase()));
                    } else {
                        specificity.c += 1;
                        simples.push(SimpleSelector::Type(name.to_ascii_lowercase()));
                    }
                }
                _ => break,
            }
        }
        if simples.is_empty() {
            return None;
        }
        Some(CompoundSelector { simples, specificity })
    }

    fn attribute(&mut self) -> Option<SimpleSelector> {
        self.skip_ws();
        let mut name = String::new();
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':') {
                name.push(b as char);
                self.pos += 1;
            } else if b == b'|' && self.s.get(self.pos + 1) != Some(&b'=') {
                self.pos += 1;
            } else {
                break;
            }
        }
        if name.is_empty() {
            return None;
        }
        self.skip_ws();
        let op = match (self.peek(), self.s.get(self.pos + 1)) {
            (Some(b']'), _) => {
                self.pos += 1;
                AttrOp::Exists
            }
            (Some(b'='), _) => {
                self.pos += 1;
                AttrOp::Equals
            }
            (Some(b'~'), Some(b'=')) => {
                self.pos += 2;
                AttrOp::Includes
            }
            (Some(b'|'), Some(b'=')) => {
                self.pos += 2;
                AttrOp::DashMatch
            }
            (Some(b'^'), Some(b'=')) => {
                self.pos += 2;
                AttrOp::Prefix
            }
            (Some(b'$'), Some(b'=')) => {
                self.pos += 2;
                AttrOp::Suffix
            }
            (Some(b'*'), Some(b'=')) => {
                self.pos += 2;
                AttrOp::Substring
            }
            _ => return None,
        };
        if op == AttrOp::Exists {
            return Some(SimpleSelector::Attribute {
                name: name.to_ascii_lowercase(),
                op,
                value: String::new(),
                case_insensitive: false,
            });
        }
        self.skip_ws();
        let value = match self.peek() {
            Some(q @ (b'"' | b'\'')) => {
                self.pos += 1;
                let mut v = String::new();
                while let Some(b) = self.peek() {
                    if b == q {
                        self.pos += 1;
                        break;
                    }
                    if b == b'\\' {
                        self.pos += 1;
                        if let Some(esc) = self.peek() {
                            v.push(esc as char);
                            self.pos += 1;
                            continue;
                        }
                    }
                    v.push(b as char);
                    self.pos += 1;
                }
                v
            }
            _ => self.ident()?,
        };
        self.skip_ws();
        let mut ci = false;
        if let Some(b) = self.peek() {
            if b == b'i' || b == b'I' {
                ci = true;
                self.pos += 1;
                self.skip_ws();
            }
        }
        if self.peek() != Some(b']') {
            return None;
        }
        self.pos += 1;
        Some(SimpleSelector::Attribute {
            name: name.to_ascii_lowercase(),
            op,
            value,
            case_insensitive: ci,
        })
    }

    fn pseudo(&mut self, name: &str) -> Option<(PseudoClass, Specificity)> {
        let mut spec = Specificity { a: 0, b: 1, c: 0 };
        let arg = if self.peek() == Some(b'(') {
            Some(self.argument()?)
        } else {
            None
        };
        let pseudo = match name {
            "first-child" => PseudoClass::FirstChild,
            "last-child" => PseudoClass::LastChild,
            "only-child" => PseudoClass::OnlyChild,
            "first-of-type" => PseudoClass::FirstOfType,
            "last-of-type" => PseudoClass::LastOfType,
            "only-of-type" => PseudoClass::OnlyOfType,
            "nth-child" => {
                let (a, b) = parse_anb(arg.as_deref()?)?;
                PseudoClass::NthChild(a, b)
            }
            "nth-last-child" => {
                let (a, b) = parse_anb(arg.as_deref()?)?;
                PseudoClass::NthLastChild(a, b)
            }
            "nth-of-type" => {
                let (a, b) = parse_anb(arg.as_deref()?)?;
                PseudoClass::NthOfType(a, b)
            }
            "nth-last-of-type" => {
                let (a, b) = parse_anb(arg.as_deref()?)?;
                PseudoClass::NthLastOfType(a, b)
            }
            "root" => PseudoClass::Root,
            "host" => {
                let list = match arg.as_deref() {
                    Some(a) => Some(Selector::parse_list(a)?),
                    None => None,
                };
                PseudoClass::Host(list)
            }
            "empty" => PseudoClass::Empty,
            "not" => {
                let list = Selector::parse_list(arg.as_deref()?)?;
                spec = list.iter().map(|s| s.specificity).max().unwrap_or_default();
                PseudoClass::Not(list)
            }
            "is" | "matches" | "-webkit-any" | "any" => {
                let list = Selector::parse_list(arg.as_deref()?)?;
                spec = list.iter().map(|s| s.specificity).max().unwrap_or_default();
                PseudoClass::Is(list)
            }
            "where" => {
                let list = Selector::parse_list(arg.as_deref()?)?;
                spec = Specificity::default();
                PseudoClass::Where(list)
            }
            "link" | "any-link" => PseudoClass::Link,
            "visited" => PseudoClass::Visited,
            "checked" => PseudoClass::Checked,
            "disabled" => PseudoClass::Disabled,
            "enabled" => PseudoClass::Enabled,
            "required" => PseudoClass::Required,
            "optional" => PseudoClass::Optional,
            "focus" | "focus-visible" | "focus-within" => PseudoClass::Focus,
            "hover" => PseudoClass::Hover,
            "active" => PseudoClass::Active,
            "target" => PseudoClass::Target,
            "scope" => PseudoClass::Scope,
            "placeholder-shown" => PseudoClass::PlaceholderShown,
            "lang" => PseudoClass::Lang(arg?.trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase()),
            // Vendor / unsupported pseudo-classes never match.
            _ => PseudoClass::Any(vec![format!("__never_{name}")]),
        };
        Some((pseudo, spec))
    }

    fn argument(&mut self) -> Option<String> {
        // Assumes current byte is '('.
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1;
        let mut in_string: Option<u8> = None;
        while let Some(b) = self.peek() {
            match in_string {
                Some(q) => {
                    if b == q {
                        in_string = None;
                    }
                }
                None => match b {
                    b'"' | b'\'' => in_string = Some(b),
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            let out = String::from_utf8_lossy(&self.s[start..self.pos]).into_owned();
                            self.pos += 1;
                            return Some(out.trim().to_string());
                        }
                    }
                    _ => {}
                },
            }
            self.pos += 1;
        }
        None
    }

    fn ident(&mut self) -> Option<String> {
        let mut out = String::new();
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b >= 0x80 {
                out.push(b as char);
                self.pos += 1;
            } else if b == b'\\' {
                self.pos += 1;
                if let Some(esc) = self.peek() {
                    out.push(esc as char);
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

pub fn parse_anb(text: &str) -> Option<(i32, i32)> {
    let s: String = text.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    if s == "odd" {
        return Some((2, 1));
    }
    if s == "even" {
        return Some((2, 0));
    }
    if let Ok(b) = s.parse::<i32>() {
        return Some((0, b));
    }
    let (a_part, b_part) = s.split_once('n')?;
    let a = match a_part {
        "" | "+" => 1,
        "-" => -1,
        other => other.parse::<i32>().ok()?,
    };
    let b = if b_part.is_empty() {
        0
    } else {
        let cleaned = b_part.strip_prefix('+').unwrap_or(b_part);
        cleaned.parse::<i32>().ok()?
    };
    Some((a, b))
}

/// Dynamic state supplied by the browser when matching.
#[derive(Debug, Default, Clone)]
pub struct MatchContext<'a> {
    pub hover: Option<NodeId>,
    pub active: Option<NodeId>,
    pub focus: Option<NodeId>,
    pub scope: Option<NodeId>,
    pub target: Option<&'a str>,
    pub visited: &'a [NodeId],
}

pub fn matches_selector_list(doc: &Document, node: NodeId, list: &[Selector], ctx: &MatchContext) -> bool {
    list.iter().any(|s| matcher(doc, node, s, ctx))
}

pub fn matcher(doc: &Document, node: NodeId, sel: &Selector, ctx: &MatchContext) -> bool {
    if doc.node(node).is_none() || !doc.node(node).map(|n| n.is_element()).unwrap_or(false) {
        return false;
    }
    let idx = sel.compounds.len() - 1;
    match_from(doc, node, sel, idx, ctx)
}

fn match_from(doc: &Document, node: NodeId, sel: &Selector, idx: usize, ctx: &MatchContext) -> bool {
    if !matches_compound(doc, node, &sel.compounds[idx], ctx) {
        return false;
    }
    if idx == 0 {
        return true;
    }
    match sel.combinators[idx - 1] {
        Combinator::Child => match doc.parent(node) {
            Some(p) => match_from(doc, p, sel, idx - 1, ctx),
            None => false,
        },
        Combinator::Descendant => {
            let mut cur = doc.parent(node);
            while let Some(p) = cur {
                if match_from(doc, p, sel, idx - 1, ctx) {
                    return true;
                }
                cur = doc.parent(p);
            }
            false
        }
        Combinator::NextSibling => match doc.prev_element_sibling(node) {
            Some(s) => match_from(doc, s, sel, idx - 1, ctx),
            None => false,
        },
        Combinator::SubsequentSibling => {
            let mut cur = doc.prev_element_sibling(node);
            while let Some(s) = cur {
                if match_from(doc, s, sel, idx - 1, ctx) {
                    return true;
                }
                cur = doc.prev_element_sibling(s);
            }
            false
        }
    }
}

fn matches_compound(doc: &Document, node: NodeId, compound: &CompoundSelector, ctx: &MatchContext) -> bool {
    compound.simples.iter().all(|s| matches_simple(doc, node, s, ctx))
}

fn matches_simple(doc: &Document, node: NodeId, simple: &SimpleSelector, ctx: &MatchContext) -> bool {
    let elem = match doc.node(node) {
        Some(e) if e.is_element() => e,
        _ => return false,
    };
    match simple {
        SimpleSelector::Universal => true,
        SimpleSelector::Type(t) => elem.tag().eq_ignore_ascii_case(t),
        SimpleSelector::Id(id) => doc.attr(node, "id") == Some(id.as_str()),
        SimpleSelector::Class(c) => doc.has_class(node, c),
        SimpleSelector::Attribute { name, op, value, case_insensitive } => {
            let actual = match doc.attr(node, name) {
                Some(a) => a.to_string(),
                None => return false,
            };
            let (actual_cmp, value_cmp) = if *case_insensitive {
                (actual.to_ascii_lowercase(), value.to_ascii_lowercase())
            } else {
                (actual.clone(), value.clone())
            };
            match op {
                AttrOp::Exists => true,
                AttrOp::Equals => actual_cmp == value_cmp,
                AttrOp::Includes => actual_cmp.split_ascii_whitespace().any(|w| w == value_cmp),
                AttrOp::DashMatch => actual_cmp == value_cmp || actual_cmp.starts_with(&format!("{}-", value_cmp)),
                AttrOp::Prefix => !value_cmp.is_empty() && actual_cmp.starts_with(&value_cmp),
                AttrOp::Suffix => !value_cmp.is_empty() && actual_cmp.ends_with(&value_cmp),
                AttrOp::Substring => !value_cmp.is_empty() && actual_cmp.contains(&value_cmp),
            }
        }
        SimpleSelector::PseudoClass(p) => matches_pseudo(doc, node, p, ctx),
    }
}

fn matches_pseudo(doc: &Document, node: NodeId, pseudo: &PseudoClass, ctx: &MatchContext) -> bool {
    match pseudo {
        PseudoClass::FirstChild => doc.prev_element_sibling(node).is_none(),
        PseudoClass::LastChild => doc.next_element_sibling(node).is_none(),
        PseudoClass::OnlyChild => doc.prev_element_sibling(node).is_none() && doc.next_element_sibling(node).is_none(),
        PseudoClass::NthChild(a, b) => anb_matches(*a, *b, doc.element_index(node, false)),
        PseudoClass::NthLastChild(a, b) => anb_matches(*a, *b, doc.element_index(node, true)),
        PseudoClass::FirstOfType => {
            doc.prev_element_sibling(node).map(|s| doc.tag_name(s) != doc.tag_name(node)).unwrap_or(true)
        }
        PseudoClass::LastOfType => {
            doc.next_element_sibling(node).map(|s| doc.tag_name(s) != doc.tag_name(node)).unwrap_or(true)
        }
        PseudoClass::OnlyOfType => {
            let tag = doc.tag_name(node);
            doc.element_siblings(node).into_iter().filter(|s| doc.tag_name(*s) == tag).count() == 1
        }
        PseudoClass::NthOfType(a, b) => anb_matches(*a, *b, doc.element_index_of_type(node, false)),
        PseudoClass::NthLastOfType(a, b) => anb_matches(*a, *b, doc.element_index_of_type(node, true)),
        PseudoClass::Root => doc.document_element() == Some(node),
        PseudoClass::Empty => doc
            .children(node)
            .iter()
            .all(|c| match doc.node(*c) {
                Some(n) => match &n.node_type {
                    crate::NodeType::Comment(_) => true,
                    crate::NodeType::Text(t) => t.is_empty(),
                    _ => false,
                },
                None => true,
            }),
        PseudoClass::Not(list) => !list.iter().any(|s| matcher(doc, node, s, ctx)),
        PseudoClass::Is(list) | PseudoClass::Where(list) => list.iter().any(|s| matcher(doc, node, s, ctx)),
        PseudoClass::Link => {
            matches!(doc.tag_name(node), "a" | "area" | "link")
                && doc.attr(node, "href").is_some()
                && !ctx.visited.contains(&node)
        }
        PseudoClass::Visited => ctx.visited.contains(&node),
        PseudoClass::Checked => doc.attr(node, "checked").is_some() || doc.attr(node, "selected").is_some(),
        PseudoClass::Disabled => doc.attr(node, "disabled").is_some(),
        PseudoClass::Enabled => is_form_control(doc.tag_name(node)) && doc.attr(node, "disabled").is_none(),
        PseudoClass::Required => doc.attr(node, "required").is_some(),
        PseudoClass::Optional => is_form_control(doc.tag_name(node)) && doc.attr(node, "required").is_none(),
        PseudoClass::Focus => ctx.focus == Some(node),
        PseudoClass::Hover => ctx.hover == Some(node),
        PseudoClass::Active => ctx.active == Some(node),
        PseudoClass::Target => {
            matches!(ctx.target, Some(t) if doc.attr(node, "id") == Some(t))
        }
        PseudoClass::Scope => ctx.scope.map(|s| s == node).unwrap_or_else(|| doc.document_element() == Some(node)),
        PseudoClass::Host(list) => {
            let host = ctx.scope.and_then(|root| doc.host_of(root));
            if host != Some(node) {
                false
            } else if let Some(list) = list {
                list.iter().any(|sel| matcher(doc, node, sel, ctx))
            } else {
                true
            }
        }
        PseudoClass::PlaceholderShown => doc.attr(node, "placeholder").is_some(),
        PseudoClass::Lang(lang) => {
            let mut cur = Some(node);
            while let Some(n) = cur {
                if let Some(l) = doc.attr(n, "lang").or_else(|| doc.attr(n, "xml:lang")) {
                    let l = l.to_ascii_lowercase();
                    return l == *lang || l.starts_with(&format!("{lang}-"));
                }
                cur = doc.parent(n);
            }
            false
        }
        PseudoClass::Any(_) => false,
    }
}

fn is_form_control(tag: &str) -> bool {
    matches!(
        tag,
        "input" | "button" | "select" | "textarea" | "option" | "optgroup" | "fieldset"
    )
}

fn anb_matches(a: i32, b: i32, index: usize) -> bool {
    let i = index as i32;
    if a == 0 {
        return i == b;
    }
    let diff = i - b;
    diff % a == 0 && diff / a >= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compound_and_specificity() {
        let s = Selector::parse("div.foo#bar[data-x=\"1\"]").unwrap();
        assert_eq!(s.compounds.len(), 1);
        assert_eq!(s.specificity, Specificity { a: 1, b: 2, c: 1 });
    }

    #[test]
    fn parses_combinators() {
        let s = Selector::parse("main > ul li + a ~ span").unwrap();
        assert_eq!(s.compounds.len(), 5);
        assert_eq!(
            s.combinators,
            vec![
                Combinator::Child,
                Combinator::Descendant,
                Combinator::NextSibling,
                Combinator::SubsequentSibling
            ]
        );
    }

    #[test]
    fn parses_pseudo_elements_and_not() {
        let s = Selector::parse("p::before").unwrap();
        assert_eq!(s.pseudo_element.as_deref(), Some("before"));
        let s = Selector::parse("a:not(.x, .y):hover").unwrap();
        assert_eq!(s.compounds[0].simples.len(), 3);
    }

    #[test]
    fn nth_formulas() {
        assert_eq!(parse_anb("2n+1"), Some((2, 1)));
        assert_eq!(parse_anb("odd"), Some((2, 1)));
        assert_eq!(parse_anb("-n+3"), Some((-1, 3)));
        assert_eq!(parse_anb("3"), Some((0, 3)));
        assert!(anb_matches(2, 1, 3));
        assert!(!anb_matches(2, 1, 4));
        assert!(anb_matches(-1, 3, 2));
    }
}
