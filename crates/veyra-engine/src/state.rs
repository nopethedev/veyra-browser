//! Shared mutable state between the page controller and the JavaScript DOM
//! bindings. Everything here is single-threaded, like a browser's main thread.

use std::collections::{HashMap, HashSet};

use veyra_common::{Point, Rect, Size, Url};
use veyra_css::StyleEngine;
use veyra_dom::{Document, NodeId};
use veyra_js::Value;
use veyra_paint::ImageCache;

#[derive(Clone)]
pub struct Listener {
    pub node: NodeId,
    pub event: String,
    pub callback: Value,
    pub once: bool,
}

#[derive(Clone)]
pub struct Observer {
    pub id: u64,
    pub target: NodeId,
    pub callback: Value,
    pub owner: Option<Value>,
    pub subtree: bool,
    pub child_list: bool,
    pub attributes: bool,
    pub character_data: bool,
}

#[derive(Clone)]
pub struct Mutation {
    pub kind: &'static str,
    pub target: NodeId,
    pub added: Vec<NodeId>,
    pub removed: Vec<NodeId>,
    pub attribute: Option<String>,
}

pub struct DomState {
    pub doc: Document,
    pub styles: StyleEngine,
    pub images: ImageCache,
    pub base: Url,
    /// Author stylesheets fetched from `<link rel=stylesheet>`.
    pub net_css: Vec<String>,
    pub extra_css: Vec<String>,
    pub wrappers: HashMap<NodeId, Value>,
    pub listeners: Vec<Listener>,
    pub observers: Vec<Observer>,
    pub mutations: Vec<Mutation>,
    pub next_observer: u64,
    pub dirty: bool,
    pub boxes: HashMap<NodeId, Rect>,
    pub viewport: Size,
    pub scroll: Point,
    pub storage: HashMap<String, String>,
    pub session_storage: HashMap<String, String>,
    pub focused: Option<NodeId>,
    pub navigation: Option<Url>,
    pub reload: bool,
    pub title: String,
    pub form_values: HashMap<NodeId, String>,
    pub checked: HashMap<NodeId, bool>,
    pub media_dark: bool,
    pub current_script: Option<NodeId>,
    /// Cookies set through `document.cookie` (name, value).
    pub cookies: Vec<(String, String)>,
    /// Set when script assigns a new image `src`; the page fetches it at the
    /// next pump.
    pub images_dirty: bool,
    /// Registered custom element constructors (tag name -> constructor).
    pub custom_elements: HashMap<String, Value>,
    /// Elements whose constructor has already run.
    pub upgraded_elements: HashSet<NodeId>,
    /// Elements whose `connectedCallback` is currently active.
    pub connected_elements: HashSet<NodeId>,
    /// Observed attributes per upgraded element.
    pub observed_attributes: HashMap<NodeId, Vec<String>>,
    /// Attribute changes queued for `attributeChangedCallback`.
    pub attribute_changes: Vec<(NodeId, String, Option<String>, Option<String>)>,
    /// Shadow root modes (`open`/`closed`) by root node.
    pub shadow_modes: HashMap<NodeId, String>,
    /// Constructor `prototype` objects exposed to page scripts, keyed by the
    /// interface name (`Element`, `Node`, `HTMLDivElement`, ...).
    pub interface_protos: HashMap<String, Value>,
    /// Element wrapper prototype by lowercase tag name.
    pub element_protos: HashMap<String, Value>,
    /// Slotted light-DOM children detached while their host had no stamped
    /// slot yet (ShadyDOM clears light DOM before it appends the template).
    /// They are re-inserted into the matching slot once it exists.
    pub pending_slotted: HashMap<NodeId, Vec<NodeId>>,
    /// Stable DocumentFragment backing each template's `content`. Polymer
    /// mutates this fragment while parsing and stamps it later, so returning a
    /// fresh clone on every read breaks its nodeInfoList paths.
    pub template_contents: HashMap<NodeId, NodeId>,
    /// Explicit `node.host` assignments (ShadyDOM's ShadowRoot shim stores its
    /// host as a plain property on the upgraded fragment).
    pub host_overrides: HashMap<NodeId, NodeId>,
    /// Scripts inserted by page code (`document.head.appendChild(script)`).
    /// The initial parser scripts are run in one batch, but dynamic module
    /// loaders depend on each inserted script executing and firing `load`.
    pub pending_scripts: Vec<NodeId>,
    pub executed_scripts: HashSet<NodeId>,
}

impl DomState {
    pub fn new(doc: Document, viewport: Size) -> DomState {
        let base = doc.base_url.clone();
        let mut styles = StyleEngine::new();
        styles.set_viewport(viewport.w, viewport.h);
        let title = doc.title.clone();
        DomState {
            doc,
            styles,
            images: ImageCache::new(),
            base,
            net_css: Vec::new(),
            extra_css: Vec::new(),
            wrappers: HashMap::new(),
            listeners: Vec::new(),
            observers: Vec::new(),
            mutations: Vec::new(),
            next_observer: 1,
            dirty: true,
            boxes: HashMap::new(),
            viewport,
            scroll: Point::ZERO,
            storage: HashMap::new(),
            session_storage: HashMap::new(),
            focused: None,
            navigation: None,
            reload: false,
            title,
            form_values: HashMap::new(),
            checked: HashMap::new(),
            media_dark: false,
            current_script: None,
            cookies: Vec::new(),
            images_dirty: false,
            custom_elements: HashMap::new(),
            upgraded_elements: HashSet::new(),
            connected_elements: HashSet::new(),
            observed_attributes: HashMap::new(),
            attribute_changes: Vec::new(),
            shadow_modes: HashMap::new(),
            interface_protos: HashMap::new(),
            element_protos: HashMap::new(),
            pending_slotted: HashMap::new(),
            template_contents: HashMap::new(),
            host_overrides: HashMap::new(),
            pending_scripts: Vec::new(),
            executed_scripts: HashSet::new(),
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn note_mutation(&mut self, m: Mutation) {
        self.dirty = true;
        if self.mutations.len() < 2000 {
            self.mutations.push(m);
        }
    }

    pub fn resolve(&self, reference: &str) -> Url {
        self.base.resolve(reference)
    }

    pub fn node_tag(&self, node: NodeId) -> String {
        self.doc.tag_name(node).to_string()
    }

    pub fn element_children(&self, node: NodeId) -> Vec<NodeId> {
        self.doc.children(node).iter().copied().filter(|n| self.doc.is_element(*n)).collect()
    }

    pub fn is_form_field(&self, node: NodeId) -> bool {
        matches!(
            self.doc.tag_name(node),
            "input" | "textarea" | "select" | "button"
        )
    }
}
