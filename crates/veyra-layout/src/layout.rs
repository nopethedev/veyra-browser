//! Veyra's layout engine: block and inline flow, text line breaking, flexbox,
//! grid, tables, floats, absolute/fixed positioning and list markers.
//!
//! Layout works in CSS pixels and produces absolute rectangles, a tree of
//! boxes for backgrounds/borders/clipping, and flat lists of text fragments
//! and image placements for painting and hit-testing.

use std::collections::HashMap;
use std::rc::Rc;

use veyra_common::{Edges, Point, Rect, Size, Url};
use veyra_css::style::{FontStyle, ListStylePosition, OverflowWrap, TextDecoration, TextTransform, WordBreak};
use veyra_css::values::*;
use veyra_css::{ComputedStyle, PseudoElement, StyleEngine};
use veyra_dom::{Document, NodeId, NodeType};

use crate::font::{FontSpec, ImageInfo, TextMeasure};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    Block,
    Inline,
    Text,
    Replaced,
    Marker,
    Form,
    Flex,
    Grid,
    Table,
    TableRow,
    TableCell,
    TableCaption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormControlKind {
    TextInput,
    PasswordInput,
    Button,
    Submit,
    Checkbox,
    Radio,
    Select,
    Textarea,
    Range,
    File,
    Other,
}

#[derive(Debug, Clone)]
pub struct FormInfo {
    pub kind: FormControlKind,
    pub value: String,
    pub checked: bool,
    pub placeholder: String,
    pub disabled: bool,
    pub options: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub node: Option<NodeId>,
    pub pseudo: Option<PseudoElement>,
    pub tag: String,
    pub style: Rc<ComputedStyle>,
    pub kind: LayoutKind,
    pub rect: Rect,
    pub content: Rect,
    pub border: Edges,
    pub padding: Edges,
    pub margin: Edges,
    pub children: Vec<LayoutBox>,
    pub clip: bool,
    pub form: Option<FormInfo>,
    pub image: Option<Url>,
    pub scroll_size: Size,
}

impl LayoutBox {
    pub fn margin_box_height(&self) -> f32 {
        self.rect.h + self.margin.top + self.margin.bottom
    }
    pub fn margin_box_width(&self) -> f32 {
        self.rect.w + self.margin.left + self.margin.right
    }
    pub fn border_box(&self) -> Rect {
        self.rect
    }
    pub fn background_rect(&self) -> Rect {
        self.rect
    }
    pub fn padding_box(&self) -> Rect {
        self.rect.inset(&self.border)
    }
}

#[derive(Debug, Clone)]
pub struct TextFragment {
    pub text: String,
    pub rect: Rect,
    pub baseline: f32,
    pub font: FontSpec,
    pub color: veyra_common::Color,
    pub decoration: TextDecoration,
    pub decoration_color: Option<veyra_common::Color>,
    pub node: Option<NodeId>,
    /// Nearest element that owns this text, used by paint for ordering.
    pub owner: Option<NodeId>,
    pub opacity: f32,
    pub is_marker: bool,
}

#[derive(Debug, Clone)]
pub struct ImagePlacement {
    pub node: NodeId,
    pub url: Url,
    pub rect: Rect,
    pub border_radius: f32,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutTree {
    pub root: Option<LayoutBox>,
    pub texts: Vec<TextFragment>,
    pub images: Vec<ImagePlacement>,
    pub boxes: HashMap<NodeId, Rect>,
    pub content_size: Size,
    pub viewport: Size,
}

impl LayoutTree {
    pub fn hit_test(&self, p: Point) -> Option<NodeId> {
        fn walk(b: &LayoutBox, p: Point, depth: u32, best: &mut Option<(u32, NodeId)>) {
            if b.clip && !b.rect.contains(p) {
                return;
            }
            let inside = b.rect.contains(p);
            for c in &b.children {
                walk(c, p, depth + 1, best);
            }
            if inside {
                if let Some(node) = b.node {
                    if best.map(|(d, _)| depth >= d).unwrap_or(true) {
                        *best = Some((depth, node));
                    }
                }
            }
        }
        let mut best = None;
        if let Some(root) = &self.root {
            walk(root, p, 0, &mut best);
        }
        best.map(|(_, n)| n)
    }

    /// All text fragments belonging to a node (or its descendants).
    pub fn text_rects(&self, node: NodeId) -> Vec<Rect> {
        self.texts.iter().filter(|t| t.node == Some(node)).map(|t| t.rect).collect()
    }

    pub fn find_text_node(&self, p: Point) -> Option<NodeId> {
        self.texts
            .iter()
            .rev()
            .find(|t| t.rect.contains(p))
            .and_then(|t| t.node)
    }

    pub fn boxes_in_order(&self) -> Vec<&LayoutBox> {
        let mut out = Vec::new();
        fn walk<'a>(b: &'a LayoutBox, out: &mut Vec<&'a LayoutBox>) {
            out.push(b);
            for c in &b.children {
                walk(c, out);
            }
        }
        if let Some(root) = &self.root {
            walk(root, &mut out);
        }
        out
    }
}

pub fn layout_document(
    doc: &Document,
    styles: &StyleEngine,
    fonts: &dyn TextMeasure,
    images: &dyn ImageInfo,
    viewport: Size,
) -> LayoutTree {
    let mut engine = Engine {
        doc,
        styles,
        fonts,
        images,
        viewport,
        texts: Vec::new(),
        images_out: Vec::new(),
        cb_stack: Vec::new(),
        measuring: 0,
        depth: 0,
        active_floats: Vec::new(),
    };
    let mut tree = LayoutTree {
        viewport,
        content_size: viewport,
        ..Default::default()
    };
    let Some(html) = doc.document_element() else {
        return tree;
    };
    let style = styles.style_rc(html);
    let mut root = engine.layout_element(
        html,
        None,
        0.0,
        0.0,
        viewport.w.max(1.0),
        Some(viewport.h),
        None,
        None,
        kind_for(&style, doc.tag_name(html)),
        0,
    );
    if root.rect.h < viewport.h {
        root.rect.h = viewport.h;
        root.content.h = (viewport.h - root.padding.vertical() - root.border.vertical()).max(0.0);
    }
    let content_h = root.rect.bottom().max(viewport.h);
    let content_w = root.rect.right().max(viewport.w);
    tree.content_size = Size::new(content_w, content_h);
    tree.texts = engine.texts;
    tree.images = engine.images_out;
    tree.root = Some(root);
    let mut boxes = HashMap::new();
    if let Some(root) = &tree.root {
        collect_boxes(root, &mut boxes);
    }
    tree.boxes = boxes;
    tree
}

fn collect_boxes(b: &LayoutBox, out: &mut HashMap<NodeId, Rect>) {
    if let Some(node) = b.node {
        if b.kind != LayoutKind::Text {
            out.entry(node)
                .and_modify(|r| *r = r.union(&b.rect))
                .or_insert(b.rect);
        }
    }
    for c in &b.children {
        collect_boxes(c, out);
    }
}

/// Horizontal space reserved by active floats on the line at `y`.
fn float_insets(floats: &[(Rect, Float)], y: f32, line_h: f32, content: Rect) -> (f32, f32) {
    let mut left = 0.0f32;
    let mut right = 0.0f32;
    for (fr, side) in floats {
        if fr.bottom() <= y + 0.5 || fr.y >= y + line_h {
            continue;
        }
        match side {
            Float::Left => left = left.max(fr.right() - content.x),
            Float::Right => right = right.max(content.right() - fr.x),
            _ => {}
        }
    }
    (left.max(0.0), right.max(0.0))
}

fn kind_for(style: &ComputedStyle, tag: &str) -> LayoutKind {
    match style.display {
        Display::Flex | Display::InlineFlex => LayoutKind::Flex,
        Display::Grid | Display::InlineGrid => LayoutKind::Grid,
        Display::Table | Display::InlineTable => LayoutKind::Table,
        Display::TableRow | Display::TableRowGroup | Display::TableHeaderGroup | Display::TableFooterGroup => {
            LayoutKind::TableRow
        }
        Display::TableCell => LayoutKind::TableCell,
        Display::TableCaption => LayoutKind::TableCaption,
        _ => {
            if matches!(tag, "img" | "video" | "canvas" | "iframe" | "object" | "svg") {
                LayoutKind::Replaced
            } else if is_form_control(tag) {
                LayoutKind::Form
            } else if style.display.is_inline_level() {
                LayoutKind::Inline
            } else {
                LayoutKind::Block
            }
        }
    }
}

fn is_form_control(tag: &str) -> bool {
    matches!(tag, "input" | "button" | "select" | "textarea")
}

/// A flex container's children include anonymous items for bare text runs,
/// so `<div class="flex">1</div>` still renders its text.
#[derive(Clone, Copy)]
enum FlexItem {
    Node(NodeId),
    Text(NodeId),
}

impl FlexItem {
    fn node(self) -> NodeId {
        match self {
            FlexItem::Node(n) | FlexItem::Text(n) => n,
        }
    }
}

struct Engine<'a> {
    doc: &'a Document,
    styles: &'a StyleEngine,
    fonts: &'a dyn TextMeasure,
    images: &'a dyn ImageInfo,
    viewport: Size,
    texts: Vec<TextFragment>,
    images_out: Vec<ImagePlacement>,
    cb_stack: Vec<Rect>,
    measuring: u32,
    depth: u32,
    /// Floats from enclosing blocks whose vertical range can affect inline
    /// content laid out inside this subtree.
    active_floats: Vec<(Rect, Float)>,
}

impl<'a> Engine<'a> {
    fn style_of(&self, node: NodeId) -> Rc<ComputedStyle> {
        self.styles.style_rc(node)
    }

    fn resolve_margins(&self, style: &ComputedStyle, cb_width: f32) -> Edges {
        Edges {
            top: style.margin.top.resolve_or(cb_width, 0.0),
            right: style.margin.right.resolve_or(cb_width, 0.0),
            bottom: style.margin.bottom.resolve_or(cb_width, 0.0),
            left: style.margin.left.resolve_or(cb_width, 0.0),
        }
    }

    fn clamp_width(&self, style: &ComputedStyle, w: f32, cb: f32) -> f32 {
        let min = style.min_width.resolve(cb).unwrap_or(0.0);
        let max = style.max_width.resolve(cb).unwrap_or(f32::INFINITY);
        w.max(min).min(max).max(0.0)
    }

