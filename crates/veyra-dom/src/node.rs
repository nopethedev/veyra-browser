//! Arena-backed DOM tree.

use std::collections::HashMap;

use veyra_common::util::escape_html;
use veyra_common::Url;

use crate::selector::{MatchContext, Selector};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl NodeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeType {
    Document,
    Doctype(String),
    Element(String),
    Text(String),
    Comment(String),
}

#[derive(Debug, Clone)]
pub struct Node {
    pub node_type: NodeType,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub attrs: Vec<(String, String)>,
    pub source_line: u32,
    /// Set when the HTML parser inserts nodes; used by the inspector.
    pub generated: bool,
}

impl Node {
    pub fn is_element(&self) -> bool {
        matches!(self.node_type, NodeType::Element(_))
    }
    pub fn is_text(&self) -> bool {
        matches!(self.node_type, NodeType::Text(_))
    }
    pub fn tag(&self) -> &str {
        match &self.node_type {
            NodeType::Element(t) => t,
            _ => "",
        }
    }
    pub fn node_value(&self) -> Option<&str> {
        match &self.node_type {
            NodeType::Text(t) | NodeType::Comment(t) => Some(t),
            NodeType::Doctype(t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    nodes: Vec<Node>,
    pub root: NodeId,
    pub base_url: Url,
    pub url: Url,
    pub quirks_mode: bool,
    pub title: String,
    pub content_type: String,
    pub ready_state: String,
    /// Document element cache maintained by the parser.
    pub html: Option<NodeId>,
    pub head: Option<NodeId>,
    pub body: Option<NodeId>,
    /// Shadow trees: host element -> shadow root node.
    pub shadow_roots: HashMap<NodeId, NodeId>,
    /// Reverse map: shadow root -> host element.
    pub shadow_hosts: HashMap<NodeId, NodeId>,
    /// Slot element -> assigned light-DOM nodes.
    pub slot_assignments: HashMap<NodeId, Vec<NodeId>>,
}

impl Document {
    pub fn new(url: Url) -> Document {
        let mut doc = Document {
            nodes: Vec::new(),
            root: NodeId(0),
            base_url: url.clone(),
            url,
            quirks_mode: false,
            title: String::new(),
            content_type: "text/html".into(),
            ready_state: "loading".into(),
            html: None,
            head: None,
            body: None,
            shadow_roots: HashMap::new(),
            shadow_hosts: HashMap::new(),
            slot_assignments: HashMap::new(),
        };
        let root = doc.alloc(NodeType::Document);
        doc.root = root;
        doc
    }

    pub fn alloc(&mut self, node_type: NodeType) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            node_type,
            parent: None,
            children: Vec::new(),
            attrs: Vec::new(),
            source_line: 0,
            generated: false,
        });
        id
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.index())
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn create_element(&mut self, tag: &str) -> NodeId {
        self.alloc(NodeType::Element(tag.to_ascii_lowercase()))
    }

    pub fn create_text(&mut self, text: &str) -> NodeId {
        self.alloc(NodeType::Text(text.to_string()))
    }

    pub fn create_comment(&mut self, text: &str) -> NodeId {
        self.alloc(NodeType::Comment(text.to_string()))
    }

    // ---- tree mutation ---------------------------------------------------

    pub fn append(&mut self, parent: NodeId, child: NodeId) {
        if parent == child || self.is_ancestor(child, parent) {
            return;
        }
        self.detach(child);
        if let Some(n) = self.node_mut(child) {
            n.parent = Some(parent);
        }
        if let Some(p) = self.node_mut(parent) {
            p.children.push(child);
        }
        self.refresh_caches();
    }

    pub fn insert_before(&mut self, parent: NodeId, child: NodeId, reference: Option<NodeId>) {
        if parent == child || self.is_ancestor(child, parent) {
            return;
        }
        self.detach(child);
        let ref_idx = reference.and_then(|r| {
            self.node(parent).and_then(|p| p.children.iter().position(|c| *c == r))
        });
        if let Some(n) = self.node_mut(child) {
            n.parent = Some(parent);
        }
        if let Some(p) = self.node_mut(parent) {
            match ref_idx {
                Some(i) => p.children.insert(i, child),
                None => p.children.push(child),
            }
        }
        self.refresh_caches();
    }

    pub fn detach(&mut self, node: NodeId) {
        let parent = self.node(node).and_then(|n| n.parent);
        if let Some(p) = parent {
            if let Some(pn) = self.node_mut(p) {
                pn.children.retain(|c| *c != node);
            }
            if let Some(n) = self.node_mut(node) {
                n.parent = None;
            }
        }
    }