    fn clamp_height(&self, style: &ComputedStyle, h: f32, cb: Option<f32>) -> f32 {
        let base = cb.unwrap_or(0.0);
        let min = style.min_height.resolve(base).unwrap_or(0.0);
        let max = style.max_height.resolve(base).unwrap_or(f32::INFINITY);
        h.max(min).min(max).max(0.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_element(
        &mut self,
        node: NodeId,
        pseudo: Option<PseudoElement>,
        x: f32,
        y: f32,
        cb_width: f32,
        cb_height: Option<f32>,
        forced_w: Option<f32>,
        forced_h: Option<f32>,
        kind: LayoutKind,
        depth: u32,
    ) -> LayoutBox {
        self.depth = self.depth.max(depth);
        let style = match pseudo {
            Some(p) => self
                .styles
                .pseudo_style_rc(node, p)
                .unwrap_or_else(|| self.style_of(node)),
            None => self.style_of(node),
        };
        let tag = self.doc.tag_name(node).to_string();
        let mut margin = self.resolve_margins(&style, cb_width);
        let border = style.border_edges_width();
        let padding = style.padding.to_edges(cb_width);
        let inner_extra = padding.horizontal() + border.horizontal();
        let vertical_extra = padding.vertical() + border.vertical();

        let explicit_w = forced_w.or_else(|| style.width.resolve(cb_width));
        let mut content_w = match explicit_w {
            Some(w) => {
                if style.box_sizing == BoxSizing::BorderBox {
                    (w - inner_extra).max(0.0)
                } else {
                    w.max(0.0)
                }
            }
            None => (cb_width - margin.left - margin.right - inner_extra).max(0.0),
        };
        content_w = self.clamp_width(&style, content_w, cb_width);

        // Centre with auto margins. This must also run for a block whose width was
        // shrunk by `max-width` (e.g. Tailwind's `mx-auto max-w-5xl`), not only for
        // an explicitly sized box, otherwise the block hugs the left edge.
        {
            let border_w = content_w + inner_extra;
            let free = (cb_width - border_w - margin.left - margin.right).max(0.0);
            let left_auto = matches!(style.margin.left, Length::Auto);
            let right_auto = matches!(style.margin.right, Length::Auto);
            if left_auto && right_auto {
                margin.left = free / 2.0;
                margin.right = free / 2.0;
            } else if left_auto {
                margin.left = free;
            } else if right_auto {
                margin.right = free;
            }
        }

        let explicit_h = match forced_h {
            Some(h) => Some(h),
            None => {
                let definite = cb_height.is_some()
                    || matches!(style.height, Length::Px(_))
                    || matches!(style.height, Length::Calc { px, .. } if px != 0.0);
                match style.height.resolve(cb_height.unwrap_or(0.0)) {
                    Some(h) if definite => Some(if style.box_sizing == BoxSizing::BorderBox {
                        (h - vertical_extra).max(0.0)
                    } else {
                        h.max(0.0)
                    }),
                    _ => None,
                }
            }
        };

        let border_x = x + margin.left;
        let border_y = y + margin.top;
        let content_x = border_x + border.left + padding.left;
        let content_y = border_y + border.top + padding.top;

        // Percentage heights of children resolve against *this* box's height.
        // When it is auto (indefinite), percentages behave as auto instead of
        // arbitrarily resolving against the viewport.
        let child_cb_height = explicit_h;
        let mut children: Vec<LayoutBox> = Vec::new();
        let mut content_h = explicit_h.unwrap_or(0.0);
        let mut form = None;
        let mut image = None;
        let mut scroll_size = Size::ZERO;

        let is_replaced = matches!(kind, LayoutKind::Replaced);
        if is_replaced {
            let (iw, ih) = self.replaced_size(node, &style, &tag, content_w);
            content_h = explicit_h.unwrap_or(ih);
            if forced_w.is_none() && style.width.resolve(cb_width).is_none() {
                content_w = self.clamp_width(&style, iw, cb_width);
            }
            if let Some(src) = self.doc.attr(node, "src") {
                let url = self.doc.effective_base().resolve(src);
                if url.is_http() || url.scheme == "data" || url.scheme == "file" {
                    image = Some(url.clone());
                    if self.measuring == 0 {
                        self.images_out.push(ImagePlacement {
                            node,
                            url,
                            rect: Rect::new(content_x, content_y, content_w.max(0.0), content_h),
                            border_radius: style.border_radius,
                        });
                    }
                }
            }
        } else if matches!(kind, LayoutKind::Form) {
            form = Some(self.form_info(node, &style, &tag));
            if let Some(f) = &form {
                if explicit_h.is_none() {
                    content_h = self.control_height(f, &style, content_w);
                }
            }
        } else if style.display.generates_box() {
            let content_rect = Rect::new(content_x, content_y, content_w, content_h.max(0.0));
            // A positioned element establishes the containing block (its
            // padding box) for absolutely positioned descendants.
            let positioned = style.position != Position::Static;
            if positioned {
                let padding = style.padding.to_edges(content_w);
                self.cb_stack.push(Rect::new(
                    content_x - padding.left,
                    content_y - padding.top,
                    content_w + padding.horizontal(),
                    content_h + padding.vertical(),
                ));
            }
            let (kids, h) = match kind {
                LayoutKind::Flex => self.layout_flex(node, &style, content_rect, child_cb_height, depth),
                LayoutKind::Grid => self.layout_grid(node, &style, content_rect, child_cb_height, depth),
                LayoutKind::Table => self.layout_table(node, &style, content_rect, depth),
                _ => self.layout_flow(node, &style, content_rect, child_cb_height, depth),
            };
            if positioned {
                self.cb_stack.pop();
            }
            children = kids;
            content_h = explicit_h.unwrap_or(h).max(0.0);
            scroll_size = Size::new(content_w, content_h);
            for c in &children {
                let bottom = c.rect.bottom() + c.margin.bottom - content_y;
                scroll_size.h = scroll_size.h.max(bottom);
            }
        }
        content_h = self.clamp_height(&style, content_h, cb_height);

        let rect = Rect::new(border_x, border_y, content_w + inner_extra, content_h + vertical_extra);
        let content = Rect::new(content_x, content_y, content_w, content_h);
        if !self.doc.rendered_children(node).is_empty() {
            scroll_size.h = scroll_size.h.max(content_h);
        }
        let clip = style.clips_children();
        let mut box_ = LayoutBox {
            node: Some(node),
            pseudo,
            tag,
            style,
            kind,
            rect,
            content,
            border,
            padding,
            margin,
            children,
            clip,
            form,
            image,
            scroll_size,
        };
        // List markers.
        if box_.style.display == Display::ListItem && pseudo.is_none() {
            self.add_marker(&mut box_);
        }
        box_
    }

    /// Text that is actually rendered inside an element, ignoring contents of
    /// non-rendered elements such as `<style>` and `<script>`. Buttons on real
    /// pages often contain a `<style>` before their label; `text_content`
    /// includes it, which made those stylesheets show up as button text.
    fn rendered_text(&self, node: NodeId) -> String {
        fn walk(engine: &Engine<'_>, node: NodeId, out: &mut String) {
            match engine.doc.node(node).map(|n| n.node_type.clone()) {
                Some(NodeType::Text(t)) => out.push_str(&t),
                Some(NodeType::Element(_)) => {
                    let tag = engine.doc.tag_name(node);
                    if matches!(
                        tag,
                        "style" | "script" | "template" | "head" | "title" | "meta" | "link"
                            | "noscript" | "base"
                    ) {
                        return;
                    }
                    for child in engine.doc.rendered_children(node) {
                        walk(engine, child, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = String::new();
        walk(self, node, &mut out);
        out
    }

    fn form_info(&self, node: NodeId, style: &ComputedStyle, tag: &str) -> FormInfo {
        let ty = self.doc.attr(node, "type").unwrap_or("text").to_ascii_lowercase();
        let kind = match tag {
            "textarea" => FormControlKind::Textarea,
            "select" => FormControlKind::Select,
            "button" => FormControlKind::Button,
            _ => match ty.as_str() {
                "password" => FormControlKind::PasswordInput,
                "checkbox" => FormControlKind::Checkbox,
                "radio" => FormControlKind::Radio,
                "submit" => FormControlKind::Submit,
                "button" | "reset" => FormControlKind::Button,
                "range" => FormControlKind::Range,
                "file" => FormControlKind::File,
                "text" | "search" | "email" | "url" | "tel" | "number" | "" => FormControlKind::TextInput,
                _ => FormControlKind::Other,
            },
        };
        let value = match kind {
            FormControlKind::Textarea => self.doc.text_content(node),
            FormControlKind::Select => self
                .doc
                .children(node)
                .iter()
                .filter_map(|c| {
                    if self.doc.tag_name(*c) == "option" {
                        let selected = self.doc.attr(*c, "selected").is_some();
                        Some((selected, self.doc.text_content(*c)))
                    } else {
                        None
                    }
                })
                .find(|(sel, _)| *sel)
                .map(|(_, t)| t.trim().to_string())
                .unwrap_or_default(),
            FormControlKind::Button | FormControlKind::Submit => {
                let attr = self.doc.attr(node, "value").map(|v| v.to_string()).unwrap_or_default();
                if attr.is_empty() {
                    self.rendered_text(node).trim().to_string()
                } else {
                    attr
                }
            }
            _ => self.doc.attr(node, "value").unwrap_or("").to_string(),
        };
        let placeholder = self
            .doc
            .attr(node, "placeholder")
            .map(|p| p.to_string())
            .unwrap_or_default();
        let options = self
            .doc
            .children(node)
            .iter()
            .filter(|c| self.doc.tag_name(**c) == "option")
            .map(|c| self.doc.text_content(*c).trim().to_string())
            .collect();
        let _ = style;
        FormInfo {
            kind,
            value,
            checked: self.doc.attr(node, "checked").is_some() || self.doc.attr(node, "selected").is_some(),
            placeholder,
            disabled: self.doc.attr(node, "disabled").is_some(),
            options,
        }
    }

    fn control_height(&self, form: &FormInfo, style: &ComputedStyle, width: f32) -> f32 {
        match form.kind {
            FormControlKind::Checkbox | FormControlKind::Radio => 13.0,
            FormControlKind::Textarea => 51.0,
            FormControlKind::Button | FormControlKind::Submit => {
                let edges = style.padding.to_edges(0.0).vertical()
                    + style.border_edges_width().vertical();
                (style.line_height + edges + 4.0).max(21.0)
            }
            FormControlKind::File => 21.0,
            _ => 21.0,
        }
        .max(style.line_height + 2.0)
        .min(width.max(0.0).max(0.0) + 1000.0)
    }

    fn replaced_size(&self, node: NodeId, style: &ComputedStyle, tag: &str, content_w: f32) -> (f32, f32) {
        let mut w = style.width.resolve(content_w);
        let mut h = style.height.resolve(content_w);
        let attr_w = self.doc.attr(node, "width").and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok());
        let attr_h = self.doc.attr(node, "height").and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok());
        let intrinsic = if tag == "img" {
            self.doc
                .attr(node, "src")
                .map(|src| self.images.image_size(&self.doc.effective_base().resolve(src)))
                .flatten()
        } else if tag == "iframe" {
            Some(Size::new(300.0, 150.0))
        } else if tag == "canvas" {
            let cw = self.doc.attr(node, "width").and_then(|v| v.parse::<f32>().ok()).unwrap_or(300.0);
            let ch = self.doc.attr(node, "height").and_then(|v| v.parse::<f32>().ok()).unwrap_or(150.0);
            Some(Size::new(cw, ch))
        } else {
            None
        };
        let (iw, ih) = intrinsic.map(|s| (s.w, s.h)).unwrap_or((300.0, 150.0));
        if w.is_none() {
            w = Some(attr_w.unwrap_or(iw));
        }
        if h.is_none() {
            h = Some(attr_h.unwrap_or(ih));
        }
        let mut fw = w.unwrap_or(iw).max(0.0);
        let mut fh = h.unwrap_or(ih).max(0.0);
        if attr_w.is_some() && style.height.resolve(content_w).is_none() && attr_h.is_none() && ih > 0.0 {
            fh = fw * ih / iw.max(1.0);
        }
        if fh <= 0.0 && ih > 0.0 {
            fh = ih;
        }
        if fw <= 0.0 {
            fw = iw;
        }
        (fw, fh)
    }

    fn add_marker(&mut self, box_: &mut LayoutBox) {
        let style = box_.style.clone();
        if style.list_style_type == ListStyleType::None {
            return;
        }
        let index = self
            .doc
            .parent(box_.node.unwrap())
            .map(|p| {
                let siblings = self.doc.element_children(p);
                siblings
                    .iter()
                    .filter(|s| matches!(self.style_of(**s).display, Display::ListItem))
                    .position(|s| Some(*s) == box_.node)
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let text = style.list_style_type.marker(index);
        if text.is_empty() {
            return;
        }
        let font = FontSpec::from_style(&style);
        let width = self.fonts.measure(&text, &font);
        let x = if style.list_style_position == ListStylePosition::Inside {
            box_.content.x
        } else {
            box_.content.x - width - 7.0
        };
        let baseline = box_.content.y + self.fonts.metrics(&font).ascent.max(style.line_height * 0.8);
        if self.measuring == 0 {
            self.texts.push(TextFragment {
                text,
                rect: Rect::new(x, box_.content.y, width, style.line_height),
                baseline,
                font,
                color: style.color,
                decoration: TextDecoration::None,
                decoration_color: None,
                node: box_.node,
                owner: box_.node,
                opacity: style.opacity,
                is_marker: true,
            });
        }
        if style.list_style_position == ListStylePosition::Inside {
            box_.content.y += style.line_height;
        }
    }

    fn layout_flow(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        content: Rect,
        cb_height: Option<f32>,
        depth: u32,
    ) -> (Vec<LayoutBox>, f32) {
        let mut children = Vec::new();
        let mut inline_items: Vec<NodeId> = Vec::new();
        let mut cursor = content.y;
        let mut prev_margin = 0.0f32;
        let mut float_bottom = content.y;
        let mut floats: Vec<(Rect, Float)> = Vec::new();
        let mut saw_block = false;

        let kids: Vec<NodeId> = self.doc.rendered_children(node);
        for child in kids {
            match self.doc.node(child).map(|n| n.node_type.clone()) {
                Some(NodeType::Text(_)) => inline_items.push(child),
                Some(NodeType::Element(_)) => {
                    if matches!(
                        self.doc.tag_name(child),
                        "style" | "script" | "template" | "head" | "title" | "meta" | "link"
                            | "noscript" | "base"
                    ) {
                        continue;
                    }
                    let cs = self.style_of(child);
                    if cs.display == Display::None {
                        continue;
                    }
                    if cs.position.is_out_of_flow() {
                        self.flush_inline(
                            &mut inline_items,
                            &mut children,
                            content,
                            style,
                            node,
                            &mut cursor,
                            depth,
                        );
                        let cb = self.containing_block();
                        let b = self.layout_absolute(child, &cs, cb, &mut cursor, content, depth);
                        children.push(b);
                        continue;
                    }
                    if cs.display.is_inline_level() && cs.float == Float::None {
                        inline_items.push(child);
                        continue;
                    }
                    self.flush_inline(
                        &mut inline_items,
                        &mut children,
                        content,
                        style,
                        node,
                        &mut cursor,
                        depth,
                    );
                    saw_block = true;
                    let m = self.resolve_margins(&cs, content.w);
                    if cs.clear != Clear::None {
                        cursor = cursor.max(float_bottom);
                        prev_margin = 0.0;
                    }
                    if cs.float != Float::None {
                        let mut w = self.shrink_to_fit(child, &cs, content.w);
                        w = w.min(content.w).max(0.0);
                        let fx = if cs.float == Float::Right {
                            content.right() - w - m.left - m.right
                        } else {
                            content.x
                        };
                        let b = self.layout_element(
                            child, None, fx, cursor, content.w, cb_height, Some(w), None,
                            kind_for(&cs, self.doc.tag_name(child)), depth + 1,
                        );
                        float_bottom = float_bottom.max(b.rect.bottom());
                        floats.push((b.rect, cs.float));
                        children.push(b);
                        continue;
                    }
                    if cs.position == Position::Relative {
                        let text_start = self.texts.len();
                        let image_start = self.images_out.len();
                        let gap = margin_gap(prev_margin, m.top, saw_block);
                        let b_y = cursor + gap - m.top;
                        let mut b = self.layout_element(
                            child, None, content.x, b_y, content.w, cb_height, None, None,
                            kind_for(&cs, self.doc.tag_name(child)), depth + 1,
                        );
                        let dx = cs.left.resolve(content.w).unwrap_or(0.0)
                            - cs.right.resolve(content.w).unwrap_or(0.0);
                        let dy = cs.top.resolve(0.0).unwrap_or(0.0) - cs.bottom.resolve(0.0).unwrap_or(0.0);
                        if dx != 0.0 || dy != 0.0 {
                            self.translate_subtree(&mut b, dx, dy, text_start, image_start);
                        }
                        cursor = b.rect.bottom() + m.bottom;
                        prev_margin = m.bottom;
                        float_bottom = float_bottom.max(cursor);
                        children.push(b);
                        continue;
                    }
                    let gap = margin_gap(prev_margin, m.top, saw_block);
                    // Floats reserve inline space inside following blocks; the
                    // inline layout narrows individual lines that overlap a
                    // float and returns to full width below it (MediaWiki
                    // thumbnails float beside article text).
                    let mut active = self.active_floats.clone();
                    active.extend(floats.iter().cloned());
                    let saved_floats = std::mem::replace(&mut self.active_floats, active);
                    let b = self.layout_element(
                        child, None, content.x, cursor + gap - m.top, content.w, cb_height, None, None,
                        kind_for(&cs, self.doc.tag_name(child)), depth + 1,
                    );
                    self.active_floats = saved_floats;
                    // Margins collapse through an empty block: its top and bottom
                    // margins become one, so it must not add both. MediaWiki's
                    // empty `#siteNotice` otherwise pushes the page down 24px.
                    if b.rect.h <= 0.5 {
                        cursor = b.rect.y;
                        prev_margin = 0.0;
                    } else {
                        cursor = b.rect.bottom() + m.bottom;
                        prev_margin = m.bottom;
                    }
                    float_bottom = float_bottom.max(cursor);
                    children.push(b);
                }
                _ => {}
            }
        }
        let box_count = children.len();
        let text_count = self.texts.len();
        self.flush_inline(
            &mut inline_items,
            &mut children,
            content,
            style,
            node,
            &mut cursor,
            depth,
        );
        // Whitespace-only text nodes are collected as inline items but do not
        // separate the last block's margin from the parent's edge.
        let trailing_inline = children.len() > box_count || self.texts.len() > text_count;
        let mut height = (cursor.max(float_bottom) - content.y).max(0.0);
        // The last child's bottom margin collapses out of an auto-height
        // block that has no bottom padding or border. Without this an empty
        // `#siteNotice` wrapper still reserves its bottom margin and shifts
        // the whole page down.
        let bottom_edge =
            style.padding.to_edges(0.0).bottom + style.border_edges_width().bottom;
        if saw_block
            && !trailing_inline
            && matches!(style.height, Length::Auto)
            && bottom_edge <= 0.0
            && float_bottom <= cursor + 0.01
        {
            height = (height - prev_margin).max(0.0);
        }
        (children, height)
    }

    fn flush_inline(
        &mut self,
        items: &mut Vec<NodeId>,
        children: &mut Vec<LayoutBox>,
        content: Rect,
        parent_style: &ComputedStyle,
        parent_node: NodeId,
        cursor: &mut f32,
        depth: u32,
    ) {
        if items.is_empty() {
            return;
        }
        let group = std::mem::take(items);
        let (boxes, h) = self.layout_inline_group(&group, content, parent_style, parent_node, *cursor, depth);
        children.extend(boxes);
        *cursor += h;
    }

    fn containing_block(&self) -> Rect {
        self.cb_stack
            .last()
            .copied()
            .unwrap_or_else(|| Rect::new(0.0, 0.0, self.viewport.w, self.viewport.h))
    }

    fn layout_absolute(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        cb: Rect,
        cursor: &mut f32,
        content: Rect,
        depth: u32,
    ) -> LayoutBox {
        let cb = if style.position == Position::Fixed {
            Rect::new(0.0, 0.0, self.viewport.w, self.viewport.h)
        } else {
            cb
        };
        let m = self.resolve_margins(style, cb.w);
        let left = style.left.resolve(cb.w);
        let right = style.right.resolve(cb.w);
        let top = style.top.resolve(cb.h);
        let bottom = style.bottom.resolve(cb.h);
        let width = style.width.resolve(cb.w);
        let height = style.height.resolve(cb.h);
        let shrink = || 0.0f32;
        let _ = shrink;

        let (mx, forced_w) = match (left, right, width) {
            (Some(l), Some(r), None) => {
                (cb.x + l, Some((cb.w - l - r).max(0.0)))
            }
            (Some(l), _, Some(w)) => (cb.x + l, Some(w)),
            (None, Some(r), Some(w)) => (cb.right() - r - w - m.left - m.right, Some(w)),
            (None, Some(r), None) => {
                let w = self.shrink_to_fit(node, style, cb.w);
                (cb.right() - r - w - m.left - m.right, Some(w))
            }
            _ => (cb.x + content.x.max(0.0), width),
        };
        let (my, forced_h) = match (top, bottom, height) {
            (Some(t), Some(b), None) => (cb.y + t, Some((cb.h - t - b).max(0.0))),
            (Some(t), _, Some(h)) => (cb.y + t, Some(h)),
            (None, Some(b), Some(h)) => (cb.bottom() - b - h - m.top - m.bottom, Some(h)),
            _ => {
                let y = if top.is_none() && bottom.is_none() { *cursor } else { cb.y };
                (y, height)
            }
        };
        self.cb_stack.push(Rect::new(cb.x, cb.y, cb.w, cb.h));
        let mut b = self.layout_element(
            node, None, mx, my, cb.w, Some(cb.h), forced_w, forced_h,
            kind_for(style, self.doc.tag_name(node)), depth + 1,
        );
        self.cb_stack.pop();
        // With an auto height the box had to be laid out before its height was
        // known, so apply a bottom-only offset afterwards.
        if top.is_none() && height.is_none() {
            if let Some(bottom) = bottom {
                let target = cb.bottom() - bottom - b.rect.h - m.bottom;
                let dy = target - b.rect.y;
                if dy.abs() > 0.01 {
                    let text_start = self.texts.len();
                    let image_start = self.images_out.len();
                    self.translate_subtree(&mut b, 0.0, dy, text_start, image_start);
                }
            }
        }
        b
    }

    fn translate_subtree(&mut self, b: &mut LayoutBox, dx: f32, dy: f32, _text_start: usize, _image_start: usize) {
        translate_box(b, dx, dy);
        // Grid and flex placement can move a box after its inline content has
        // already produced fragments, so match fragments by node ownership
        // instead of relying on a positional watermark.
        let mut nodes = Vec::new();
        collect_box_nodes(b, &mut nodes);
        for t in self.texts.iter_mut() {
            let inside = t.node.is_some_and(|n| nodes.contains(&n))
                || t.owner.is_some_and(|n| nodes.contains(&n));
            if inside {
                t.rect.x += dx;
                t.rect.y += dy;
                t.baseline += dy;
            }
        }
        for i in self.images_out.iter_mut() {
            if nodes.contains(&i.node) {
                i.rect.x += dx;
                i.rect.y += dy;
            }
        }
    }

    // ---- inline layout ---------------------------------------------------

    fn layout_inline_group(
        &mut self,
        items: &[NodeId],
        content: Rect,
        parent_style: &ComputedStyle,
        parent_node: NodeId,
        start_y: f32,
        depth: u32,
    ) -> (Vec<LayoutBox>, f32) {
        let mut tokens: Vec<InlineToken> = Vec::new();
        if let Some(before) = self.styles.pseudo_style_rc(parent_node, PseudoElement::Before) {
            if before.content.is_some() {
                self.push_pseudo_token(&before, &mut tokens);
            }
        }
        let mut ancestors = Vec::new();
        let context_style = Rc::new(parent_style.clone());
        for item in items {
            self.collect_inline(*item, &mut ancestors, &context_style, &mut tokens);
        }
        if let Some(after) = self.styles.pseudo_style_rc(parent_node, PseudoElement::After) {
            if after.content.is_some() {
                self.push_pseudo_token(&after, &mut tokens);
            }
        }
        if tokens.is_empty() {
            return (Vec::new(), 0.0);
        }

        let line_height = parent_style.line_height.max(parent_style.font_size);
        let mut lines: Vec<Vec<PlacedToken>> = Vec::new();
        let mut current: Vec<PlacedToken> = Vec::new();
        let mut x = 0.0f32;
        let mut pending_space: Option<f32> = None;
        let max_w = content.w.max(0.0);
        let mut line_insets: Vec<(f32, f32)> = Vec::new();
        let mut line_top = start_y;
        let mut cur_inset = float_insets(&self.active_floats, line_top, line_height, content);

        let flush = |current: &mut Vec<PlacedToken>, lines: &mut Vec<Vec<PlacedToken>>, x: &mut f32| {
            if !current.is_empty() {
                lines.push(std::mem::take(current));
                *x = 0.0;
            }
        };

        for token in tokens {
            match &token.kind {
                TokenKind::Newline => {
                    flush(&mut current, &mut lines, &mut x);
                    line_top += line_height;
                    cur_inset = float_insets(&self.active_floats, line_top, line_height, content);
                    pending_space = None;
                }
                TokenKind::Space { width } => {
                    if !current.is_empty() {
                        pending_space = Some(*width);
                    }
                }
                TokenKind::Text { .. } | TokenKind::Atomic { .. } => {
                    let (mut tw, th) = match &token.kind {
                        TokenKind::Text { .. } => (token.width, token.line_height.max(line_height)),
                        TokenKind::Atomic { .. } => (token.width, token.height),
                        _ => unreachable!(),
                    };
                    let space = pending_space.take().unwrap_or(0.0);
                    let mut avail = (max_w - cur_inset.0 - cur_inset.1).max(0.0);
                    if x + space + tw > avail && !current.is_empty() {
                        flush(&mut current, &mut lines, &mut x);
                        line_top += line_height;
                        cur_inset = float_insets(&self.active_floats, line_top, line_height, content);
                        avail = (max_w - cur_inset.0 - cur_inset.1).max(0.0);
                    }
                    let mut extra_space = space;
                    if x + space + tw > avail && current.is_empty() && tw > avail {
                        // Break very long words into chunks when allowed.
                        let breakable = token.style.overflow_wrap != OverflowWrap::Normal
                            || token.style.word_break == WordBreak::BreakAll
                            || token.style.word_break == WordBreak::BreakWord;
                        if breakable {
                            let mut chunk = String::new();
                            let mut cw = 0.0;
                            for ch in token.text.chars() {
                                let cwidth = self.fonts.measure(&ch.to_string(), &token.font);
                                if x + cw + cwidth > avail && !chunk.is_empty() {
                                    let mut t = token.clone();
                                    if let TokenKind::Text { text } = &mut t.kind {
                                        *text = std::mem::take(&mut chunk);
                                    }
                                    t.width = cw;
                                    if current.is_empty() {
                                        line_insets.push(cur_inset);
                                    }
                                    current.push(PlacedToken { token: t, x, w: cw, space_before: extra_space });
                                    x += cw;
                                    flush(&mut current, &mut lines, &mut x);
                                    line_top += line_height;
                                    cur_inset = float_insets(&self.active_floats, line_top, line_height, content);
                                    avail = (max_w - cur_inset.0 - cur_inset.1).max(0.0);
                                    cw = 0.0;
                                    extra_space = 0.0;
                                }
                                chunk.push(ch);
                                cw += cwidth;
                            }
                            if !chunk.is_empty() {
                                tw = cw;
                                let mut t = token.clone();
                                if let TokenKind::Text { text } = &mut t.kind {
                                    *text = chunk;
                                }
                                t.width = cw;
                                if current.is_empty() {
                                    line_insets.push(cur_inset);
                                }
                                current.push(PlacedToken { token: t, x: x + extra_space, w: tw, space_before: extra_space });
                                x += extra_space + tw;
                            }
                            continue;
                        }
                    }
                    let tx = x + extra_space;
                    if current.is_empty() {
                        line_insets.push(cur_inset);
                    }
                    current.push(PlacedToken { token, x: tx, w: tw, space_before: extra_space });
                    x = tx + tw;
                    let _ = th;
                }
            }
        }
        flush(&mut current, &mut lines, &mut x);

        let mut y = start_y;
        let mut boxes: Vec<LayoutBox> = Vec::new();
        let mut unions: HashMap<NodeId, Rect> = HashMap::new();
        let mut total_h = 0.0f32;

        for (line_index, line) in lines.iter().enumerate() {
            let (inset_left, inset_right) = line_insets
                .get(line_index)
                .copied()
                .unwrap_or((0.0, 0.0));
            let mut max_ascent = 0.0f32;
            let mut max_descent = 0.0f32;
            let mut line_h = line_height;
            for placed in line {
                let (a, d) = match &placed.token.kind {
                    TokenKind::Atomic { .. } => (placed.token.height, 0.0),
                    _ => {
                        let m = self.fonts.metrics(&placed.token.font);
                        (m.ascent, m.descent)
                    }
                };
                max_ascent = max_ascent.max(a);
                max_descent = max_descent.max(d);
                line_h = line_h.max(placed.token.line_height).max(placed.token.height);
            }
            if line.is_empty() {
                y += line_h;
                total_h += line_h;
                continue;
            }
            let line_w = line.last().map(|p| p.x + p.w).unwrap_or(0.0);
            let free = (max_w - inset_left - inset_right - line_w).max(0.0);
            let align = parent_style.text_align.resolve(parent_style.direction == Direction::Rtl);
            let is_last = std::ptr::eq(line, lines.last().unwrap());
            let mut shift = match align {
                TextAlign::Center => free / 2.0,
                TextAlign::Right => free,
                _ => 0.0,
            };
            let justify = align == TextAlign::Justify && !is_last;
            let spaces = line.iter().filter(|p| p.token.kind.is_text()).count();
            let extra_per_space = if justify && spaces > 1 {
                free / (spaces as f32 - 1.0)
            } else {
                0.0
            };
            let baseline = y + max_ascent + (line_h - max_ascent - max_descent).max(0.0) / 2.0;

            for placed in line {
                let mut px = content.x + inset_left + placed.x + shift;
                if extra_per_space > 0.0 {
                    px += placed.space_before / placed.token.font.size.max(1.0) * 0.0;
                    // Apply justification by pushing following words.
                }
                let rect = Rect::new(px, y, placed.w, line_h);
                match &placed.token.kind {
                    TokenKind::Text { text } => {
                        if self.measuring == 0 {
                            self.texts.push(TextFragment {
                                text: text.clone(),
                                rect,
                                baseline,
                                font: placed.token.font.clone(),
                                color: placed.token.style.color,
                                decoration: placed.token.style.text_decoration,
                                decoration_color: placed.token.style.text_decoration_color,
                                node: placed.token.node,
                                owner: placed
                                    .token
                                    .ancestors
                                    .last()
                                    .copied()
                                    .or(Some(parent_node)),
                                opacity: placed.token.style.opacity,
                                is_marker: false,
                            });
                        }
                        for a in &placed.token.ancestors {
                            unions
                                .entry(*a)
                                .and_modify(|r| *r = r.union(&rect))
                                .or_insert(rect);
                        }
                        if extra_per_space > 0.0 {
                            shift += extra_per_space;
                        }
                    }
                    TokenKind::Atomic { node } => {
                        let node = *node;
                        let st = self.style_of(node);
                        let ay = baseline - placed.token.height;
                        let start_texts = self.texts.len();
                        let start_images = self.images_out.len();
                        let mut b = self.layout_element(
                            node,
                            None,
                            content.x + placed.x + shift,
                            ay,
                            max_w,
                            None,
                            Some(placed.w),
                            None,
                            kind_for(&st, self.doc.tag_name(node)),
                            depth + 1,
                        );
                        let dy = baseline - placed.token.height - b.rect.y;
                        if dy.abs() > 0.01 {
                            self.translate_subtree(&mut b, 0.0, dy, start_texts, start_images);
                        }
                        for a in &placed.token.ancestors {
                            unions
                                .entry(*a)
                                .and_modify(|r| *r = r.union(&b.rect))
                                .or_insert(b.rect);
                        }
                        boxes.push(b);
                        if extra_per_space > 0.0 {
                            shift += extra_per_space;
                        }
                    }
                    _ => {}
                }
                let _ = px;
                px = content.x;
            }
            y += line_h;
            total_h += line_h;
        }

        for (node, rect) in unions {
            let st = self.style_of(node);
            boxes.push(LayoutBox {
                node: Some(node),
                pseudo: None,
                tag: self.doc.tag_name(node).to_string(),
                style: st.clone(),
                kind: LayoutKind::Inline,
                rect,
                content: rect,
                border: Edges::ZERO,
                padding: Edges::ZERO,
                margin: Edges::ZERO,
                children: Vec::new(),
                clip: false,
                form: None,
                image: None,
                scroll_size: rect.size(),
            });
        }
        (boxes, total_h.max(0.0))
    }

    fn push_pseudo_token(&mut self, style: &ComputedStyle, out: &mut Vec<InlineToken>) {
        if let Some(text) = &style.content {
            if text.is_empty() {
                return;
            }
            let font = FontSpec::from_style(style);
            let width = self.fonts.measure(text, &font);
            out.push(InlineToken {
                kind: TokenKind::Text { text: text.clone() },
                text: text.clone(),
                width,
                font,
                line_height: style.line_height,
                height: style.line_height,
                style: self.style_rc_from(style),
                node: None,
                ancestors: Vec::new(),
            });
        }
    }

    fn style_rc_from(&self, style: &ComputedStyle) -> Rc<ComputedStyle> {
        Rc::new(style.clone())
    }

    fn collect_inline(
        &mut self,
        node: NodeId,
        ancestors: &mut Vec<NodeId>,
        context_style: &Rc<ComputedStyle>,
        out: &mut Vec<InlineToken>,
    ) {
        match self.doc.node(node).map(|n| n.node_type.clone()) {
            Some(NodeType::Text(text)) => {
                self.tokenize_text(&text, context_style, Some(node), ancestors, out);
            }
            Some(NodeType::Element(_)) => {
                let style = self.style_of(node);
                if style.display == Display::None {
                    return;
                }
                let tag = self.doc.tag_name(node).to_string();
                if let Some(before) = self.styles.pseudo_style_rc(node, PseudoElement::Before) {
                    if before.content.is_some() {
                        self.push_pseudo_token(&before, out);
                    }
                }
                if tag == "br" {
                    out.push(InlineToken {
                        kind: TokenKind::Newline,
                        text: String::new(),
                        width: 0.0,
                        font: FontSpec::from_style(&style),
                        line_height: style.line_height,
                        height: 0.0,
                        style: style.clone(),
                        node: Some(node),
                        ancestors: ancestors.clone(),
                    });
                } else if matches!(tag.as_str(), "img" | "video" | "canvas" | "iframe" | "input" | "select" | "textarea" | "button")
                    || style.display == Display::InlineBlock
                    || style.display == Display::InlineFlex
                    || style.display == Display::InlineGrid
                {
                    let (w, h) = self.measure_atomic(node, &style);
                    out.push(InlineToken {
                        kind: TokenKind::Atomic { node },
                        text: String::new(),
                        width: w,
                        font: FontSpec::from_style(&style),
                        line_height: style.line_height,
                        height: h,
                        style: style.clone(),
                        node: Some(node),
                        ancestors: ancestors.clone(),
                    });
                } else if style.display.is_inline_level() {
                    ancestors.push(node);
                    let child_style = Rc::new((*style).clone());
                    for child in self.doc.rendered_children(node) {
                        self.collect_inline(child, ancestors, &child_style, out);
                    }
                    ancestors.pop();
                } else {
                    let (w, h) = self.measure_atomic(node, &style);
                    out.push(InlineToken {
                        kind: TokenKind::Atomic { node },
                        text: String::new(),
                        width: w,
                        font: FontSpec::from_style(&style),
                        line_height: style.line_height,
                        height: h,
                        style: style.clone(),
                        node: Some(node),
                        ancestors: ancestors.clone(),
                    });
                }
                if let Some(after) = self.styles.pseudo_style_rc(node, PseudoElement::After) {
                    if after.content.is_some() {
                        self.push_pseudo_token(&after, out);
                    }
                }
            }
            _ => {}
        }
    }

    fn tokenize_text(
        &mut self,
        text: &str,
        style: &Rc<ComputedStyle>,
        node: Option<NodeId>,
        ancestors: &[NodeId],
        out: &mut Vec<InlineToken>,
    ) {
        let transformed = apply_text_transform(text, style.text_transform);
        let font = FontSpec::from_style(style);
        let ws = style.white_space;
        let mut push_text = |s: &str, out: &mut Vec<InlineToken>| {
            if s.is_empty() {
                return;
            }
            let w = self.fonts.measure(s, &font);
            out.push(InlineToken {
                kind: TokenKind::Text { text: s.to_string() },
                text: s.to_string(),
                width: w,
                font: font.clone(),
                line_height: style.line_height,
                height: style.line_height,
                style: style.clone(),
                node,
                ancestors: ancestors.to_vec(),
            });
        };
        let push_space = |out: &mut Vec<InlineToken>| {
            let w = self.fonts.measure(" ", &font);
            out.push(InlineToken {
                kind: TokenKind::Space { width: w },
                text: " ".into(),
                width: w,
                font: font.clone(),
                line_height: style.line_height,
                height: style.line_height,
                style: style.clone(),
                node,
                ancestors: ancestors.to_vec(),
            });
        };
        let whitespace = '\u{20}' as u32;

        if ws == WhiteSpace::Pre || ws == WhiteSpace::PreWrap || ws == WhiteSpace::BreakSpaces {
            let mut buf = String::new();
            for ch in transformed.chars() {
                if ch == '\n' {
                    push_text(&buf, out);
                    buf.clear();
                    out.push(InlineToken {
                        kind: TokenKind::Newline,
                        text: String::new(),
                        width: 0.0,
                        font: font.clone(),
                        line_height: style.line_height,
                        height: 0.0,
                        style: style.clone(),
                        node,
                        ancestors: ancestors.to_vec(),
                    });
                } else if ch == '\t' {
                    buf.push_str("    ");
                } else {
                    buf.push(ch);
                }
            }
            push_text(&buf, out);
            return;
        }

        let mut word = String::new();
        // Start as if the previous character was not whitespace so a leading
        // space in this text node still separates it from the previous inline
        // node. The line layout ignores a pending space at the start of a line.
        let mut prev_space = false;
        let mut chars = transformed.chars().peekable();
        while let Some(ch) = chars.next() {
            let is_ws = ch as u32 == whitespace || ch == '\t' || ch == '\r';
            if ch == '\n' {
                if ws.preserves_newlines() {
                    push_text(&word, out);
                    word.clear();
                    out.push(InlineToken {
                        kind: TokenKind::Newline,
                        text: String::new(),
                        width: 0.0,
                        font: font.clone(),
                        line_height: style.line_height,
                        height: 0.0,
                        style: style.clone(),
                        node,
                        ancestors: ancestors.to_vec(),
                    });
                    prev_space = true;
                } else if !prev_space {
                    push_text(&word, out);
                    word.clear();
                    push_space(out);
                    prev_space = true;
                }
                continue;
            }
            if is_ws {
                if !prev_space {
                    push_text(&word, out);
                    word.clear();
                    push_space(out);
                    prev_space = true;
                }
            } else {
                word.push(ch);
                prev_space = false;
            }
        }
        push_text(&word, out);
    }

    fn measure_atomic(&mut self, node: NodeId, style: &ComputedStyle) -> (f32, f32) {
        let tag = self.doc.tag_name(node).to_string();
        if tag == "img" || tag == "video" || tag == "canvas" || tag == "iframe" {
            let w = self.viewport.w;
            let (iw, ih) = self.replaced_size(node, style, &tag, w);
            return (iw, ih);
        }
        if is_form_control(&tag) {
            let form = self.form_info(node, style, &tag);
            let width = style.width.resolve(200.0).unwrap_or_else(|| {
                if matches!(form.kind, FormControlKind::Button | FormControlKind::Submit) {
                    let label = if form.value.is_empty() { "Submit".to_string() } else { form.value.clone() };
                    let text = self.fonts.measure(&label, &FontSpec::from_style(style));
                    let edges = style.padding.to_edges(0.0).horizontal()
                        + style.border_edges_width().horizontal();
                    text + edges + 4.0
                } else if matches!(form.kind, FormControlKind::Checkbox | FormControlKind::Radio) {
                    13.0
                } else {
                    170.0
                }
            });
            let height = style.height.resolve(100.0).unwrap_or_else(|| self.control_height(&form, style, width));
            return (width, height);
        }
        // inline-block: measure by laying out with suppressed side effects.
        let avail = style
            .width
            .resolve(self.containing_block().w)
            .unwrap_or_else(|| self.shrink_to_fit(node, style, self.containing_block().w));
        self.measuring += 1;
        let b = self.layout_element(
            node,
            None,
            0.0,
            0.0,
            self.containing_block().w,
            None,
            Some(avail),
            None,
            kind_for(style, &tag),
            0,
        );
        self.measuring -= 1;
        (b.rect.w.max(avail).min(self.containing_block().w.max(avail)), b.rect.h)
    }

    fn shrink_to_fit(&mut self, node: NodeId, style: &ComputedStyle, available: f32) -> f32 {
        let maxc = self.max_content_width(node, style, 0);
        let minc = self.min_content_width(node, style, 0);
        maxc.min(available.max(minc)).max(minc.min(available))
    }

    fn max_content_width(&mut self, node: NodeId, style: &ComputedStyle, depth: u32) -> f32 {
        if depth > 30 {
            return 0.0;
        }
        let extra = style.padding.to_edges(0.0).horizontal() + style.border_edges_width().horizontal();
        // Percentages are indefinite during intrinsic sizing; resolving them
        // against a zero basis made `width:100%` children measure as zero and
        // collapsed their flex/grid ancestors (Discord's hero column).
        let definite = match style.width {
            Length::Px(w) => Some(w),
            Length::Calc { px, pct } if pct == 0.0 => Some(px),
            _ => None,
        };
        if let Some(w) = definite {
            return if style.box_sizing == BoxSizing::BorderBox {
                w
            } else {
                w + extra
            };
        }
        // A `display: table` wrapper around a replaced element (MediaWiki
        // thumbnails are `<figure>` tables) is as wide as its image; captions
        // wrap inside that width instead of widening the table.
        if style.display == Display::Table {
            let img_w = self.max_replaced_width(node, 0);
            if img_w > 0.0 {
                return img_w + extra;
            }
        }
        let tag = self.doc.tag_name(node);
        if tag == "img" {
            let (w, _) = self.replaced_size(node, style, tag, 1000.0);
            return w + extra;
        }
        if is_form_control(tag) {
            let (w, _) = self.measure_atomic(node, style);
            return w;
        }
        let font = FontSpec::from_style(style);
        let mut total = 0.0f32;
        let mut max = 0.0f32;
        let mut has_text = false;
        for child in self.doc.rendered_children(node) {
            match self.doc.node(child).map(|n| n.node_type.clone()) {
                Some(NodeType::Text(t)) => {
                    has_text = true;
                    let text = t.split_whitespace().collect::<Vec<_>>().join(" ");
                    total += self.fonts.measure(&text, &font);
                }
                Some(NodeType::Element(_)) => {
                    let cs = self.style_of(child);
                    if cs.display == Display::None || cs.position.is_out_of_flow() {
                        continue;
                    }
                    let w = self.max_content_width(child, &cs, depth + 1);
                    if cs.display.is_inline_level() {
                        total += w;
                    } else {
                        max = max.max(w);
                    }
                }
                _ => {}
            }
        }
        // Inline *elements* contribute to `total` just like direct text; the
        // old check only looked at direct text nodes, so a block/flex item
        // whose whole content was an inline-block child measured as zero and
        // collapsed to its padding (Google's Sign in button, footer columns).
        let inner = if total > 0.0 || has_text {
            total.max(max)
        } else {
            max
        };
        inner + extra
    }

    /// Widest rendered replaced element inside `node` (0 when none).
    fn max_replaced_width(&mut self, node: NodeId, depth: u32) -> f32 {
        if depth > 30 {
            return 0.0;
        }
        let tag = self.doc.tag_name(node).to_string();
        let style = self.style_of(node);
        if tag == "img" || tag == "video" || tag == "canvas" {
            let (w, _) = self.replaced_size(node, &style, &tag, 1000.0);
            return w + style.padding.to_edges(0.0).horizontal()
                + style.border_edges_width().horizontal();
        }
        let mut best = 0.0f32;
        for child in self.doc.rendered_children(node) {
            if !self.doc.is_element(child) {
                continue;
            }
            let cs = self.style_of(child);
            if cs.display == Display::None || cs.position.is_out_of_flow() {
                continue;
            }
            best = best.max(self.max_replaced_width(child, depth + 1));
        }
        best
    }

    fn min_content_width(&mut self, node: NodeId, style: &ComputedStyle, depth: u32) -> f32 {
        if depth > 30 {
            return 0.0;
        }
        let extra = style.padding.to_edges(0.0).horizontal() + style.border_edges_width().horizontal();
        if let Some(w) = style.width.resolve(0.0) {
            return w + if style.box_sizing == BoxSizing::BorderBox { 0.0 } else { extra };
        }
        let tag = self.doc.tag_name(node);
        if tag == "img" {
            let (w, _) = self.replaced_size(node, style, tag, 1000.0);
            return w + extra;
        }
        let font = FontSpec::from_style(style);
        let mut widest = 0.0f32;
        for child in self.doc.rendered_children(node) {
            match self.doc.node(child).map(|n| n.node_type.clone()) {
                Some(NodeType::Text(t)) => {
                    for word in t.split_whitespace() {
                        widest = widest.max(self.fonts.measure(word, &font));
                    }
                }
                Some(NodeType::Element(_)) => {
                    let cs = self.style_of(child);
                    if cs.display == Display::None || cs.position.is_out_of_flow() {
                        continue;
                    }
                    widest = widest.max(self.min_content_width(child, &cs, depth + 1));
                }
                _ => {}
            }
        }
        widest + extra
    }

    // ---- flexbox ---------------------------------------------------------

    fn flex_text(&self, node: NodeId) -> String {
        match self.doc.node(node).map(|n| n.node_type.clone()) {
            Some(NodeType::Text(t)) => t.split_whitespace().collect::<Vec<_>>().join(" "),
            _ => String::new(),
        }
    }

    /// Lay out a bare text run that is a direct child of a flex container as a
    /// single anonymous flex item.
    fn layout_flex_text(
        &mut self,
        node: NodeId,
        parent_style: &ComputedStyle,
        x: f32,
        y: f32,
        forced_w: Option<f32>,
        forced_h: Option<f32>,
    ) -> LayoutBox {
        let font = FontSpec::from_style(parent_style);
        let text = self.flex_text(node);
        let measured = self.fonts.measure(&text, &font);
        let w = forced_w.unwrap_or(measured).max(0.0);
        let h = forced_h.unwrap_or(parent_style.line_height).max(0.0);
        let rect = Rect::new(x, y, w, h);
        let baseline = y + self.fonts.metrics(&font).ascent.max(h * 0.8);
        if self.measuring == 0 && !text.is_empty() {
            self.texts.push(TextFragment {
                text,
                rect,
                baseline,
                font,
                color: parent_style.color,
                decoration: parent_style.text_decoration,
                decoration_color: parent_style.text_decoration_color,
                node: Some(node),
                owner: Some(node),
                opacity: parent_style.opacity,
                is_marker: false,
            });
        }
        LayoutBox {
            node: Some(node),
            pseudo: None,
            tag: "#text".to_string(),
            style: self.style_rc_from(parent_style),
            kind: LayoutKind::Inline,
            rect,
            content: rect,
            border: Edges::ZERO,
            padding: Edges::ZERO,
            margin: Edges::ZERO,
            children: Vec::new(),
            clip: false,
            form: None,
            image: None,
            scroll_size: rect.size(),
        }
    }

    /// Collects children that participate in a flex/grid formatting context,
    /// descending through `display: contents` wrappers so their children are
    /// placed as if they were direct children.
    fn collect_flow_items(&mut self, node: NodeId, out: &mut Vec<FlexItem>, depth: u32) {
        if depth > 16 {
            return;
        }
        for child in self.doc.rendered_children(node) {
            match self.doc.node(child).map(|n| n.node_type.clone()) {
                Some(NodeType::Text(t)) => {
                    if !t.trim().is_empty() {
                        out.push(FlexItem::Text(child));
                    }
                }
                Some(NodeType::Element(_)) => {
                    let cs = self.style_of(child);
                    if cs.display == Display::None || cs.position.is_out_of_flow() {
                        continue;
                    }
                    if cs.display == Display::Contents {
                        self.collect_flow_items(child, out, depth + 1);
                    } else {
                        out.push(FlexItem::Node(child));
                    }
                }
                _ => {}
            }
        }
    }

    fn layout_flex(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        content: Rect,
        cb_height: Option<f32>,
        depth: u32,
    ) -> (Vec<LayoutBox>, f32) {
        let row = style.flex_direction.is_row();
        let reversed = style.flex_direction.reversed();
        let mut items: Vec<FlexItem> = Vec::new();
        self.collect_flow_items(node, &mut items, 0);
        items.sort_by_key(|item| match item {
            FlexItem::Node(n) => self.style_of(*n).order,
            FlexItem::Text(_) => 0,
        });
        if reversed {
            items.reverse();
        }
        if items.is_empty() {
            return (Vec::new(), 0.0);
        }
        let gap = if row {
            style.column_gap.resolve_or(content.w, 0.0)
        } else {
            style.row_gap.resolve_or(content.w, 0.0)
        };
        let main_avail = if row {
            content.w
        } else {
            style
                .height
                .resolve(cb_height.unwrap_or(0.0))
                .unwrap_or(f32::INFINITY)
        };

        // Base sizes along the main axis.
        let mut base: Vec<f32> = Vec::with_capacity(items.len());
        for item in &items {
            if let FlexItem::Text(t) = item {
                let font = FontSpec::from_style(style);
                let text = self.flex_text(*t);
                let tw = self.fonts.measure(&text, &font);
                base.push(if row { tw } else { style.line_height });
                continue;
            }
            let cs = self.style_of(item.node());
            let m = self.resolve_margins(&cs, content.w);
            let extra = if row {
                cs.padding.to_edges(content.w).horizontal() + cs.border_edges_width().horizontal()
            } else {
                cs.padding.to_edges(content.w).vertical() + cs.border_edges_width().vertical()
            };
            let specified = if row {
                cs.flex_basis
                    .resolve(content.w)
                    .or_else(|| cs.width.resolve(content.w))
            } else {
                cs.flex_basis
                    .resolve(cb_height.unwrap_or(0.0))
                    .or_else(|| cs.height.resolve(cb_height.unwrap_or(0.0)))
            };
                            let size = match specified {
                Some(v) => v,
                None => {
                    if row {
                        self.max_content_width(item.node(), &cs, 0).min(content.w)
                    } else {
                        let measured = self.measure_atomic(item.node(), &cs);
                        measured.1
                    }
                }
            };
            let margins = if row { m.horizontal() } else { m.vertical() };
            base.push(size.max(0.0) + if row { extra } else { 0.0 } + margins);
        }

        // Line breaking along the main axis.
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut line: Vec<usize> = Vec::new();
        let mut line_sum = 0.0f32;
        for (i, b) in base.iter().enumerate() {
            let add = if line.is_empty() { *b } else { *b + gap };
            let wrap = style.flex_wrap != FlexWrap::NoWrap;
            if wrap && !line.is_empty() && line_sum + add > main_avail {
                lines.push(std::mem::take(&mut line));
                line_sum = *b;
                line.push(i);
            } else {
                line_sum += add;
                line.push(i);
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }

        let mut children: Vec<LayoutBox> = Vec::new();
        let mut cross_cursor = content.y;
        let mut used_main = 0.0f32;

        for line in &lines {
            let n = line.len();
            let gaps = gap * (n.saturating_sub(1)) as f32;
            let mut sum: f32 = line.iter().map(|i| base[*i]).sum::<f32>() + gaps;
            // A column flex container with `height: auto` has no definite main
            // size; growing items against infinity gives infinite boxes and
            // NaN positions. In that case the container is sized by its items.
            let free = if main_avail.is_finite() { main_avail - sum } else { 0.0 };
            let mut finals: Vec<f32> = line.iter().map(|i| base[*i]).collect();
            if free > 0.0 {
                let grows: Vec<f32> = line
                    .iter()
                    .map(|i| match items[*i] {
                        FlexItem::Node(n) => self.style_of(n).flex_grow,
                        FlexItem::Text(_) => 0.0,
                    })
                    .collect();
                let total_grow: f32 = grows.iter().sum();
                if total_grow > 0.0 {
                    for (k, g) in grows.iter().enumerate() {
                        finals[k] += free * g / total_grow;
                    }
                }
            } else if free < 0.0 {
                let factors: Vec<f32> = line
                    .iter()
                    .map(|i| match items[*i] {
                        FlexItem::Node(n) => self.style_of(n).flex_shrink * base[*i],
                        FlexItem::Text(_) => base[*i],
                    })
                    .collect();
                let total: f32 = factors.iter().sum();
                if total > 0.0 {
                    for (k, f) in factors.iter().enumerate() {
                        finals[k] += free * f / total;
                    }
                }
            }
            sum = finals.iter().sum::<f32>() + gaps;
            let leftover = if main_avail.is_finite() {
                (main_avail - sum).max(0.0)
            } else {
                0.0
            };

            // Justify along the main axis.
            let justify = style.justify_content;
            let (mut offset, extra_gap) = match justify {
                Align::Center => (leftover / 2.0, 0.0),
                Align::End => (leftover, 0.0),
                Align::SpaceBetween if n > 1 => (0.0, leftover / (n - 1) as f32),
                Align::SpaceAround => (leftover / (2.0 * n as f32), leftover / n as f32),
                Align::SpaceEvenly => (leftover / (n + 1) as f32, leftover / (n + 1) as f32),
                _ => (0.0, 0.0),
            };
            if n == 1 && matches!(justify, Align::SpaceBetween) {
                offset = 0.0;
            }

            let mut placed: Vec<LayoutBox> = Vec::new();
            let mut main_cursor = if row { content.x + offset } else { content.y + offset };
            let mut line_cross = 0.0f32;
            let mut items_line: Vec<(FlexItem, LayoutBox, f32, f32)> = Vec::new();
            for (k, idx) in line.iter().enumerate() {
                let item = items[*idx];
                let cs = match item {
                    FlexItem::Node(n) => self.style_of(n),
                    FlexItem::Text(_) => self.style_rc_from(style),
                };
                let m = self.resolve_margins(&cs, content.w);
                let pad_h = cs.padding.to_edges(content.w).horizontal() + cs.border_edges_width().horizontal();
                let pad_v = cs.padding.to_edges(content.w).vertical() + cs.border_edges_width().vertical();
                let main_margins = if row { m.horizontal() } else { m.vertical() };
                // Border-box size along the main axis. `layout_element` takes a
                // border-box size and subtracts padding/border for border-box
                // styling, so passing the content width here would double-count it.
                let main_border = (finals[k] - main_margins).max(0.0);
                let main_content = (main_border - if row { pad_h } else { pad_v }).max(0.0);
                let main_forced = if cs.box_sizing == BoxSizing::BorderBox {
                    main_border
                } else {
                    main_content
                };
                let (x, y, forced_w, forced_h) = if row {
                    (main_cursor, cross_cursor, Some(main_forced), None)
                } else {
                    // In a column flex container the cross axis is horizontal:
                    // stretch fills the container, an explicit width is kept,
                    // and any other item is shrink-to-fit. Forcing the full
                    // width here made centred pages and 48px icons stretch.
                    let align = if cs.align_self != Align::Auto {
                        cs.align_self
                    } else {
                        style.align_items
                    };
                    let cross_forced = if let Some(w) = cs.width.resolve(content.w) {
                        Some(w.max(0.0))
                    } else if align == Align::Stretch {
                        let cross_border = (content.w - m.horizontal()).max(0.0);
                        Some(if cs.box_sizing == BoxSizing::BorderBox {
                            cross_border
                        } else {
                            (cross_border - pad_h).max(0.0)
                        })
                    } else {
                        match item {
                            FlexItem::Node(n) => {
                                let avail = (content.w - m.horizontal()).max(0.0);
                                let max = self.max_content_width(n, &cs, depth + 1).min(avail);
                                let min = self.min_content_width(n, &cs, depth + 1).min(max);
                                let border = max.max(min);
                                Some(if cs.box_sizing == BoxSizing::BorderBox {
                                    border
                                } else {
                                    (border - pad_h).max(0.0)
                                })
                            }
                            FlexItem::Text(_) => None,
                        }
                    };
                    (content.x, main_cursor, cross_forced, Some(main_forced))
                };
                let mut b = match item {
                    FlexItem::Node(n) => self.layout_element(
                        n,
                        None,
                        x,
                        y,
                        content.w,
                        cb_height,
                        forced_w,
                        forced_h,
                        kind_for(&cs, self.doc.tag_name(n)),
                        depth + 1,
                    ),
                    FlexItem::Text(t) => {
                        self.layout_flex_text(t, style, x, y, forced_w, forced_h)
                    }
                };
                let cross = if row {
                    b.rect.h + m.top + m.bottom
                } else {
                    b.rect.w + m.left + m.right
                };
                line_cross = line_cross.max(cross);
                items_line.push((item, b, cross, main_border));
                main_cursor += finals[k];
                if k + 1 < n {
                    main_cursor += gap + extra_gap;
                }
            }
            // In a single-line flex container the line spans the container on
            // the cross axis, so `align-items: center` can position items.
            if lines.len() == 1 {
                if row {
                    if content.h > 0.0 {
                        line_cross = line_cross.max(content.h);
                    }
                } else {
                    line_cross = line_cross.max(content.w);
                }
            }
            used_main = used_main.max(if row { main_cursor - content.x } else { main_cursor - content.y });

            // Cross-axis alignment.
            for (item, mut b, cross, main_border) in items_line {
                let cs = match item {
                    FlexItem::Node(n) => self.style_of(n),
                    FlexItem::Text(_) => self.style_rc_from(style),
                };
                let m = self.resolve_margins(&cs, content.w);
                let pad_h = cs.padding.to_edges(content.w).horizontal() + cs.border_edges_width().horizontal();
                let pad_v = cs.padding.to_edges(content.w).vertical() + cs.border_edges_width().vertical();
                let border_adjusted = |cs: &ComputedStyle, border: f32, extra: f32| {
                    if cs.box_sizing == BoxSizing::BorderBox {
                        border.max(0.0)
                    } else {
                        (border - extra).max(0.0)
                    }
                };
                let align = if cs.align_self != Align::Auto { cs.align_self } else { style.align_items };
                let cross_pos = if row { b.rect.x } else { b.rect.y };
                let (cross_size, free_cross) = if row {
                    (b.rect.h + m.top + m.bottom, line_cross - (b.rect.h + m.top + m.bottom))
                } else {
                    (b.rect.w + m.left + m.right, line_cross - (b.rect.w + m.left + m.right))
                };
                let _ = (cross_pos, cross_size);
                let mut delta = 0.0f32;
                match align {
                    Align::Center => delta = free_cross / 2.0,
                    Align::End => delta = free_cross,
                    Align::Stretch => {
                        if cross < line_cross {
                            let extra = line_cross - cross;
                            let auto_cross = if row {
                                cs.height.resolve(0.0).is_none()
                            } else {
                                cs.width.resolve(0.0).is_none()
                            };
                            if auto_cross && matches!(item, FlexItem::Node(_)) {
                                let text_start = self.texts.len();
                                let image_start = self.images_out.len();
                                // Keep the main-axis size the flex algorithm chose and
                                // only grow the cross axis; using `width:auto` here would
                                // re-expand the item to the container's full width.
                                let (fw, fh) = if row {
                                    (
                                        Some(border_adjusted(&cs, main_border, pad_h)),
                                        Some(border_adjusted(&cs, (line_cross - m.vertical()).max(0.0), pad_v)),
                                    )
                                } else {
                                    (
                                        Some(border_adjusted(&cs, (line_cross - m.horizontal()).max(0.0), pad_h)),
                                        Some(border_adjusted(&cs, main_border, pad_v)),
                                    )
                                };
                                let new_b = self.layout_element(
                                    item.node(),
                                    None,
                                    b.rect.x,
                                    b.rect.y,
                                    content.w,
                                    cb_height,
                                    fw,
                                    fh,
                                    kind_for(&cs, self.doc.tag_name(item.node())),
                                    depth + 1,
                                );
                                self.discard_since(text_start, image_start);
                                b = new_b;
                            }
                            let _ = extra;
                        }
                    }
                    _ => {}
                }
                if row {
                    let text_start = self.texts.len();
                    let image_start = self.images_out.len();
                    if delta.abs() > 0.01 {
                        self.translate_subtree(&mut b, 0.0, delta, text_start, image_start);
                    }
                } else {
                    let text_start = self.texts.len();
                    let image_start = self.images_out.len();
                    if delta.abs() > 0.01 {
                        self.translate_subtree(&mut b, delta, 0.0, text_start, image_start);
                    }
                }
                placed.push(b);
            }
            children.extend(placed);
            cross_cursor += line_cross + if row {
                style.row_gap.resolve_or(content.w, 0.0)
            } else {
                style.column_gap.resolve_or(content.w, 0.0)
            };
        }
        // No gap follows the final line/item, so remove the trailing gap the
        // loop added after it.
        let line_count = lines.len();
        let trailing_gap = if line_count == 0 {
            0.0
        } else if row {
            style.row_gap.resolve_or(content.w, 0.0)
        } else {
            style.column_gap.resolve_or(content.w, 0.0)
        };
        // In a row container height comes from the wrapped lines; in a column
        // container the cross cursor tracks widths and must not become height.
        let mut height = if row {
            cross_cursor - content.y
        } else {
            used_main
        };
        height = (height - trailing_gap).max(0.0);
        (children, height)
    }

    fn discard_since(&mut self, text_start: usize, image_start: usize) {
        self.texts.truncate(text_start);
        self.images_out.truncate(image_start);
    }

    // ---- grid ------------------------------------------------------------

    fn layout_grid(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        content: Rect,
        cb_height: Option<f32>,
        depth: u32,
    ) -> (Vec<LayoutBox>, f32) {
        let mut flow: Vec<FlexItem> = Vec::new();
        self.collect_flow_items(node, &mut flow, 0);
        let mut items: Vec<NodeId> = flow
            .into_iter()
            .filter_map(|item| match item {
                FlexItem::Node(n) => Some(n),
                FlexItem::Text(_) => None,
            })
            .collect();
        if items.is_empty() {
            return (Vec::new(), 0.0);
        }
        let gap_x = style.column_gap.resolve_or(content.w, 0.0);
        let gap_y = style.row_gap.resolve_or(content.w, 0.0);
        let area_cols = style
            .grid_areas
            .iter()
            .map(|row| row.len())
            .max()
            .unwrap_or(0);
        let ncols = if style.grid_columns.is_empty() {
            area_cols.max(1)
        } else {
            style.grid_columns.len().max(1)
        };
        let tracks = if style.grid_columns.is_empty() {
            vec![GridTrack::Fr(1.0); ncols]
        } else {
            style.grid_columns.clone()
        };
        let fixed: f32 = tracks
            .iter()
            .map(|t| match t {
                GridTrack::Px(v) => *v,
                GridTrack::Percent(p) => content.w * p / 100.0,
                GridTrack::MinMax(min, _) => *min,
                _ => 0.0,
            })
            .sum();
        let fr_total: f32 = tracks
            .iter()
            .map(|t| match t {
                GridTrack::Fr(f) => *f,
                _ => 0.0,
            })
            .sum();
        let remaining = (content.w - fixed - gap_x * (ncols.saturating_sub(1)) as f32).max(0.0);

        // Place items: named grid areas first, then explicit grid lines,
        // then row-major auto placement.
        struct Cell {
            item: NodeId,
            row: usize,
            col: usize,
            span: usize,
            row_span: usize,
        }
        let mut cells: Vec<Cell> = Vec::new();
        let mut occupied: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
        let mut auto_items: Vec<(NodeId, usize)> = Vec::new();
        for item in &items {
            let cs = self.style_of(*item);
            let mut placed: Option<(usize, usize, usize, usize)> = None;
            if let Some(name) = &cs.grid_area {
                if let Some((r0, c0, r1, c1)) = area_bounds(&style.grid_areas, name) {
                    placed = Some((r0, c0, r1 - r0 + 1, c1 - c0 + 1));
                }
            }
            if placed.is_none() {
                let row_line = match cs.grid_row_start {
                    GridPlacement::Line(l) => Some((l.max(1) as usize - 1).min(4096)),
                    _ => None,
                };
                let col_line = match cs.grid_column_start {
                    GridPlacement::Line(l) => Some((l.max(1) as usize - 1).min(ncols - 1)),
                    _ => None,
                };
                if row_line.is_some() || col_line.is_some() {
                    placed = Some((row_line.unwrap_or(0), col_line.unwrap_or(0), 1, 1));
                }
            }
            if let Some((r, c, row_span, mut span)) = placed {
                // An explicit end line can widen the column span. Negative
                // lines count from the end (`-1` is the final grid line), so
                // `grid-column: 1 / -1` spans every column.
                if let GridPlacement::Line(l) = cs.grid_column_end {
                    let e = if l < 0 {
                        ncols.saturating_sub((-l) as usize)
                    } else {
                        (l.max(1) as usize - 1).min(ncols)
                    };
                    if e > c {
                        span = span.max(e - c);
                    }
                }
                let span = span.min(ncols.saturating_sub(c)).max(1);
                for rr in r..(r + row_span.max(1)) {
                    for cc in c..(c + span) {
                        occupied.insert((rr, cc));
                    }
                }
                cells.push(Cell { item: *item, row: r, col: c, span, row_span });
            } else {
                let mut span = 1usize;
                if let GridPlacement::Span(n) = cs.grid_column_start {
                    span = n as usize;
                }
                if let GridPlacement::Span(n) = cs.grid_column_end {
                    span = (n as usize).max(span);
                }
                auto_items.push((*item, span.min(ncols).max(1)));
            }
        }
        // Auto-placement starts at the first cell that is not taken by an
        // explicitly placed item. Without this, items with `col-start-2` and
        // rows skip ahead and their siblings end up crammed into a gutter
        // (Tailwind's homepage is built exactly this way).
        let mut cursor_row = 0usize;
        let mut cursor_col = 0usize;
        for (item, span) in auto_items {
            loop {
                if cursor_col + span > ncols {
                    cursor_col = 0;
                    cursor_row += 1;
                    continue;
                }
                let free = (0..span).all(|k| !occupied.contains(&(cursor_row, cursor_col + k)));
                if free {
                    for k in 0..span {
                        occupied.insert((cursor_row, cursor_col + k));
                    }
                    cells.push(Cell { item, row: cursor_row, col: cursor_col, span, row_span: 1 });
                    cursor_col += span;
                    if cursor_col >= ncols {
                        cursor_col = 0;
                        cursor_row += 1;
                    }
                    break;
                }
                cursor_col += 1;
            }
        }
        let nrows = cells
            .iter()
            .map(|c| c.row + c.row_span)
            .max()
            .unwrap_or(style.grid_areas.len())
            .max(style.grid_areas.len())
            .max(1);

        // Track sizing. `min-content`/`auto` tracks take the minimum content
        // width of their items; fixed tracks shrink so those minima fit when
        // the sum exceeds the container (MediaWiki's article + appearance
        // columns rely on this).
        let mut auto_min = vec![0.0f32; ncols];
        for cell in &cells {
            if matches!(
                tracks[cell.col],
                GridTrack::Auto | GridTrack::MinContent | GridTrack::MaxContent
            ) && cell.span == 1
            {
                let cs = self.style_of(cell.item);
                let intrinsic = self.min_content_width(cell.item, &cs, depth + 1);
                auto_min[cell.col] = auto_min[cell.col].max(intrinsic);
            }
        }
        let gaps = gap_x * (ncols.saturating_sub(1)) as f32;
        let available = (content.w - gaps).max(0.0);
        let auto_total: f32 = (0..ncols)
            .filter(|i| {
                matches!(
                    tracks[*i],
                    GridTrack::Auto | GridTrack::MinContent | GridTrack::MaxContent
                )
            })
            .map(|i| auto_min[i])
            .sum();
        let mut widths = vec![0.0f32; ncols];
        if fr_total > 0.0 {
            let fixed_auto: f32 = tracks
                .iter()
                .enumerate()
                .map(|(i, t)| match t {
                    GridTrack::Px(v) => *v,
                    GridTrack::Percent(p) => content.w * p / 100.0,
                    GridTrack::MinMax(min, _) => *min,
                    GridTrack::Auto | GridTrack::MinContent | GridTrack::MaxContent => auto_min[i],
                    _ => 0.0,
                })
                .sum();
            let rem = (available - fixed_auto).max(0.0);
            for (i, t) in tracks.iter().enumerate() {
                widths[i] = match t {
                    GridTrack::Px(v) => *v,
                    GridTrack::Percent(p) => content.w * p / 100.0,
                    GridTrack::MinMax(min, _) => *min,
                    GridTrack::Auto | GridTrack::MinContent | GridTrack::MaxContent => auto_min[i],
                    GridTrack::Fr(f) => rem * f / fr_total,
                };
            }
        } else {
            // Fixed tracks do not shrink on their own in CSS grid. Only a
            // separate auto/min-content column forces the fixed track to give
            // way (MediaWiki's article/appearance split relies on that); with
            // no such column, overflow is the correct behaviour.
            let has_minmax = tracks.iter().any(|t| matches!(t, GridTrack::MinMax(_, _)));
            let fixed_total = fixed;
            let scale = if !has_minmax && fixed_total > 0.0 && fixed_total + auto_total > available {
                ((available - auto_total).max(0.0)) / fixed_total
            } else {
                1.0
            };
            for (i, t) in tracks.iter().enumerate() {
                widths[i] = match t {
                    GridTrack::Px(v) => *v * scale,
                    GridTrack::Percent(p) => content.w * p / 100.0 * scale,
                    GridTrack::MinMax(min, _) => *min * scale,
                    _ => auto_min[i],
                };
            }
            let used: f32 = widths.iter().sum();
            let mut spare = (available - used).max(0.0);
            // Grow finite minmax tracks up to their maximum first: a
            // `minmax(0, 768px)` content column should absorb free space
            // before `minmax(15rem, 1fr)` side columns grow. Then share any
            // remaining space between the unbounded minmax columns.
            for (i, t) in tracks.iter().enumerate() {
                if let GridTrack::MinMax(min, max) = t {
                    if max.is_finite() {
                        let extra = (max - min).max(0.0).min(spare);
                        widths[i] += extra;
                        spare -= extra;
                    }
                }
            }
            let unbounded: Vec<usize> = tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| matches!(t, GridTrack::MinMax(_, max) if !max.is_finite()))
                .map(|(i, _)| i)
                .collect();
            if !unbounded.is_empty() && spare > 0.5 {
                let share = spare / unbounded.len() as f32;
                for i in unbounded {
                    widths[i] += share;
                }
                spare = 0.0;
            }
            let auto_count = tracks
                .iter()
                .filter(|t| {
                    matches!(
                        t,
                        GridTrack::Auto | GridTrack::MinContent | GridTrack::MaxContent
                    )
                })
                .count();
            if auto_count > 0 && spare > 0.5 {
                for (i, t) in tracks.iter().enumerate() {
                    if matches!(
                        t,
                        GridTrack::Auto | GridTrack::MinContent | GridTrack::MaxContent
                    ) {
                        widths[i] += spare / auto_count as f32;
                    }
                }
            }
        }
        let _ = remaining;
        if std::env::var("VEYRA_DEBUG_GRID").is_ok() {
            eprintln!(
                "GRID tag={} content_w={} tracks={:?} widths={:?} auto_min={:?} ncols={} fr_total={}",
                self.doc.tag_name(node), content.w, tracks, widths, auto_min, ncols, fr_total
            );
        }

        let mut row_heights = vec![0.0f32; nrows];
        let mut laid: Vec<Option<LayoutBox>> = Vec::new();
        laid.resize_with(cells.len(), || None);
        for (i, cell) in cells.iter().enumerate() {
            let width: f32 = (cell.col..(cell.col + cell.span).min(ncols)).map(|c| widths[c]).sum::<f32>()
                + gap_x * (cell.span.saturating_sub(1)) as f32;
            let cs = self.style_of(cell.item);
            // Grid items size their content box to the track minus their own
            // margins; the margins are applied again when the item is placed
            // in its row below.
            let m = self.resolve_margins(&cs, content.w);
            let border_w = (width - m.horizontal()).max(0.0);
            let b = self.layout_element(
                cell.item,
                None,
                content.x + widths[..cell.col].iter().sum::<f32>() + gap_x * cell.col as f32,
                content.y,
                width.max(0.0),
                cb_height,
                Some(border_w),
                None,
                kind_for(&cs, self.doc.tag_name(cell.item)),
                depth + 1,
            );
            if cell.row_span <= 1 {
                // An empty item's bottom margin collapses through, so it must
                // not contribute a second track of blank space (MediaWiki's
                // empty `#siteNotice` above the article body).
                let v = b.rect.h
                    + b.margin.top
                    + if b.rect.h > 0.5 { b.margin.bottom } else { 0.0 };
                row_heights[cell.row] = row_heights[cell.row].max(v);
            }
            laid[i] = Some(b);
        }
        // Items spanning several rows (e.g. a sidebar next to an article) grow
        // the last row they cover until their full height fits.
        for (i, cell) in cells.iter().enumerate() {
            if cell.row_span <= 1 {
                continue;
            }
            let Some(b) = &laid[i] else { continue };
            let need = b.rect.h + b.margin.vertical();
            let have: f32 = (cell.row..(cell.row + cell.row_span).min(nrows))
                .map(|r| row_heights[r])
                .sum::<f32>()
                + gap_y * (cell.row_span.saturating_sub(1)) as f32;
            if need > have {
                let last = (cell.row + cell.row_span - 1).min(nrows - 1);
                row_heights[last] += need - have;
            }
        }
        let mut row_offsets = vec![content.y; nrows];
        for r in 1..nrows {
            row_offsets[r] = row_offsets[r - 1] + row_heights[r - 1] + gap_y;
        }
        let mut children = Vec::new();
        for (i, cell) in cells.iter().enumerate() {
            if let Some(mut b) = laid[i].take() {
                let target_y = row_offsets[cell.row] + b.margin.top;
                let text_start = self.texts.len();
                let image_start = self.images_out.len();
                let dy = target_y - b.rect.y;
                if dy.abs() > 0.01 {
                    self.translate_subtree(&mut b, 0.0, dy, text_start, image_start);
                }
                children.push(b);
            }
        }
        let used_h = row_offsets.last().map(|y| y + row_heights.last().copied().unwrap_or(0.0)).unwrap_or(content.y)
            - content.y;
        (children, used_h)
    }

    // ---- tables ----------------------------------------------------------

    fn layout_table(
        &mut self,
        node: NodeId,
        style: &ComputedStyle,
        content: Rect,
        depth: u32,
    ) -> (Vec<LayoutBox>, f32) {
        let spacing = if style.border_collapse { 0.0 } else { style.border_spacing };
        let mut rows: Vec<(NodeId, Vec<NodeId>)> = Vec::new();
        let mut captions: Vec<NodeId> = Vec::new();
        for child in self.doc.rendered_children(node) {
            let tag = self.doc.tag_name(child);
            match tag {
                "caption" => captions.push(child),
                "tr" => {
                    let cells = self.cells_of(child);
                    rows.push((child, cells));
                }
                "thead" | "tbody" | "tfoot" => {
                    for tr in self.doc.element_children(child) {
                        if self.doc.tag_name(tr) == "tr" {
                            let cells = self.cells_of(tr);
                            rows.push((tr, cells));
                        }
                    }
                }
                _ => {}
            }
        }
        if rows.is_empty() {
            // `display: table` is used for non-table markup too (MediaWiki
            // thumbnails wrap images in a `<figure>`), so fall back to normal
            // flow layout and keep the child boxes.
            return self.layout_flow(node, style, content, None, depth);
        }
        let ncols = rows
            .iter()
            .map(|(_, cells)| cells.iter().map(|c| self.colspan(*c)).sum::<usize>())
            .max()
            .unwrap_or(1)
            .max(1);
        let avail = content.w - spacing * (ncols + 1) as f32;
        let col_w = (avail / ncols as f32).max(0.0);
        let mut children = Vec::new();
        let mut y = content.y + spacing;
        let mut caption_h = 0.0;
        for cap in captions {
            let cs = self.style_of(cap);
            let b = self.layout_element(cap, None, content.x, y, content.w, None, None, None, LayoutKind::TableCaption, depth + 1);
            caption_h += b.rect.h;
            y += b.rect.h;
            children.push(b);
            let _ = cs;
        }
        for (row_node, cells) in rows {
            let mut x = content.x + spacing;
            let mut row_children = Vec::new();
            let mut row_h = 0.0f32;
            let mut cidx = 0usize;
            for cell in &cells {
                if cidx >= ncols {
                    break;
                }
                let span = self.colspan(*cell).min(ncols - cidx).max(1);
                let width = col_w * span as f32 + spacing * (span - 1) as f32;
                let cs = self.style_of(*cell);
                let b = self.layout_element(
                    *cell,
                    None,
                    x,
                    y,
                    width.max(0.0),
                    None,
                    Some(width.max(0.0)),
                    None,
                    LayoutKind::TableCell,
                    depth + 1,
                );
                row_h = row_h.max(b.rect.h + b.margin.vertical());
                x += width + spacing;
                row_children.push(b);
                cidx += span;
                let _ = cs;
            }
            let rs = self.style_of(row_node);
            let row_box = LayoutBox {
                node: Some(row_node),
                pseudo: None,
                tag: "tr".into(),
                style: rs,
                kind: LayoutKind::TableRow,
                rect: Rect::new(content.x, y, content.w, row_h + spacing),
                content: Rect::new(content.x, y, content.w, row_h),
                border: Edges::ZERO,
                padding: Edges::ZERO,
                margin: Edges::ZERO,
                children: row_children,
                clip: false,
                form: None,
                image: None,
                scroll_size: Size::ZERO,
            };
            y += row_h + spacing;
            children.push(row_box);
        }
        (children, (y - content.y).max(caption_h))
    }

    fn cells_of(&self, row: NodeId) -> Vec<NodeId> {
        self.doc
            .element_children(row)
            .into_iter()
            .filter(|c| matches!(self.doc.tag_name(*c), "td" | "th"))
            .collect()
    }

    fn colspan(&self, cell: NodeId) -> usize {
        self.doc
            .attr(cell, "colspan")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1)
            .min(20)
            .max(1)
    }
}

struct InlineToken {
    kind: TokenKind,
    text: String,
    width: f32,
    font: FontSpec,
    line_height: f32,
    height: f32,
    style: Rc<ComputedStyle>,
    node: Option<NodeId>,
    ancestors: Vec<NodeId>,
}

impl Clone for InlineToken {
    fn clone(&self) -> Self {
        InlineToken {
            kind: self.kind.clone(),
            text: self.text.clone(),
            width: self.width,
            font: self.font.clone(),
            line_height: self.line_height,
            height: self.height,
            style: self.style.clone(),
            node: self.node,
            ancestors: self.ancestors.clone(),
        }
    }
}

#[derive(Clone)]
enum TokenKind {
    Text { text: String },
    Space { width: f32 },
    Newline,
    Atomic { node: NodeId },
}

impl TokenKind {
    fn is_text(&self) -> bool {
        matches!(self, TokenKind::Text { .. })
    }
}

struct PlacedToken {
    token: InlineToken,
    x: f32,
    w: f32,
    space_before: f32,
}

fn collect_box_nodes(b: &LayoutBox, out: &mut Vec<NodeId>) {
    if let Some(n) = b.node {
        out.push(n);
    }
    for c in &b.children {
        collect_box_nodes(c, out);
    }
}

/// Finds the bounding box of a named `grid-template-areas` cell.
fn area_bounds(areas: &[Vec<String>], name: &str) -> Option<(usize, usize, usize, usize)> {
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for (r, row) in areas.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell == name {
                bounds = Some(match bounds {
                    Some((r0, c0, r1, c1)) => (r0.min(r), c0.min(c), r1.max(r), c1.max(c)),
                    None => (r, c, r, c),
                });
            }
        }
    }
    bounds
}

fn translate_box(b: &mut LayoutBox, dx: f32, dy: f32) {
    b.rect.x += dx;
    b.rect.y += dy;
    b.content.x += dx;
    b.content.y += dy;
    for c in &mut b.children {
        translate_box(c, dx, dy);
    }
    for t in &mut b.children {
        let _ = t;
    }
    if let Some(img) = &mut b.image {
        let _ = img;
    }
}

fn margin_gap(prev: f32, current: f32, saw_block: bool) -> f32 {
    if !saw_block {
        current
    } else {
        prev.max(current)
    }
}

fn apply_text_transform(text: &str, transform: TextTransform) -> String {
    match transform {
        TextTransform::None => text.to_string(),
        TextTransform::Uppercase => text.to_uppercase(),
        TextTransform::Lowercase => text.to_lowercase(),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut start = true;
            for c in text.chars() {
                if c.is_alphanumeric() {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.push(c);
                    }
                    start = false;
                } else {
                    out.push(c);
                    start = true;
                }
            }
            out
        }
    }
}

pub fn font_spec_for(style: &ComputedStyle) -> FontSpec {
    let _ = FontStyle::Normal;
    FontSpec::from_style(style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use veyra_common::Url;
    use veyra_css::StyleEngine;
    use veyra_html::parse_document;

    fn build(html: &str, css: &str, width: f32) -> (Document, LayoutTree) {
        let base = Url::parse("https://example.com/").unwrap();
        let (doc, _) = parse_document(html, base);
        let mut styles = StyleEngine::new();
        styles.set_viewport(width, 800.0);
        styles.add_stylesheet(css, &doc.base_url);
        styles.compute(&doc);
        let tree = layout_document(&doc, &styles, &crate::font::SimpleMeasure, &crate::font::NoImages, Size::new(width, 800.0));
        (doc, tree)
    }

    #[test]
    fn block_stacking_and_margins() {
        let (doc, tree) = build(
            "<div id=a style='height:50px'></div><div id=b style='height:30px;margin-top:10px'></div>",
            "",
            500.0,
        );
        let a = doc.get_element_by_id("a").unwrap();
        let b = doc.get_element_by_id("b").unwrap();
        let ra = tree.boxes[&a];
        let rb = tree.boxes[&b];
        assert_eq!(ra.y, 8.0);
        assert_eq!(rb.y, ra.bottom() + 10.0);
    }

    #[test]
    fn inline_text_wraps() {
        let (_, tree) = build(
            "<p>one two three four five six seven eight nine ten eleven twelve</p>",
            "body{margin:0} p{width:100px;margin:0;font-size:16px}",
            300.0,
        );
        assert!(tree.texts.len() > 3, "text should be split into words");
        let max_right = tree.texts.iter().map(|t| t.rect.right()).fold(0.0, f32::max);
        assert!(max_right <= 101.0, "text overflowed: {max_right}");
        let ys: std::collections::HashSet<i32> = tree.texts.iter().map(|t| t.rect.y as i32).collect();
        assert!(ys.len() > 1, "expected multiple lines");
    }

    #[test]
    fn flex_row_distributes_space() {
        let (doc, tree) = build(
            "<div id=f><div id=a></div><div id=b></div></div>",
            "#f{display:flex;width:300px;height:40px} #a{flex:1;background:red} #b{flex:2}",
            500.0,
        );
        let a = tree.boxes[&doc.get_element_by_id("a").unwrap()];
        let b = tree.boxes[&doc.get_element_by_id("b").unwrap()];
        assert!((a.w - 100.0).abs() < 1.0, "a width {}", a.w);
        assert!((b.w - 200.0).abs() < 1.0, "b width {}", b.w);
        assert_eq!(a.y, b.y);
    }

    #[test]
    fn grid_places_in_columns() {
        let (doc, tree) = build(
            "<div id=g><div id=a></div><div id=b></div><div id=c></div></div>",
            "#g{display:grid;grid-template-columns:1fr 1fr;gap:10px;width:210px} #g>div{height:20px}",
            400.0,
        );
        let a = tree.boxes[&doc.get_element_by_id("a").unwrap()];
        let b = tree.boxes[&doc.get_element_by_id("b").unwrap()];
        let c = tree.boxes[&doc.get_element_by_id("c").unwrap()];
        assert!((a.w - 100.0).abs() < 1.0, "a {}", a.w);
        assert_eq!(a.y, b.y);
        assert!((b.x - (a.right() + 10.0)).abs() < 1.0);
        assert!(c.y > a.bottom());
    }

    #[test]
    fn absolute_bottom_with_auto_height() {
        let (doc, tree) = build(
            "<div id=rel style='position:relative;height:200px'><div id=abs style='position:absolute;bottom:10px;width:50px;padding:5px'>x</div></div>",
            "body{margin:0}",
            500.0,
        );
        let rel = tree.boxes[&doc.get_element_by_id("rel").unwrap()];
        let abs = tree.boxes[&doc.get_element_by_id("abs").unwrap()];
        assert!((abs.bottom() - (rel.bottom() - 10.0)).abs() < 1.5, "abs bottom {} vs {}", abs.bottom(), rel.bottom());
    }

    #[test]
    fn absolute_positioning() {
        let (doc, tree) = build(
            "<div id=rel style='position:relative;height:200px'><div id=abs style='position:absolute;top:10px;left:20px;width:50px;height:50px'></div></div>",
            "body{margin:0}",
            500.0,
        );
        let rel = tree.boxes[&doc.get_element_by_id("rel").unwrap()];
        let abs = tree.boxes[&doc.get_element_by_id("abs").unwrap()];
        assert_eq!(abs.x, rel.x + 0.0 + 0.0 + 20.0);
        assert_eq!(abs.y, rel.y + 10.0);
    }

    #[test]
    fn tables_get_rows_and_cells() {
        let (doc, tree) = build(
            "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>",
            "table{width:200px;border-spacing:0}",
            500.0,
        );
        assert_eq!(doc.query_selector_all(doc.root, "td").len(), 4);
        let cells = doc.query_selector_all(doc.root, "td");
        assert_eq!(tree.boxes[&cells[0]].y, tree.boxes[&cells[1]].y);
        assert!(tree.boxes[&cells[2]].y >= tree.boxes[&cells[0]].bottom());
    }

    #[test]
    fn lists_add_markers() {
        let (_, tree) = build("<ul><li>one</li><li>two</li></ul>", "", 400.0);
        let markers: Vec<_> = tree.texts.iter().filter(|t| t.is_marker).collect();
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].text, "•");
    }

    #[test]
    fn max_width_block_is_centered() {
        let (doc, tree) = build(
            "<div id=w style='height:20px'></div>",
            "#w{max-width:600px;margin:0 auto}",
            800.0,
        );
        let w = tree.boxes[&doc.get_element_by_id("w").unwrap()];
        assert!((w.w - 600.0).abs() < 1.0, "width {}", w.w);
        assert!((w.x - 100.0).abs() < 1.0, "centered x {}", w.x);
    }

    #[test]
    fn flex_children_respect_container_padding() {
        let (doc, tree) = build(
            "<div id=f><div id=a></div><div id=b></div></div>",
            "#f{display:flex;width:300px;height:40px;padding:10px;box-sizing:border-box} #a,#b{flex:1}",
            500.0,
        );
        let f = tree.boxes[&doc.get_element_by_id("f").unwrap()];
        let a = tree.boxes[&doc.get_element_by_id("a").unwrap()];
        let b = tree.boxes[&doc.get_element_by_id("b").unwrap()];
        assert!((a.x - (f.x + 10.0)).abs() < 1.0, "a.x {}", a.x);
        assert!((a.w - 140.0).abs() < 1.0, "a.w {}", a.w);
        assert!((b.right() - (f.right() - 10.0)).abs() < 1.0, "b right {}", b.right());
    }

    #[test]
    fn flex_text_child_lays_out_inside_container() {
        let (doc, tree) = build(
            "<div id=f>Hello flexible world</div>",
            "#f{display:flex;width:90px;font-size:16px}",
            300.0,
        );
        let node = doc.get_element_by_id("f").unwrap();
        let f = tree.boxes[&node];
        let frags: Vec<_> = tree.texts.iter().filter(|t| t.text.contains("Hello") || t.text.contains("flexible")).collect();
        assert!(!frags.is_empty(), "flex container text should be measured");
        assert!(
            frags.iter().all(|t| t.rect.right() <= f.right() + 0.5),
            "text escaped the flex container: {:?}",
            frags.iter().map(|t| (t.text.clone(), t.rect.right())).collect::<Vec<_>>()
        );
    }

    #[test]
    fn flex_gap_leaves_no_trailing_space() {
        let (doc, tree) = build(
            "<div id=f><div id=a></div><div id=b></div><div id=c></div></div>",
            "#f{display:flex;gap:10px;width:320px;height:20px} #f>div{flex:1}",
            500.0,
        );
        let f = tree.boxes[&doc.get_element_by_id("f").unwrap()];
        let c = tree.boxes[&doc.get_element_by_id("c").unwrap()];
        assert!((c.right() - f.right()).abs() < 1.0, "trailing gap at {}", c.right());
    }

    #[test]
    fn grid_min_content_track_shrinks_fixed_track() {
        let (doc, tree) = build(
            "<div id=g><div id=main></div><div id=aside></div></div>",
            "#g{display:grid;grid-template-columns:minmax(0,400px) min-content;width:500px}\n             #aside{width:150px;height:20px} #main{height:20px}",
            600.0,
        );
        let g = tree.boxes[&doc.get_element_by_id("g").unwrap()];
        let main = tree.boxes[&doc.get_element_by_id("main").unwrap()];
        let aside = tree.boxes[&doc.get_element_by_id("aside").unwrap()];
        assert!((main.w - 350.0).abs() < 1.0, "main {}", main.w);
        assert!((aside.w - 150.0).abs() < 1.0, "aside {}", aside.w);
        assert!((aside.right() - g.right()).abs() < 1.0);
    }

    #[test]
    fn float_reserves_space_for_inline_text() {
        let (doc, tree) = build(
            "<div id=box><div id=f></div><p>alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima</p></div>",
            "#box{width:300px;font-size:16px;line-height:20px} #f{float:right;width:100px;height:20px;background:red} p{margin:0}",
            400.0,
        );
        let float = tree.boxes[&doc.get_element_by_id("f").unwrap()];
        let overlapping: Vec<_> = tree
            .texts
            .iter()
            .filter(|t| t.rect.y < float.bottom() - 0.5 && t.rect.right() > float.x - 0.5)
            .collect();
        assert!(overlapping.is_empty(), "text overlaps float: {:?}", overlapping);
        assert!(
            tree.texts.iter().any(|t| t.rect.right() > float.x + 1.0),
            "text never returns to full width below the float"
        );
    }

    #[test]
    fn empty_block_margins_collapse() {
        let (doc, tree) = build(
            "<div id=e style='margin:20px 0'></div><div id=b style='height:10px'></div>",
            "",
            500.0,
        );
        let e = tree.boxes[&doc.get_element_by_id("e").unwrap()];
        let b = tree.boxes[&doc.get_element_by_id("b").unwrap()];
        assert!((b.y - e.y).abs() < 1.0, "empty block added double margins: {} vs {}", b.y, e.y);
    }

    #[test]
    fn fixed_height_flex_centers_items() {
        let (doc, tree) = build(
            "<div id=f><div id=a></div></div>",
            "#f{display:flex;height:100px;align-items:center} #a{width:10px;height:20px}",
            300.0,
        );
        let f = tree.boxes[&doc.get_element_by_id("f").unwrap()];
        let a = tree.boxes[&doc.get_element_by_id("a").unwrap()];
        let expected = f.y + (f.h - a.h) / 2.0;
        assert!((a.y - expected).abs() < 1.0, "centered y {} (expected {expected})", a.y);
    }
}