    pub fn remove(&mut self, node: NodeId) {
        self.detach(node);
        self.refresh_caches();
    }

    pub fn replace_child(&mut self, parent: NodeId, new_child: NodeId, old_child: NodeId) {
        if let Some(p) = self.node(parent) {
            if let Some(idx) = p.children.iter().position(|c| *c == old_child) {
                self.detach(old_child);
                self.detach(new_child);
                if let Some(n) = self.node_mut(new_child) {
                    n.parent = Some(parent);
                }
                if let Some(p) = self.node_mut(parent) {
                    let idx = idx.min(p.children.len());
                    p.children.insert(idx, new_child);
                }
            }
        }
        self.refresh_caches();
    }

    pub fn is_ancestor(&self, maybe_ancestor: NodeId, node: NodeId) -> bool {
        let mut cur = self.parent(node);
        while let Some(n) = cur {
            if n == maybe_ancestor {
                return true;
            }
            cur = self.parent(n);
        }
        false
    }

    pub fn is_in_document(&self, node: NodeId) -> bool {
        let mut cur = Some(node);
        while let Some(n) = cur {
            if n == self.root {
                return true;
            }
            // A shadow root is not in the light tree; continue through its host.
            cur = match self.parent(n) {
                Some(p) => Some(p),
                None => self.shadow_hosts.get(&n).copied(),
            };
        }
        false
    }

    /// Records the host of a shadow root created outside `attach_shadow`
    /// (the ShadyDOM compatibility layer builds its own fragment).
    pub fn set_shadow_host(&mut self, root: NodeId, host: NodeId) {
        self.shadow_hosts.insert(root, host);
    }

    /// Attaches a shadow root to `host`, or returns the existing one.
    pub fn attach_shadow(&mut self, host: NodeId) -> NodeId {
        if let Some(root) = self.shadow_roots.get(&host) {
            return *root;
        }
        let root = self.create_element("#shadow-root");
        self.shadow_roots.insert(host, root);
        self.shadow_hosts.insert(root, host);
        root
    }

    pub fn shadow_root(&self, host: NodeId) -> Option<NodeId> {
        self.shadow_roots.get(&host).copied()
    }

    pub fn host_of(&self, root: NodeId) -> Option<NodeId> {
        self.shadow_hosts.get(&root).copied()
    }

    pub fn is_shadow_root(&self, node: NodeId) -> bool {
        self.shadow_hosts.contains_key(&node)
    }

    /// True when `node` is a shadow-tree descendant of `root` (not slotted
    /// light content, which belongs to the outer tree).
    pub fn in_shadow_tree(&self, node: NodeId, root: NodeId) -> bool {
        let mut cur = Some(node);
        while let Some(n) = cur {
            if n == root {
                return true;
            }
            cur = self.parent(n);
        }
        false
    }

    /// True when the node lives inside any shadow tree.
    pub fn is_in_shadow(&self, node: NodeId) -> bool {
        let mut cur = Some(node);
        while let Some(n) = cur {
            if self.shadow_hosts.contains_key(&n) {
                return true;
            }
            cur = self.parent(n);
        }
        false
    }

    /// True when `node` is assigned to a slot inside `root`'s shadow tree.
    pub fn assigned_in_shadow(&self, node: NodeId, root: NodeId) -> bool {
        self.slot_assignments.iter().any(|(slot, assigned)| {
            self.in_shadow_tree(*slot, root) && assigned.contains(&node)
        })
    }

    /// The name of the slot a light-DOM child is assigned to.
    pub fn slot_name(&self, node: NodeId) -> &str {
        self.attr(node, "slot").unwrap_or("")
    }

    /// Recomputes slot assignment for every shadow host.
    pub fn distribute_slots(&mut self) {
        self.slot_assignments.clear();
        let hosts: Vec<(NodeId, NodeId)> = self.shadow_roots.iter().map(|(h, r)| (*h, *r)).collect();
        for (host, root) in hosts {
            let light: Vec<NodeId> = self.children(host).to_vec();
            let mut stack = vec![root];
            let mut slots = Vec::new();
            while let Some(n) = stack.pop() {
                if self.tag_name(n) == "slot" {
                    slots.push(n);
                }
                stack.extend(self.children(n).iter().copied());
            }
            for slot in slots {
                let name = self.attr(slot, "name").unwrap_or("");
                let assigned: Vec<NodeId> = light
                    .iter()
                    .copied()
                    .filter(|c| {
                        self.node(*c).map(|n| n.is_element() || n.is_text()).unwrap_or(false)
                            && self.slot_name(*c) == name
                    })
                    .collect();
                self.slot_assignments.insert(slot, assigned);
            }
        }
    }

    /// Children used for layout/painting. A shadow host renders its shadow
    /// tree; a slot renders its assigned light children (or its fallback).
    pub fn rendered_children(&self, node: NodeId) -> Vec<NodeId> {
        if let Some(assigned) = self.slot_assignments.get(&node) {
            if !assigned.is_empty() {
                return assigned.clone();
            }
        }
        if let Some(root) = self.shadow_roots.get(&node) {
            return self.children(*root).to_vec();
        }
        self.children(node).to_vec()
    }

    fn refresh_caches(&mut self) {
        if let Some(html) = self.html {
            if self.parent(html) != Some(self.root) {
                self.html = None;
                self.head = None;
                self.body = None;
            }
        }
        if self.html.is_none() {
            self.html = self
                .node(self.root)
                .and_then(|r| r.children.iter().copied().find(|c| self.tag_name(*c) == "html"));
        }
        if self.head.is_none() {
            self.head = self.html.and_then(|h| {
                self.node(h).and_then(|n| n.children.iter().copied().find(|c| self.tag_name(*c) == "head"))
            });
        }
        if self.body.is_none() {
            self.body = self.html.and_then(|h| {
                self.node(h).and_then(|n| n.children.iter().copied().find(|c| self.tag_name(*c) == "body"))
            });
        }
    }

    pub fn document_element(&self) -> Option<NodeId> {
        self.html.or_else(|| {
            self.node(self.root)
                .and_then(|r| r.children.iter().copied().find(|c| self.node(*c).map(|n| n.is_element()).unwrap_or(false)))
        })
    }

    pub fn head(&self) -> Option<NodeId> {
        self.head
    }

    pub fn body(&self) -> Option<NodeId> {
        self.body
    }

    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.node(node).and_then(|n| n.parent)
    }

    pub fn children(&self, node: NodeId) -> &[NodeId] {
        self.node(node).map(|n| n.children.as_slice()).unwrap_or(&[])
    }

    pub fn attrs(&self, node: NodeId) -> &[(String, String)] {
        self.node(node).map(|n| n.attrs.as_slice()).unwrap_or(&[])
    }

    pub fn element_children(&self, node: NodeId) -> Vec<NodeId> {
        self.children(node)
            .iter()
            .copied()
            .filter(|c| self.node(*c).map(|n| n.is_element()).unwrap_or(false))
            .collect()
    }

    pub fn element_siblings(&self, node: NodeId) -> Vec<NodeId> {
        match self.parent(node) {
            Some(p) => self.element_children(p),
            None => vec![node],
        }
    }

    pub fn prev_element_sibling(&self, node: NodeId) -> Option<NodeId> {
        let siblings = self.element_siblings(node);
        let idx = siblings.iter().position(|c| *c == node)?;
        if idx == 0 {
            None
        } else {
            Some(siblings[idx - 1])
        }
    }

    pub fn next_element_sibling(&self, node: NodeId) -> Option<NodeId> {
        let siblings = self.element_siblings(node);
        let idx = siblings.iter().position(|c| *c == node)?;
        siblings.get(idx + 1).copied()
    }

    pub fn element_index(&self, node: NodeId, from_end: bool) -> usize {
        let siblings = self.element_siblings(node);
        let idx = siblings.iter().position(|c| *c == node).unwrap_or(0);
        if from_end {
            siblings.len() - idx
        } else {
            idx + 1
        }
    }

    pub fn element_index_of_type(&self, node: NodeId, from_end: bool) -> usize {
        let tag = self.tag_name(node);
        let siblings: Vec<NodeId> = self
            .element_siblings(node)
            .into_iter()
            .filter(|s| self.tag_name(*s) == tag)
            .collect();
        let idx = siblings.iter().position(|c| *c == node).unwrap_or(0);
        if from_end {
            siblings.len() - idx
        } else {
            idx + 1
        }
    }

    pub fn tag_name(&self, node: NodeId) -> &str {
        self.node(node).map(|n| n.tag()).unwrap_or("")
    }

    pub fn node_name(&self, node: NodeId) -> String {
        match self.node(node).map(|n| &n.node_type) {
            Some(NodeType::Element(t)) => t.to_ascii_uppercase(),
            Some(NodeType::Text(_)) => "#text".into(),
            Some(NodeType::Comment(_)) => "#comment".into(),
            Some(NodeType::Document) => "#document".into(),
            Some(NodeType::Doctype(_)) => "#doctype".into(),
            None => String::new(),
        }
    }

    pub fn is_element(&self, node: NodeId) -> bool {
        self.node(node).map(|n| n.is_element()).unwrap_or(false)
    }

    // ---- attributes ------------------------------------------------------

    pub fn attr(&self, node: NodeId, name: &str) -> Option<&str> {
        self.node(node)
            .and_then(|n| n.attrs.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()))
    }

    pub fn attr_pairs(&self, node: NodeId) -> &[(String, String)] {
        self.node(node).map(|n| n.attrs.as_slice()).unwrap_or(&[])
    }

    pub fn set_attr(&mut self, node: NodeId, name: &str, value: &str) {
        if let Some(n) = self.node_mut(node) {
            let lower = name.to_ascii_lowercase();
            if let Some(slot) = n.attrs.iter_mut().find(|(k, _)| *k == lower) {
                slot.1 = value.to_string();
            } else {
                n.attrs.push((lower, value.to_string()));
            }
        }
    }

    pub fn remove_attr(&mut self, node: NodeId, name: &str) {
        if let Some(n) = self.node_mut(node) {
            n.attrs.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        }
    }

    pub fn has_class(&self, node: NodeId, class: &str) -> bool {
        self.attr(node, "class")
            .map(|c| c.split_ascii_whitespace().any(|x| x == class))
            .unwrap_or(false)
    }

    pub fn classes(&self, node: NodeId) -> Vec<String> {
        self.attr(node, "class")
            .map(|c| c.split_ascii_whitespace().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }

    pub fn add_class(&mut self, node: NodeId, class: &str) {
        let mut classes = self.classes(node);
        if !classes.iter().any(|c| c == class) {
            classes.push(class.to_string());
            self.set_attr(node, "class", &classes.join(" "));
        }
    }

    pub fn remove_class(&mut self, node: NodeId, class: &str) {
        let mut classes = self.classes(node);
        classes.retain(|c| c != class);
        self.set_attr(node, "class", &classes.join(" "));
    }

    pub fn toggle_class(&mut self, node: NodeId, class: &str, force: Option<bool>) -> bool {
        let has = self.has_class(node, class);
        let next = force.unwrap_or(!has);
        if next && !has {
            self.add_class(node, class);
        } else if !next && has {
            self.remove_class(node, class);
        }
        next
    }

    pub fn id(&self, node: NodeId) -> Option<&str> {
        self.attr(node, "id")
    }

    // ---- content ---------------------------------------------------------

    pub fn text_content(&self, node: NodeId) -> String {
        let mut out = String::new();
        self.collect_text(node, &mut out);
        out
    }

    fn collect_text(&self, node: NodeId, out: &mut String) {
        match self.node(node).map(|n| &n.node_type) {
            Some(NodeType::Text(t)) => out.push_str(t),
            Some(NodeType::Element(_)) | Some(NodeType::Document) => {
                for c in self.children(node) {
                    self.collect_text(*c, out);
                }
            }
            _ => {}
        }
    }

    /// Text used for rendering and find-in-page; omits scripts, styles and
    /// the document head.
    pub fn visible_text(&self) -> String {
        let mut out = String::new();
        let start = self.body().or_else(|| self.document_element()).unwrap_or(self.root);
        self.collect_visible_text(start, &mut out);
        out
    }

    fn collect_visible_text(&self, node: NodeId, out: &mut String) {
        if self.is_skipped_for_rendering(node) {
            return;
        }
        match self.node(node).map(|n| &n.node_type) {
            Some(NodeType::Text(t)) => out.push_str(t),
            _ => {
                for c in self.children(node) {
                    self.collect_visible_text(*c, out);
                    let tag = self.tag_name(*c);
                    if matches!(tag, "p" | "div" | "br" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "tr" | "section") {
                        out.push('\n');
                    }
                }
            }
        }
    }

    pub fn is_skipped_for_rendering(&self, node: NodeId) -> bool {
        matches!(self.tag_name(node), "script" | "style" | "head" | "title" | "meta" | "link" | "template" | "noscript")
    }

    pub fn set_text_content(&mut self, node: NodeId, text: &str) {
        // Character data nodes hold their value directly; replacing their
        // textContent must not append a child to a leaf node.
        match self.node(node).map(|n| n.node_type.clone()) {
            Some(NodeType::Text(_)) => {
                if let Some(n) = self.node_mut(node) {
                    n.node_type = NodeType::Text(text.to_string());
                }
                return;
            }
            Some(NodeType::Comment(_)) => {
                if let Some(n) = self.node_mut(node) {
                    n.node_type = NodeType::Comment(text.to_string());
                }
                return;
            }
            _ => {}
        }
        // Remove existing children.
        let children = self.children(node).to_vec();
        for c in children {
            self.detach(c);
        }
        if !text.is_empty() {
            let t = self.create_text(text);
            self.append(node, t);
        }
    }

    pub fn inner_html(&self, node: NodeId) -> String {
        let mut out = String::new();
        for c in self.children(node) {
            self.serialize(*c, &mut out);
        }
        out
    }

    pub fn outer_html(&self, node: NodeId) -> String {
        let mut out = String::new();
        self.serialize(node, &mut out);
        out
    }

    pub fn serialize(&self, node: NodeId, out: &mut String) {
        match self.node(node).map(|n| &n.node_type) {
            Some(NodeType::Text(t)) => out.push_str(&escape_html(t)),
            Some(NodeType::Comment(t)) => {
                out.push_str("<!--");
                out.push_str(t);
                out.push_str("-->");
            }
            Some(NodeType::Doctype(name)) => {
                out.push_str("<!DOCTYPE ");
                out.push_str(name);
                out.push('>');
            }
            Some(NodeType::Element(tag)) => {
                out.push('<');
                out.push_str(tag);
                for (k, v) in self.attr_pairs(node) {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    out.push_str(&escape_html(v));
                    out.push('"');
                }
                out.push('>');
                if is_void_element(tag) {
                    return;
                }
                if matches!(tag.as_str(), "script" | "style" | "textarea" | "title") {
                    for c in self.children(node) {
                        if let Some(NodeType::Text(t)) = self.node(*c).map(|n| &n.node_type) {
                            out.push_str(t);
                        }
                    }
                } else {
                    for c in self.children(node) {
                        self.serialize(*c, out);
                    }
                }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
            Some(NodeType::Document) => {
                for c in self.children(node) {
                    self.serialize(*c, out);
                }
            }
            None => {}
        }
    }

    pub fn clone_node(&mut self, node: NodeId, deep: bool) -> NodeId {
        let new_id = match self.node(node).map(|n| n.node_type.clone()) {
            Some(nt) => {
                let id = self.alloc(nt);
                for (k, v) in self.attr_pairs(node).to_vec() {
                    self.set_attr(id, &k, &v);
                }
                id
            }
            None => return self.alloc(NodeType::Comment(String::new())),
        };
        if deep {
            for c in self.children(node).to_vec() {
                let cloned = self.clone_node(c, true);
                self.append(new_id, cloned);
            }
        }
        new_id
    }

    // ---- queries ---------------------------------------------------------

    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        self.walk::<NodeId, _>(self.root, &mut |doc, n| {
            if doc.is_element(n) && doc.attr(n, "id") == Some(id) {
                Some(n)
            } else {
                None
            }
        })
    }

    pub fn get_elements_by_tag_name(&self, tag: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.walk::<NodeId, _>(self.root, &mut |doc, n| {
            if doc.is_element(n) && (tag == "*" || doc.tag_name(n).eq_ignore_ascii_case(tag)) {
                out.push(n);
            }
            None
        });
        out
    }

    pub fn get_elements_by_class_name(&self, class: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.walk::<NodeId, _>(self.root, &mut |doc, n| {
            if doc.has_class(n, class) {
                out.push(n);
            }
            None
        });
        out
    }

    pub fn query_selector(&self, scope: NodeId, selector: &str) -> Option<NodeId> {
        let list = Selector::parse_list(selector)?;
        let ctx = MatchContext { scope: Some(scope), ..Default::default() };
        self.walk_including::<NodeId, _>(scope, &mut |doc, n| {
            if doc.is_element(n) && crate::selector::matches_selector_list(doc, n, &list, &ctx) {
                Some(n)
            } else {
                None
            }
        })
    }

    pub fn query_selector_all(&self, scope: NodeId, selector: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        if let Some(list) = Selector::parse_list(selector) {
            let ctx = MatchContext { scope: Some(scope), ..Default::default() };
            self.walk_including::<NodeId, _>(scope, &mut |doc, n| {
                if doc.is_element(n) && crate::selector::matches_selector_list(doc, n, &list, &ctx) {
                    out.push(n);
                }
                None
            });
        }
        out
    }

    pub fn matches(&self, node: NodeId, selector: &str) -> bool {
        match Selector::parse_list(selector) {
            Some(list) => crate::selector::matches_selector_list(self, node, &list, &MatchContext::default()),
            None => false,
        }
    }

    pub fn closest(&self, node: NodeId, selector: &str) -> Option<NodeId> {
        let list = Selector::parse_list(selector)?;
        let ctx = MatchContext::default();
        let mut cur = Some(node);
        while let Some(n) = cur {
            if self.is_element(n) && crate::selector::matches_selector_list(self, n, &list, &ctx) {
                return Some(n);
            }
            cur = self.parent(n);
        }
        None
    }

    /// Depth-first pre-order walk of the whole document.
    pub fn walk<T, F>(&self, node: NodeId, f: &mut F) -> Option<T>
    where
        F: FnMut(&Document, NodeId) -> Option<T>,
    {
        if let Some(found) = f(self, node) {
            return Some(found);
        }
        for c in self.children(node) {
            if let Some(found) = self.walk(*c, f) {
                return Some(found);
            }
        }
        None
    }

    pub fn walk_including<T, F>(&self, start: NodeId, f: &mut F) -> Option<T>
    where
        F: FnMut(&Document, NodeId) -> Option<T>,
    {
        self.walk(start, f)
    }

    pub fn elements_in_document_order(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.walk::<NodeId, _>(self.root, &mut |doc, n| {
            if doc.is_element(n) {
                out.push(n);
            }
            None
        });
        out
    }

    /// Effective base URL, honouring `<base href>`.
    pub fn effective_base(&self) -> Url {
        self.walk::<Url, _>(self.root, &mut |doc, n| {
            if doc.tag_name(n) == "base" {
                doc.attr(n, "href").map(|h| doc.base_url.resolve(h))
            } else {
                None
            }
        })
        .unwrap_or_else(|| self.base_url.clone())
    }

    pub fn set_title(&mut self) {
        let title = self.walk::<String, _>(self.root, &mut |doc, n| {
            if doc.tag_name(n) == "title" {
                Some(doc.text_content(n))
            } else {
                None
            }
        });
        if let Some(t) = title {
            self.title = t.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    }
}

pub fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::new(Url::parse("https://example.com/").unwrap())
    }

    #[test]
    fn tree_operations() {
        let mut d = doc();
        let html = d.create_element("html");
        let body = d.create_element("body");
        let p = d.create_element("p");
        let t = d.create_text("hello");
        d.append(d.root, html);
        d.append(html, body);
        d.append(body, p);
        d.append(p, t);
        assert_eq!(d.body(), Some(body));
        assert_eq!(d.text_content(body), "hello");
        assert_eq!(d.inner_html(body), "<p>hello</p>");
        assert!(d.is_in_document(t));
        d.remove(p);
        assert_eq!(d.inner_html(body), "");
        assert!(!d.is_in_document(p));
    }

    #[test]
    fn class_and_query() {
        let mut d = doc();
        let html = d.create_element("html");
        let body = d.create_element("body");
        let a = d.create_element("a");
        d.append(d.root, html);
        d.append(html, body);
        d.append(body, a);
        d.set_attr(a, "class", "btn primary");
        d.set_attr(a, "id", "go");
        assert!(d.has_class(a, "primary"));
        assert_eq!(d.get_element_by_id("go"), Some(a));
        assert_eq!(d.query_selector(d.root, "body > a.btn"), Some(a));
        assert_eq!(d.query_selector_all(d.root, ".btn").len(), 1);
        assert!(d.matches(a, "a[href], a.primary"));
        d.toggle_class(a, "primary", None);
        assert!(!d.has_class(a, "primary"));
    }

    #[test]
    fn serialisation_escapes() {
        let mut d = doc();
        let p = d.create_element("p");
        d.set_attr(p, "title", "a<b");
        let t = d.create_text("<hi> & bye");
        d.append(p, t);
        assert_eq!(d.outer_html(p), "<p title=\"a&lt;b\">&lt;hi&gt; &amp; bye</p>");
    }

    #[test]
    fn prevents_cycles() {
        let mut d = doc();
        let a = d.create_element("div");
        let b = d.create_element("span");
        d.append(d.root, a);
        d.append(a, b);
        d.append(b, a);
        assert_eq!(d.parent(a), Some(d.root));
    }
}
