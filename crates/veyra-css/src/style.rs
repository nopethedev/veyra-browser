//! Computed styles: the resolved result of the cascade for one element.

use std::collections::HashMap;

use veyra_common::{Color, Edges, Size, Url};

use crate::values::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDecoration {
    None,
    Underline,
    LineThrough,
    Overline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordBreak {
    Normal,
    BreakAll,
    KeepAll,
    BreakWord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowWrap {
    Normal,
    BreakWord,
    Anywhere,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VerticalAlign {
    Baseline,
    Top,
    Middle,
    Bottom,
    TextTop,
    TextBottom,
    Sub,
    Super,
    Length(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundClip {
    #[default]
    BorderBox,
    PaddingBox,
    ContentBox,
    /// Clip the background to the element's text (gradient headings).
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundRepeat {
    Repeat,
    NoRepeat,
    RepeatX,
    RepeatY,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundSize {
    Auto,
    Cover,
    Contain,
    Explicit(Length, Length),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStylePosition {
    Inside,
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableLayout {
    Auto,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Default,
    Pointer,
    Text,
    Move,
    NotAllowed,
    Crosshair,
    Wait,
    Help,
    Grab,
    Grabbing,
    ZoomIn,
}

/// A 2D affine transform (column-major CSS matrix).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform2D {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Transform2D {
    pub const IDENTITY: Transform2D = Transform2D { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
    pub fn translate(x: f32, y: f32) -> Transform2D {
        Transform2D { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: x, f: y }
    }
    pub fn scale(x: f32, y: f32) -> Transform2D {
        Transform2D { a: x, b: 0.0, c: 0.0, d: y, e: 0.0, f: 0.0 }
    }
    pub fn rotate(deg: f32) -> Transform2D {
        let r = deg.to_radians();
        let (s, c) = r.sin_cos();
        Transform2D { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }
    pub fn skew(x: f32, y: f32) -> Transform2D {
        let (tx, ty) = (x.to_radians().tan(), y.to_radians().tan());
        Transform2D { a: 1.0, b: ty, c: tx, d: 1.0, e: 0.0, f: 0.0 }
    }
    /// Matrix product `self * other` (other applies first).
    pub fn multiply(&self, o: &Transform2D) -> Transform2D {
        Transform2D {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
    pub fn inverse(&self) -> Option<Transform2D> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-9 {
            return None;
        }
        let ia = self.d / det;
        let ib = -self.b / det;
        let ic = -self.c / det;
        let id = self.a / det;
        Some(Transform2D {
            a: ia,
            b: ib,
            c: ic,
            d: id,
            e: -(ia * self.e + ic * self.f),
            f: -(ib * self.e + id * self.f),
        })
    }
    pub fn is_identity(&self) -> bool {
        (self.a - 1.0).abs() < 1e-6
            && self.b.abs() < 1e-6
            && self.c.abs() < 1e-6
            && (self.d - 1.0).abs() < 1e-6
            && self.e.abs() < 1e-6
            && self.f.abs() < 1e-6
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    pub inset: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridAutoFlow {
    Row,
    Column,
    RowDense,
    ColumnDense,
}

#[derive(Debug, Clone)]
pub struct ComputedStyle {
    // Box generation and layout.
    pub display: Display,
    pub position: Position,
    pub float: Float,
    pub clear: Clear,
    pub width: Length,
    pub height: Length,
    pub min_width: Length,
    pub max_width: Length,
    pub min_height: Length,
    pub max_height: Length,
    pub margin: EdgesLength,
    pub padding: EdgesLength,
    pub border_width: EdgesLength,
    pub border_style: [BorderStyle; 4],
    pub border_color: [Color; 4],
    pub border_radius: f32,
    pub box_sizing: BoxSizing,
    pub top: Length,
    pub right: Length,
    pub bottom: Length,
    pub left: Length,
    pub z_index: i32,
    pub opacity: f32,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: Align,
    pub align_items: Align,
    pub align_content: Align,
    pub align_self: Align,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Length,
    pub order: i32,
    pub row_gap: Length,
    pub column_gap: Length,
    pub grid_columns: Vec<GridTrack>,
    pub grid_rows: Vec<GridTrack>,
    pub grid_auto_flow: GridAutoFlow,
    pub grid_column_start: GridPlacement,
    pub grid_column_end: GridPlacement,
    pub grid_row_start: GridPlacement,
    pub grid_row_end: GridPlacement,
    /// Named areas from `grid-template-areas`; empty strings are `.` cells.
    pub grid_areas: Vec<Vec<String>>,
    /// The item's `grid-area` name, if it was placed by name.
    pub grid_area: Option<String>,

    // Typography.
    pub color: Color,
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: u16,
    pub font_style: FontStyle,
    pub line_height: f32,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub text_align: TextAlign,
    pub text_decoration: TextDecoration,
    pub text_decoration_color: Option<Color>,
    pub text_transform: TextTransform,
    pub text_indent: f32,
    pub white_space: WhiteSpace,
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrap,
    pub direction: Direction,
    pub vertical_align: VerticalAlign,

    // Backgrounds.
    pub background_color: Color,
    pub background_image: Option<Url>,
    pub background_gradient: Option<(f32, Vec<GradientStop>)>,
    pub background_repeat: BackgroundRepeat,
    pub background_clip: BackgroundClip,
    pub background_size: BackgroundSize,
    pub background_position: (Length, Length),
    pub background_fixed: bool,

    // Lists, tables, misc.
    pub list_style_type: ListStyleType,
    pub list_style_position: ListStylePosition,
    pub border_collapse: bool,
    pub border_spacing: f32,
    pub table_layout: TableLayout,
    pub visibility: Visibility,
    pub cursor: Cursor,
    pub content: Option<String>,
    pub transform: Option<Transform2D>,
    pub transform_origin: (Length, Length),
    pub box_shadows: Vec<BoxShadow>,
    pub custom_properties: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct StyleCtx {
    pub font_size: f32,
    pub root_font_size: f32,
    pub viewport: Size,
    pub current_color: Color,
    pub base_url: Url,
}

impl ComputedStyle {
    pub fn initial() -> ComputedStyle {
        ComputedStyle {
            display: Display::Inline,
            position: Position::Static,
            float: Float::None,
            clear: Clear::None,
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            max_width: Length::Auto,
            min_height: Length::Auto,
            max_height: Length::Auto,
            margin: EdgesLength::zero(),
            padding: EdgesLength::zero(),
            border_width: EdgesLength::zero(),
            border_style: [BorderStyle::None; 4],
            border_color: [Color::BLACK; 4],
            border_radius: 0.0,
            box_sizing: BoxSizing::ContentBox,
            top: Length::Auto,
            right: Length::Auto,
            bottom: Length::Auto,
            left: Length::Auto,
            z_index: 0,
            opacity: 1.0,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: Align::Start,
            align_items: Align::Stretch,
            align_content: Align::Stretch,
            align_self: Align::Auto,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Length::Auto,
            order: 0,
            row_gap: Length::ZERO,
            column_gap: Length::ZERO,
            grid_columns: Vec::new(),
            grid_rows: Vec::new(),
            grid_auto_flow: GridAutoFlow::Row,
            grid_column_start: GridPlacement::Auto,
            grid_column_end: GridPlacement::Auto,
            grid_row_start: GridPlacement::Auto,
            grid_row_end: GridPlacement::Auto,
            grid_areas: Vec::new(),
            grid_area: None,
            color: Color::rgb(0, 0, 0),
            font_family: "DejaVu Sans".into(),
            font_size: 16.0,
            font_weight: 400,
            font_style: FontStyle::Normal,
            line_height: 19.2,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_align: TextAlign::Start,
            text_decoration: TextDecoration::None,
            text_decoration_color: None,
            text_transform: TextTransform::None,
            text_indent: 0.0,
            white_space: WhiteSpace::Normal,
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            direction: Direction::Ltr,
            vertical_align: VerticalAlign::Baseline,
            background_color: Color::TRANSPARENT,
            background_image: None,
            background_gradient: None,
            background_repeat: BackgroundRepeat::Repeat,
            background_clip: BackgroundClip::BorderBox,
            background_size: BackgroundSize::Auto,
            background_position: (Length::percent(0.0), Length::percent(0.0)),
            background_fixed: false,
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            border_collapse: false,
            border_spacing: 0.0,
            table_layout: TableLayout::Auto,
            visibility: Visibility::Visible,
            cursor: Cursor::Default,
            content: None,
            transform: None,
            transform_origin: (Length::percent(50.0), Length::percent(50.0)),
            box_shadows: Vec::new(),
            custom_properties: HashMap::new(),
        }
    }

    /// Builds the starting style for an element: inherited properties from
    /// the parent, everything else at its initial value.
    pub fn inherit_base(parent: &ComputedStyle) -> ComputedStyle {
        let mut s = ComputedStyle::initial();
        s.color = parent.color;
        s.font_family = parent.font_family.clone();
        s.font_size = parent.font_size;
        s.font_weight = parent.font_weight;
        s.font_style = parent.font_style;
        s.line_height = parent.line_height;
        s.letter_spacing = parent.letter_spacing;
        s.word_spacing = parent.word_spacing;
        s.text_align = parent.text_align;
        s.text_transform = parent.text_transform;
        s.text_indent = parent.text_indent;
        s.white_space = parent.white_space;
        s.word_break = parent.word_break;
        s.overflow_wrap = parent.overflow_wrap;
        s.direction = parent.direction;
        s.visibility = parent.visibility;
        s.cursor = parent.cursor;
        s.list_style_type = parent.list_style_type;
        s.list_style_position = parent.list_style_position;
        s.border_collapse = parent.border_collapse;
        s.border_spacing = parent.border_spacing;
        s.custom_properties = parent.custom_properties.clone();
        s
    }

    pub fn border_edges_width(&self) -> Edges {
        let get = |i: usize| {
            if self.border_style[i].visible() {
                self.border_width.get(i)
            } else {
                0.0
            }
        };
        Edges::new(get(0), get(1), get(2), get(3))
    }

    pub fn is_visible(&self) -> bool {
        self.display != Display::None && self.visibility != Visibility::Hidden && self.opacity > 0.0
    }

    pub fn clips_children(&self) -> bool {
        self.overflow_x.clips() || self.overflow_y.clips()
    }

    pub fn font_size_px(&self) -> f32 {
        self.font_size.max(1.0)
    }

    /// Applies one declaration. Returns true when the property is supported.
    /// `font-size` must be applied before other properties (handled by the
    /// cascade) so that relative units resolve correctly.
    pub fn apply(&mut self, name: &str, value: &str, ctx: &StyleCtx) -> bool {
        let v = value.trim();
        if v.is_empty() {
            return true;
        }
        if let Some(custom) = name.strip_prefix("--") {
            self.custom_properties.insert(format!("--{custom}"), v.to_string());
            return true;
        }
        let len = |text: &str| parse_length(text, self.font_size, ctx.root_font_size, ctx.viewport);
        let col = |text: &str| parse_color_value(text, self.color);
        let vp = ctx.viewport;
        match name {
            "display" => {
                if let Some(d) = Display::parse_multi(&v.split_whitespace().collect::<Vec<_>>()) {
                    self.display = d;
                }
            }
            "position" => {
                if let Some(p) = Position::parse(v) {
                    self.position = p;
                }
            }
            "float" => {
                if let Some(f) = Float::parse(v) {
                    self.float = f;
                }
            }
            "clear" => {
                if let Some(c) = Clear::parse(v) {
                    self.clear = c;
                }
            }
            "box-sizing" => match v {
                "border-box" => self.box_sizing = BoxSizing::BorderBox,
                "content-box" => self.box_sizing = BoxSizing::ContentBox,
                _ => {}
            },
            "width" | "height" => {
                if let Some(l) = len(v) {
                    if name == "width" {
                        self.width = l;
                    } else {
                        self.height = l;
                    }
                }
            }
            "min-width" => {
                if let Some(l) = len(v) {
                    self.min_width = l;
                }
            }
            "max-width" => {
                if v == "none" {
                    self.max_width = Length::Auto;
                } else if let Some(l) = len(v) {
                    self.max_width = l;
                }
            }
            "min-height" => {
                if let Some(l) = len(v) {
                    self.min_height = l;
                }
            }
            "max-height" => {
                if v == "none" {
                    self.max_height = Length::Auto;
                } else if let Some(l) = len(v) {
                    self.max_height = l;
                }
            }
            "margin" => {
                if let Some(e) = parse_four_lengths(v, self.font_size, ctx.root_font_size, vp) {
                    self.margin = e;
                }
            }
            "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => {
                if let Some(l) = len(v) {
                    match name {
                        "margin-top" => self.margin.top = l,
                        "margin-right" => self.margin.right = l,
                        "margin-bottom" => self.margin.bottom = l,
                        _ => self.margin.left = l,
                    }
                }
            }
            "padding" => {
                if let Some(e) = parse_four_lengths(v, self.font_size, ctx.root_font_size, vp) {
                    self.padding = e;
                }
            }
            "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => {
                if let Some(l) = len(v) {
                    match name {
                        "padding-top" => self.padding.top = l,
                        "padding-right" => self.padding.right = l,
                        "padding-bottom" => self.padding.bottom = l,
                        _ => self.padding.left = l,
                    }
                }
            }
            "border-width" => {
                if let Some(e) = parse_four_lengths(v, self.font_size, ctx.root_font_size, vp) {
                    self.border_width = e;
                }
            }
            "border-style" => {
                let parts: Vec<&str> = v.split_whitespace().collect();
                if !parts.is_empty() {
                    let styles: Vec<BorderStyle> = parts.iter().filter_map(|p| BorderStyle::parse(p)).collect();
                    if !styles.is_empty() {
                        let (t, r, b, l) = match styles.len() {
                            1 => (styles[0], styles[0], styles[0], styles[0]),
                            2 => (styles[0], styles[1], styles[0], styles[1]),
                            3 => (styles[0], styles[1], styles[2], styles[1]),
                            _ => (styles[0], styles[1], styles[2], styles[3.min(styles.len() - 1)]),
                        };
                        self.border_style = [t, r, b, l];
                    }
                }
            }
            "border-color" => {
                let parts: Vec<&str> = v.split_whitespace().collect();
                let colors: Vec<Color> = parts.iter().filter_map(|p| col(p)).collect();
                if !colors.is_empty() {
                    let (t, r, b, l) = match colors.len() {
                        1 => (colors[0], colors[0], colors[0], colors[0]),
                        2 => (colors[0], colors[1], colors[0], colors[1]),
                        3 => (colors[0], colors[1], colors[2], colors[1]),
                        _ => (colors[0], colors[1], colors[2], colors[colors.len() - 1]),
                    };
                    self.border_color = [t, r, b, l];
                }
            }
            "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
                let sides: Vec<usize> = match name {
                    "border-top" => vec![0],
                    "border-right" => vec![1],
                    "border-bottom" => vec![2],
                    "border-left" => vec![3],
                    _ => vec![0, 1, 2, 3],
                };
                let mut width = None;
                let mut style = None;
                let mut color = None;
                for token in split_tokens(v) {
                    if let Some(l) = parse_length(&token, self.font_size, ctx.root_font_size, vp) {
                        width = Some(l);
                    } else if let Some(s) = BorderStyle::parse(&token) {
                        style = Some(s);
                    } else if let Some(c) = col(&token) {
                        color = Some(c);
                    }
                }
                for side in sides {
                    if let Some(w) = width {
                        set_edge(&mut self.border_width, side, w);
                    }
                    if let Some(s) = style {
                        self.border_style[side] = s;
                    }
                    if let Some(c) = color {
                        self.border_color[side] = c;
                    }
                    if width.is_none() && style.is_some() && style != Some(BorderStyle::None) {
                        set_edge(&mut self.border_width, side, Length::Px(3.0));
                    }
                }
            }
            "border-radius" => {
                if let Some(l) = parse_length(v.split('/').next().unwrap_or(v).trim(), self.font_size, ctx.root_font_size, vp) {
                    self.border_radius = l.resolve_or(vp.w.min(vp.h), 0.0);
                }
            }
            "inset" | "top" | "right" | "bottom" | "left" => {
                if name == "inset" {
                    if let Some(e) = parse_four_lengths(v, self.font_size, ctx.root_font_size, vp) {
                        self.top = e.top;
                        self.right = e.right;
                        self.bottom = e.bottom;
                        self.left = e.left;
                    }
                } else if let Some(l) = len(v) {
                    match name {
                        "top" => self.top = l,
                        "right" => self.right = l,
                        "bottom" => self.bottom = l,
                        _ => self.left = l,
                    }
                }
            }
            "z-index" => {
                self.z_index = if v == "auto" {
                    0
                } else {
                    v.parse::<i32>().unwrap_or(0)
                }
            }
            "opacity" => {
                self.opacity = v.parse::<f32>().map(|n| n.clamp(0.0, 1.0)).unwrap_or(1.0);
            }
            "overflow" | "overflow-x" | "overflow-y" => {
                let parts: Vec<&str> = v.split_whitespace().collect();
                let x = parts.first().and_then(|p| Overflow::parse(p));
                let y = parts.get(1).and_then(|p| Overflow::parse(p)).or(x);
                if name != "overflow-y" {
                    if let Some(x) = x {
                        self.overflow_x = x;
                    }
                }
                if name != "overflow-x" {
                    if let Some(y) = y {
                        self.overflow_y = y;
                    }
                }
            }
            "visibility" => {
                if let Some(vis) = Visibility::parse(v) {
                    self.visibility = vis;
                }
            }
            "color" => {
                if let Some(c) = col(v) {
                    self.color = c;
                }
            }
            "background-color" => {
                if let Some(c) = col(v) {
                    self.background_color = c;
                }
            }
            "background-image" => {
                self.background_image = None;
                self.background_gradient = None;
                if v != "none" {
                    if let Some(url) = crate::parser::parse_url_value(v, &ctx.base_url) {
                        self.background_image = Some(url);
                    } else if let Some(g) = parse_linear_gradient(v) {
                        self.background_gradient = Some((g.0, g.1));
                    }
                }
            }
            "background-repeat" => {
                self.background_repeat = match v {
                    "no-repeat" => BackgroundRepeat::NoRepeat,
                    "repeat-x" => BackgroundRepeat::RepeatX,
                    "repeat-y" => BackgroundRepeat::RepeatY,
                    _ => BackgroundRepeat::Repeat,
                }
            }
            "background-clip" => {
                self.background_clip = match v {
                    "text" => BackgroundClip::Text,
                    "padding-box" => BackgroundClip::PaddingBox,
                    "content-box" => BackgroundClip::ContentBox,
                    _ => BackgroundClip::BorderBox,
                };
            }
            "background-size" => {
                self.background_size = match v {
                    "cover" => BackgroundSize::Cover,
                    "contain" => BackgroundSize::Contain,
                    "auto" => BackgroundSize::Auto,
                    _ => {
                        let parts: Vec<&str> = v.split_whitespace().collect();
                        match (
                            parts.first().and_then(|p| len(p)),
                            parts.get(1).and_then(|p| len(p)),
                        ) {
                            (Some(a), Some(b)) => BackgroundSize::Explicit(a, b),
                            (Some(a), None) => BackgroundSize::Explicit(a, Length::Auto),
                            _ => BackgroundSize::Auto,
                        }
                    }
                }
            }
            "background-position" => {
                let parts: Vec<&str> = v.split_whitespace().collect();
                let mut x = Length::percent(0.0);
                let mut y = Length::percent(0.0);
                for (i, p) in parts.iter().enumerate() {
                    let l = match *p {
                        "left" => Some(Length::percent(0.0)),
                        "center" => Some(Length::percent(50.0)),
                        "right" => Some(Length::percent(100.0)),
                        "top" => None,
                        "bottom" => None,
                        other => len(other),
                    };
                    if let Some(l) = l {
                        if i == 0 {
                            x = l;
                        } else {
                            y = l;
                        }
                    }
                }
                if parts.first() == Some(&"top") {
                    y = Length::percent(0.0);
                    if parts.get(1) == Some(&"bottom") {
                        y = Length::percent(100.0);
                    }
                }
                if parts.first() == Some(&"bottom") {
                    y = Length::percent(100.0);
                }
                self.background_position = (x, y);
            }
            "background" => {
                self.background_image = None;
                self.background_gradient = None;
                self.background_color = Color::TRANSPARENT;
                for token in split_tokens(v) {
                    if let Some(c) = col(&token) {
                        self.background_color = c;
                    } else if let Some(url) = crate::parser::parse_url_value(&token, &ctx.base_url) {
                        self.background_image = Some(url);
                    } else if let Some(g) = parse_linear_gradient(&token) {
                        self.background_gradient = Some((g.0, g.1));
                    } else if token == "no-repeat" {
                        self.background_repeat = BackgroundRepeat::NoRepeat;
                    }
                }
            }
            "background-attachment" => self.background_fixed = v == "fixed",
            "font-size" => {
                let px = match v {
                    "xx-small" => Some(9.0),
                    "x-small" => Some(10.0),
                    "small" => Some(13.0),
                    "medium" => Some(16.0),
                    "large" => Some(18.0),
                    "x-large" => Some(24.0),
                    "xx-large" => Some(32.0),
                    "smaller" => Some(ctx.font_size / 1.2),
                    "larger" => Some(ctx.font_size * 1.2),
                    _ => None,
                };
                let size = px.or_else(|| {
                    parse_length(v, ctx.font_size, ctx.root_font_size, vp)
                        .and_then(|l| l.resolve(ctx.font_size))
                });
                if let Some(s) = size {
                    self.font_size = s.clamp(1.0, 400.0);
                }
            }
            "font-family" => {
                let families: Vec<String> = split_commas(v).iter().map(|f| strip_quotes(f)).collect();
                if !families.is_empty() {
                    self.font_family = families.join(", ");
                }
            }
            "font-weight" => {
                self.font_weight = match v {
                    "normal" | "400" => 400,
                    "bold" | "700" => 700,
                    "bolder" => 700,
                    "lighter" => 300,
                    other => other.parse::<u16>().unwrap_or(400).clamp(100, 900),
                };
            }
            "font-style" => {
                self.font_style = match v {
                    "italic" => FontStyle::Italic,
                    "oblique" => FontStyle::Oblique,
                    _ => FontStyle::Normal,
                };
            }
            "line-height" => {
                let lh = match v {
                    "normal" => Some(self.font_size * 1.2),
                    _ => {
                        if let Ok(n) = v.parse::<f32>() {
                            Some(n * self.font_size)
                        } else {
                            parse_length(v, self.font_size, ctx.root_font_size, vp)
                                .and_then(|l| l.resolve(self.font_size))
                        }
                    }
                };
                if let Some(lh) = lh {
                    self.line_height = lh.max(1.0);
                }
            }
            "letter-spacing" => {
                if v == "normal" {
                    self.letter_spacing = 0.0;
                } else if let Some(l) = parse_length(v, self.font_size, ctx.root_font_size, vp) {
                    self.letter_spacing = l.resolve(self.font_size).unwrap_or(0.0);
                }
            }
            "word-spacing" => {
                if v == "normal" {
                    self.word_spacing = 0.0;
                } else if let Some(l) = parse_length(v, self.font_size, ctx.root_font_size, vp) {
                    self.word_spacing = l.resolve(self.font_size).unwrap_or(0.0);
                }
            }
            "text-align" => {
                if let Some(t) = TextAlign::parse(v) {
                    self.text_align = t;
                }
            }
            "text-decoration" | "text-decoration-line" => {
                self.text_decoration = if v.contains("underline") {
                    TextDecoration::Underline
                } else if v.contains("line-through") {
                    TextDecoration::LineThrough
                } else if v.contains("overline") {
                    TextDecoration::Overline
                } else {
                    TextDecoration::None
                };
            }
            "text-decoration-color" => self.text_decoration_color = col(v),
            "text-transform" => {
                self.text_transform = match v {
                    "uppercase" => TextTransform::Uppercase,
                    "lowercase" => TextTransform::Lowercase,
                    "capitalize" => TextTransform::Capitalize,
                    _ => TextTransform::None,
                }
            }
            "text-indent" => {
                if let Some(l) = len(v) {
                    self.text_indent = l.resolve(vp.w).unwrap_or(0.0);
                }
            }
            "white-space" => {
                if let Some(w) = WhiteSpace::parse(v) {
                    self.white_space = w;
                }
            }
            "word-break" => {
                self.word_break = match v {
                    "break-all" => WordBreak::BreakAll,
                    "keep-all" => WordBreak::KeepAll,
                    "break-word" => WordBreak::BreakWord,
                    _ => WordBreak::Normal,
                }
            }
            "overflow-wrap" | "word-wrap" => {
                self.overflow_wrap = match v {
                    "break-word" => OverflowWrap::BreakWord,
                    "anywhere" => OverflowWrap::Anywhere,
                    _ => OverflowWrap::Normal,
                }
            }
            "direction" => {
                self.direction = if v == "rtl" { Direction::Rtl } else { Direction::Ltr }
            }
            "vertical-align" => {
                self.vertical_align = match v {
                    "top" => VerticalAlign::Top,
                    "middle" => VerticalAlign::Middle,
                    "bottom" => VerticalAlign::Bottom,
                    "text-top" => VerticalAlign::TextTop,
                    "text-bottom" => VerticalAlign::TextBottom,
                    "sub" => VerticalAlign::Sub,
                    "super" => VerticalAlign::Super,
                    "baseline" => VerticalAlign::Baseline,
                    other => parse_length(other, self.font_size, ctx.root_font_size, vp)
                        .and_then(|l| l.resolve(self.font_size))
                        .map(VerticalAlign::Length)
                        .unwrap_or(VerticalAlign::Baseline),
                }
            }
            "list-style-type" => {
                if let Some(l) = ListStyleType::parse(v) {
                    self.list_style_type = l;
                }
            }
            "list-style-position" => {
                self.list_style_position = if v == "inside" {
                    ListStylePosition::Inside
                } else {
                    ListStylePosition::Outside
                }
            }
            "list-style" => {
                for token in split_tokens(v) {
                    if let Some(l) = ListStyleType::parse(&token) {
                        self.list_style_type = l;
                    } else if token == "inside" {
                        self.list_style_position = ListStylePosition::Inside;
                    } else if token == "outside" {
                        self.list_style_position = ListStylePosition::Outside;
                    }
                }
            }
            "cursor" => {
                self.cursor = match v {
                    "pointer" => Cursor::Pointer,
                    "text" => Cursor::Text,
                    "move" => Cursor::Move,
                    "not-allowed" => Cursor::NotAllowed,
                    "crosshair" => Cursor::Crosshair,
                    "wait" => Cursor::Wait,
                    "help" => Cursor::Help,
                    "grab" => Cursor::Grab,
                    "grabbing" => Cursor::Grabbing,
                    "zoom-in" => Cursor::ZoomIn,
                    _ => Cursor::Default,
                }
            }
            "border-collapse" => self.border_collapse = v == "collapse",
            "border-spacing" => {
                if let Some(l) = parse_length(v.split_whitespace().next().unwrap_or(v), self.font_size, ctx.root_font_size, vp) {
                    self.border_spacing = l.resolve(self.font_size).unwrap_or(0.0);
                }
            }
            "table-layout" => self.table_layout = if v == "fixed" { TableLayout::Fixed } else { TableLayout::Auto },
            "flex-direction" => {
                if let Some(d) = FlexDirection::parse(v) {
                    self.flex_direction = d;
                }
            }
            "flex-wrap" => {
                if let Some(w) = FlexWrap::parse(v) {
                    self.flex_wrap = w;
                }
            }
            "flex-flow" => {
                for token in split_tokens(v) {
                    if let Some(d) = FlexDirection::parse(&token) {
                        self.flex_direction = d;
                    }
                    if let Some(w) = FlexWrap::parse(&token) {
                        self.flex_wrap = w;
                    }
                }
            }
            "justify-content" | "place-content" => {
                if let Some(a) = Align::parse(v.split_whitespace().next().unwrap_or(v)) {
                    self.justify_content = a;
                }
            }
            "align-items" | "place-items" => {
                if let Some(a) = Align::parse(v.split_whitespace().next().unwrap_or(v)) {
                    self.align_items = a;
                }
            }
            "align-self" => {
                if let Some(a) = Align::parse(v) {
                    self.align_self = a;
                }
            }
            "align-content" => {
                if let Some(a) = Align::parse(v) {
                    self.align_content = a;
                }
            }
            "flex-grow" => self.flex_grow = v.parse::<f32>().unwrap_or(0.0).max(0.0),
            "flex-shrink" => self.flex_shrink = v.parse::<f32>().unwrap_or(1.0).max(0.0),
            "flex-basis" => {
                self.flex_basis = if v == "content" {
                    Length::Auto
                } else {
                    len(v).unwrap_or(Length::Auto)
                }
            }
            "flex" => {
                parse_flex_shorthand(self, v, ctx);
            }
            "order" => self.order = v.parse::<i32>().unwrap_or(0),
            "gap" => {
                let parts: Vec<&str> = v.split_whitespace().collect();
                if let Some(r) = parts.first().and_then(|p| len(p)) {
                    self.row_gap = r;
                }
                if let Some(c) = parts.get(1).and_then(|p| len(p)) {
                    self.column_gap = c;
                } else if let Some(r) = parts.first().and_then(|p| len(p)) {
                    self.column_gap = r;
                }
            }
            "row-gap" => {
                if let Some(l) = len(v) {
                    self.row_gap = l;
                }
            }
            "column-gap" => {
                if let Some(l) = len(v) {
                    self.column_gap = l;
                }
            }
            "grid-template-areas" => {
                self.grid_areas = parse_grid_areas(v);
            }
            "grid-template" => {
                let (rows_part, cols_part) = match v.find('/') {
                    Some(i) => (&v[..i], Some(&v[i + 1..])),
                    None => (v, None),
                };
                self.grid_areas = parse_grid_areas(rows_part);
                let rows = strip_quoted_strings(rows_part);
                if !rows.trim().is_empty() && rows.trim() != "none" {
                    self.grid_rows = parse_grid_tracks(&rows, self.font_size, ctx.root_font_size, vp);
                } else {
                    self.grid_rows = Vec::new();
                }
                if let Some(cols) = cols_part {
                    self.grid_columns = parse_grid_tracks(cols, self.font_size, ctx.root_font_size, vp);
                }
            }
            "grid-area" => {
                let parts: Vec<&str> = v.split('/').map(|p| p.trim()).collect();
                if parts.len() == 1 {
                    let name = parts[0].trim_matches(|c| c == '\'' || c == '"');
                    self.grid_area = if name.is_empty() || name == "auto" {
                        None
                    } else {
                        Some(name.to_string())
                    };
                } else {
                    self.grid_area = None;
                    self.grid_row_start = parts.first().and_then(|p| GridPlacement::parse(p)).unwrap_or(GridPlacement::Auto);
                    self.grid_column_start = parts.get(1).and_then(|p| GridPlacement::parse(p)).unwrap_or(GridPlacement::Auto);
                    self.grid_row_end = parts.get(2).and_then(|p| GridPlacement::parse(p)).unwrap_or(GridPlacement::Auto);
                    self.grid_column_end = parts.get(3).and_then(|p| GridPlacement::parse(p)).unwrap_or(GridPlacement::Auto);
                }
            }
            "grid-template-columns" | "grid-template-rows" => {
                let tracks = parse_grid_tracks(v, self.font_size, ctx.root_font_size, vp);
                if name.ends_with("columns") {
                    self.grid_columns = tracks;
                } else {
                    self.grid_rows = tracks;
                }
            }
            "grid-auto-flow" => {
                self.grid_auto_flow = match v.replace(' ', "").as_str() {
                    "column" => GridAutoFlow::Column,
                    "rowdense" => GridAutoFlow::RowDense,
                    "columndense" => GridAutoFlow::ColumnDense,
                    _ => GridAutoFlow::Row,
                }
            }
            "grid-column" | "grid-row" => {
                let parts: Vec<&str> = v.split('/').collect();
                let start = parts.first().and_then(|p| GridPlacement::parse(p)).unwrap_or(GridPlacement::Auto);
                let end = parts.get(1).and_then(|p| GridPlacement::parse(p)).unwrap_or(GridPlacement::Auto);
                if name == "grid-column" {
                    self.grid_column_start = start;
                    self.grid_column_end = end;
                } else {
                    self.grid_row_start = start;
                    self.grid_row_end = end;
                }
            }
            "grid-column-start" => self.grid_column_start = GridPlacement::parse(v).unwrap_or(GridPlacement::Auto),
            "grid-column-end" => self.grid_column_end = GridPlacement::parse(v).unwrap_or(GridPlacement::Auto),
            "grid-row-start" => self.grid_row_start = GridPlacement::parse(v).unwrap_or(GridPlacement::Auto),
            "grid-row-end" => self.grid_row_end = GridPlacement::parse(v).unwrap_or(GridPlacement::Auto),
            "content" => {
                if v == "none" || v == "normal" {
                    self.content = None;
                } else {
                    self.content = Some(strip_quotes(v.split_whitespace().next().unwrap_or(v)));
                }
            }
            "transform" => {
                self.transform = parse_transform(v, self.font_size, ctx.viewport);
            }
            "transform-origin" => {
                let parts: Vec<&str> = v.split_whitespace().collect();
                let mut origin = (Length::percent(50.0), Length::percent(50.0));
                let mut seen_y = false;
                for p in &parts {
                    let l = match *p {
                        "left" => Some(Length::percent(0.0)),
                        "right" => Some(Length::percent(100.0)),
                        "top" => Some(Length::percent(0.0)),
                        "bottom" => Some(Length::percent(100.0)),
                        "center" => Some(Length::percent(50.0)),
                        other => len(other),
                    };
                    if let Some(l) = l {
                        if !seen_y && !matches!(*p, "top" | "bottom") {
                            origin.0 = l;
                        } else {
                            origin.1 = l;
                            seen_y = true;
                        }
                    }
                }
                if parts.len() == 1 && matches!(parts[0], "top" | "bottom") {
                    origin.1 = if parts[0] == "top" { Length::percent(0.0) } else { Length::percent(100.0) };
                }
                self.transform_origin = origin;
            }
            "box-shadow" => {
                self.box_shadows = parse_shadows(v);
            }
            "font" => parse_font_shorthand(self, v, ctx),
            // Properties we knowingly ignore but do not treat as errors.
            "transition" | "transition-property" | "transition-duration" | "transition-timing-function"
            | "animation" | "animation-name" | "animation-duration" | "transform" | "transform-origin"
            | "box-shadow" | "text-shadow" | "filter" | "backdrop-filter" | "will-change"
            | "user-select" | "-webkit-user-select" | "pointer-events" | "outline" | "outline-offset"
            | "outline-color" | "outline-style" | "outline-width" | "caret-color" | "appearance"
            | "-webkit-appearance" | "resize" | "scroll-behavior" | "touch-action" | "contain"
            | "content-visibility" | "clip-path" | "mask" | "isolation" | "mix-blend-mode"
            | "aspect-ratio" | "object-fit" | "object-position" | "columns" | "column-count"
            | "column-width" | "column-rule" | "white-space-collapse" | "text-wrap" | "text-overflow"
            | "line-clamp" | "-webkit-line-clamp" | "unicode-bidi" | "tab-size" | "hyphens"
            | "font-variant" | "font-feature-settings" | "font-stretch" | "text-rendering"
            | "-webkit-font-smoothing" | "zoom" | "translate" | "rotate" | "scale" | "offset"
            | "inset-inline" | "inset-block" | "inset-inline-start" | "inset-inline-end"
            | "inset-block-start" | "inset-block-end" | "margin-inline" | "margin-block"
            | "margin-inline-start" | "margin-inline-end" | "margin-block-start" | "margin-block-end"
            | "padding-inline" | "padding-block" | "padding-inline-start" | "padding-inline-end"
            | "padding-block-start" | "padding-block-end" | "border-inline" | "border-block"
            | "border-inline-start" | "border-inline-end" | "border-block-start" | "border-block-end"
            | "background-blend-mode" | "background-clip" | "background-origin" | "backface-visibility"
            | "perspective" | "perspective-origin" | "transform-style" | "justify-items"
            | "justify-self" | "grid-auto-columns" | "grid-auto-rows"
            | "grid-gap" | "grid-row-gap" | "grid-column-gap" | "place-self"
            | "scrollbar-width" | "scrollbar-color" | "accent-color" | "color-scheme" | "writing-mode"
            | "text-orientation" | "float-defer" | "shape-outside" | "shape-margin" | "contain-intrinsic-size"
            | "container-type" | "container-name" | "anchor-name" | "position-anchor" => {}
            _ => return false,
        }
        true
    }

    /// Copies a single property from another style (used for `inherit`,
    /// `initial` and `unset`).
    pub fn copy_property_from(&mut self, src: &ComputedStyle, name: &str) {
        match name {
            "display" => self.display = src.display,
            "position" => self.position = src.position,
            "float" => self.float = src.float,
            "clear" => self.clear = src.clear,
            "width" => self.width = src.width,
            "height" => self.height = src.height,
            "min-width" => self.min_width = src.min_width,
            "max-width" => self.max_width = src.max_width,
            "min-height" => self.min_height = src.min_height,
            "max-height" => self.max_height = src.max_height,
            "margin" => self.margin = src.margin,
            "margin-top" => self.margin.top = src.margin.top,
            "margin-right" => self.margin.right = src.margin.right,
            "margin-bottom" => self.margin.bottom = src.margin.bottom,
            "margin-left" => self.margin.left = src.margin.left,
            "padding" => self.padding = src.padding,
            "padding-top" => self.padding.top = src.padding.top,
            "padding-right" => self.padding.right = src.padding.right,
            "padding-bottom" => self.padding.bottom = src.padding.bottom,
            "padding-left" => self.padding.left = src.padding.left,
            "border-width" | "border" => self.border_width = src.border_width,
            "border-style" | "border" => self.border_style = src.border_style,
            "border-color" | "border" => self.border_color = src.border_color,
            "border-radius" => self.border_radius = src.border_radius,
            "border-collapse" => self.border_collapse = src.border_collapse,
            "border-spacing" => self.border_spacing = src.border_spacing,
            "box-sizing" => self.box_sizing = src.box_sizing,
            "top" => self.top = src.top,
            "right" => self.right = src.right,
            "bottom" => self.bottom = src.bottom,
            "left" => self.left = src.left,
            "z-index" => self.z_index = src.z_index,
            "opacity" => self.opacity = src.opacity,
            "overflow" | "overflow-x" => self.overflow_x = src.overflow_x,
            "overflow-y" => self.overflow_y = src.overflow_y,
            "visibility" => self.visibility = src.visibility,
            "color" => self.color = src.color,
            "background-color" => self.background_color = src.background_color,
            "background-image" => {
                self.background_image = src.background_image.clone();
                self.background_gradient = src.background_gradient.clone();
            }
            "font-family" => self.font_family = src.font_family.clone(),
            "font-size" => self.font_size = src.font_size,
            "font-weight" => self.font_weight = src.font_weight,
            "font-style" => self.font_style = src.font_style,
            "line-height" => self.line_height = src.line_height,
            "letter-spacing" => self.letter_spacing = src.letter_spacing,
            "word-spacing" => self.word_spacing = src.word_spacing,
            "text-align" => self.text_align = src.text_align,
            "text-decoration" => self.text_decoration = src.text_decoration,
            "text-decoration-color" => self.text_decoration_color = src.text_decoration_color,
            "text-transform" => self.text_transform = src.text_transform,
            "text-indent" => self.text_indent = src.text_indent,
            "white-space" => self.white_space = src.white_space,
            "word-break" => self.word_break = src.word_break,
            "overflow-wrap" => self.overflow_wrap = src.overflow_wrap,
            "direction" => self.direction = src.direction,
            "vertical-align" => self.vertical_align = src.vertical_align,
            "list-style-type" => self.list_style_type = src.list_style_type,
            "list-style-position" => self.list_style_position = src.list_style_position,
            "flex-direction" => self.flex_direction = src.flex_direction,
            "flex-wrap" => self.flex_wrap = src.flex_wrap,
            "justify-content" => self.justify_content = src.justify_content,
            "align-items" => self.align_items = src.align_items,
            "align-self" => self.align_self = src.align_self,
            "align-content" => self.align_content = src.align_content,
            "flex-grow" => self.flex_grow = src.flex_grow,
            "flex-shrink" => self.flex_shrink = src.flex_shrink,
            "flex-basis" => self.flex_basis = src.flex_basis,
            "order" => self.order = src.order,
            "gap" => {
                self.row_gap = src.row_gap;
                self.column_gap = src.column_gap;
            }
            "row-gap" => self.row_gap = src.row_gap,
            "column-gap" => self.column_gap = src.column_gap,
            "grid-template-columns" => self.grid_columns = src.grid_columns.clone(),
            "grid-template-rows" => self.grid_rows = src.grid_rows.clone(),
            "grid-auto-flow" => self.grid_auto_flow = src.grid_auto_flow,
            "grid-column" => {
                self.grid_column_start = src.grid_column_start;
                self.grid_column_end = src.grid_column_end;
            }
            "grid-row" => {
                self.grid_row_start = src.grid_row_start;
                self.grid_row_end = src.grid_row_end;
            }
            "table-layout" => self.table_layout = src.table_layout,
            "cursor" => self.cursor = src.cursor,
            "content" => self.content = src.content.clone(),
            "transform" => self.transform = src.transform,
            "transform-origin" => self.transform_origin = src.transform_origin,
            "box-shadow" => self.box_shadows = src.box_shadows.clone(),
            other if other.starts_with("--") => {
                if let Some(v) = src.custom_properties.get(other) {
                    self.custom_properties.insert(other.to_string(), v.clone());
                }
            }
            _ => {}
        }
    }
}

fn parse_transform(value: &str, font_size: f32, viewport: Size) -> Option<Transform2D> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return None;
    }
    let mut m = Transform2D::IDENTITY;
    let mut rest = v;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().to_ascii_lowercase();
        let close = match rest[open..].find(')') {
            Some(c) => open + c,
            None => break,
        };
        let args: Vec<String> = rest[open + 1..close]
            .split([',', ' '])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let num = |i: usize| -> f32 {
            args.get(i)
                .map(|s| parse_length(s, font_size, font_size, viewport).and_then(|l| l.resolve(1.0)))
                .flatten()
                .unwrap_or(0.0)
        };
        let angle = |i: usize| -> f32 {
            args.get(i)
                .map(|s| s.trim_end_matches("deg").parse::<f32>().unwrap_or(0.0))
                .unwrap_or(0.0)
        };
        let next = match name.as_str() {
            "translate" => Transform2D::translate(num(0), if args.len() > 1 { num(1) } else { 0.0 }),
            "translatex" => Transform2D::translate(num(0), 0.0),
            "translatey" => Transform2D::translate(0.0, num(0)),
            "scale" => {
                let sx = args.first().map(|s| s.parse::<f32>().unwrap_or(1.0)).unwrap_or(1.0);
                let sy = args.get(1).map(|s| s.parse::<f32>().unwrap_or(sx)).unwrap_or(sx);
                Transform2D::scale(sx, sy)
            }
            "scalex" => Transform2D::scale(args.first().map(|s| s.parse().unwrap_or(1.0)).unwrap_or(1.0), 1.0),
            "scaley" => Transform2D::scale(1.0, args.first().map(|s| s.parse().unwrap_or(1.0)).unwrap_or(1.0)),
            "rotate" | "rotatez" => Transform2D::rotate(angle(0)),
            "skew" => Transform2D::skew(angle(0), if args.len() > 1 { angle(1) } else { 0.0 }),
            "skewx" => Transform2D::skew(angle(0), 0.0),
            "skewy" => Transform2D::skew(0.0, angle(0)),
            "matrix" => {
                if args.len() >= 6 {
                    let p = |i: usize| args[i].parse::<f32>().unwrap_or(0.0);
                    Transform2D { a: p(0), b: p(1), c: p(2), d: p(3), e: p(4), f: p(5) }
                } else {
                    Transform2D::IDENTITY
                }
            }
            _ => Transform2D::IDENTITY,
        };
        m = m.multiply(&next);
        rest = &rest[close + 1..];
    }
    if m.is_identity() {
        None
    } else {
        Some(m)
    }
}

fn parse_shadows(value: &str) -> Vec<BoxShadow> {
    let mut out = Vec::new();
    if value.eq_ignore_ascii_case("none") {
        return out;
    }
    for part in split_commas(value) {
        let tokens = split_tokens(&part);
        if tokens.is_empty() {
            continue;
        }
        let mut lengths = Vec::new();
        let mut color = None;
        let mut inset = false;
        for t in &tokens {
            if t == "inset" {
                inset = true;
            } else if let Some(c) = Color::parse(t) {
                color = Some(c);
            } else if let Some(l) = parse_length(t, 16.0, 16.0, Size::new(1.0, 1.0)) {
                lengths.push(l.resolve(0.0).unwrap_or(0.0));
            }
        }
        if lengths.is_empty() {
            continue;
        }
        let x = lengths[0];
        let y = *lengths.get(1).unwrap_or(&0.0);
        let blur = *lengths.get(2).unwrap_or(&0.0);
        let spread = *lengths.get(3).unwrap_or(&0.0);
        out.push(BoxShadow {
            x,
            y,
            blur,
            spread,
            color: color.unwrap_or(Color::rgba(0, 0, 0, 100)),
            inset,
        });
    }
    out
}

fn set_edge(edges: &mut EdgesLength, side: usize, value: Length) {
    match side {
        0 => edges.top = value,
        1 => edges.right = value,
        2 => edges.bottom = value,
        _ => edges.left = value,
    }
}

impl EdgesLength {
    fn get(&self, side: usize) -> f32 {
        match side {
            0 => self.top.resolve_or(0.0, 0.0),
            1 => self.right.resolve_or(0.0, 0.0),
            2 => self.bottom.resolve_or(0.0, 0.0),
            _ => self.left.resolve_or(0.0, 0.0),
        }
    }
}

fn parse_flex_shorthand(style: &mut ComputedStyle, value: &str, ctx: &StyleCtx) {
    let tokens = split_tokens(value);
    if tokens.is_empty() {
        return;
    }
    if tokens.len() == 1 {
        let t = &tokens[0];
        if t == "none" {
            style.flex_grow = 0.0;
            style.flex_shrink = 0.0;
            style.flex_basis = Length::Auto;
            return;
        }
        if t == "auto" {
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            style.flex_basis = Length::Auto;
            return;
        }
        if let Ok(n) = t.parse::<f32>() {
            style.flex_grow = n;
            style.flex_shrink = 1.0;
            style.flex_basis = Length::Percent(0.0);
            return;
        }
        if let Some(l) = parse_length(t, style.font_size, ctx.root_font_size, ctx.viewport) {
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            style.flex_basis = l;
        }
        return;
    }
    for (i, t) in tokens.iter().enumerate() {
        if i == 0 {
            if let Ok(n) = t.parse::<f32>() {
                style.flex_grow = n;
                continue;
            }
        }
        if i == 1 {
            if let Ok(n) = t.parse::<f32>() {
                style.flex_shrink = n;
                continue;
            }
        }
        if t == "auto" {
            style.flex_basis = Length::Auto;
        } else if t == "content" {
            style.flex_basis = Length::Auto;
        } else if let Some(l) = parse_length(t, style.font_size, ctx.root_font_size, ctx.viewport) {
            style.flex_basis = l;
        }
    }
}

fn parse_font_shorthand(style: &mut ComputedStyle, value: &str, ctx: &StyleCtx) {
    // Very small subset: [style] [weight] size[/line-height] family
    let tokens = split_tokens(value);
    let mut idx = 0;
    while idx < tokens.len() {
        let t = &tokens[idx];
        if matches!(t.as_str(), "italic" | "oblique") {
            style.font_style = FontStyle::Italic;
            idx += 1;
            continue;
        }
        if matches!(t.as_str(), "bold" | "bolder" | "lighter") || t.parse::<u16>().is_ok() {
            style.font_weight = match t.as_str() {
                "bold" => 700,
                "bolder" => 700,
                "lighter" => 300,
                _ => t.parse::<u16>().unwrap_or(400),
            };
            idx += 1;
            continue;
        }
        break;
    }
    if idx >= tokens.len() {
        return;
    }
    let size_token = &tokens[idx];
    let (size_part, line_part) = match size_token.split_once('/') {
        Some((s, l)) => (s, Some(l)),
        None => (size_token.as_str(), None),
    };
    if let Some(l) = parse_length(size_part, ctx.font_size, ctx.root_font_size, ctx.viewport) {
        if let Some(px) = l.resolve(ctx.font_size) {
            style.font_size = px;
        }
    }
    if let Some(lp) = line_part {
        if let Ok(n) = lp.parse::<f32>() {
            style.line_height = n * style.font_size;
        }
    }
    idx += 1;
    if idx < tokens.len() {
        let family: Vec<String> = tokens[idx..].iter().map(|f| strip_quotes(f)).collect();
        style.font_family = family.join(", ");
    }
}

/// Parses `grid-template-areas` row strings into rows of cell names.
/// `.` and `...` cells become empty strings.
fn parse_grid_areas(value: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut rest = value.trim();
    while let Some(start) = rest.find(['\'', '"']) {
        let quote = rest.as_bytes()[start] as char;
        let Some(rel_end) = rest[start + 1..].find(quote) else {
            break;
        };
        let row = &rest[start + 1..start + 1 + rel_end];
        let cells = row
            .split_whitespace()
            .map(|cell| {
                if cell.chars().all(|c| c == '.') {
                    String::new()
                } else {
                    cell.to_string()
                }
            })
            .collect::<Vec<_>>();
        if !cells.is_empty() {
            rows.push(cells);
        }
        rest = &rest[start + 1 + rel_end + 1..];
    }
    rows
}

/// Removes quoted strings (area names) from a `grid-template` value.
fn strip_quoted_strings(value: &str) -> String {
    let mut out = String::new();
    let mut quote = None;
    for c in value.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '\'' || c == '"' {
                    quote = Some(c);
                } else {
                    out.push(c);
                }
            }
        }
    }
    out
}

pub fn parse_grid_tracks(value: &str, font_size: f32, root_size: f32, viewport: Size) -> Vec<GridTrack> {
    let mut out = Vec::new();
    let tokens = split_tokens(value);
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if t == "none" {
            return Vec::new();
        }
        if let Some(rest) = t.strip_prefix("repeat(") {
            let rest = rest.trim_end_matches(')');
            let mut parts = rest.splitn(2, ',');
            let count_str = parts.next().unwrap_or("1").trim();
            let track_str = parts.next().unwrap_or("1fr");
            let count = if count_str == "auto-fill" || count_str == "auto-fit" {
                1
            } else {
                count_str.parse::<usize>().unwrap_or(1).min(24)
            };
            let inner = parse_grid_tracks(track_str, font_size, root_size, viewport);
            for _ in 0..count {
                out.extend(inner.iter().copied());
            }
            i += 1;
            continue;
        }
        if t.starts_with("[") || t.ends_with("]") {
            // Named grid lines `[content-start]` are labels, not tracks.
            i += 1;
            continue;
        }
        if t.starts_with("minmax(") {
            let inner = t.trim_start_matches("minmax(").trim_end_matches(')');
            let mut parts = inner.splitn(2, ',');
            let a = parse_track(parts.next().unwrap_or("0"), font_size, root_size, viewport);
            let b = parse_track(parts.next().unwrap_or("auto"), font_size, root_size, viewport);
            let min_px = match a {
                GridTrack::Px(v) => v,
                GridTrack::Percent(p) => viewport.w * p / 100.0,
                _ => 0.0,
            };
            out.push(match b {
                GridTrack::Px(v) => GridTrack::MinMax(min_px, v.max(min_px)),
                GridTrack::Percent(p) => {
                    let max = (viewport.w * p / 100.0).max(min_px);
                    GridTrack::MinMax(min_px, max)
                }
                // `minmax(0, 1fr)` has no fixed minimum to enforce; keep it
                // flexible. A non-zero minimum (`minmax(15rem, 1fr)`) needs
                // its floor preserved, which an unbounded MinMax expresses.
                GridTrack::Fr(f) => {
                    if min_px > 0.0 {
                        GridTrack::MinMax(min_px, f32::INFINITY)
                    } else {
                        GridTrack::Fr(f)
                    }
                }
                GridTrack::Auto | GridTrack::MinContent | GridTrack::MaxContent => {
                    GridTrack::MinMax(min_px, min_px)
                }
                GridTrack::MinMax(_, m) => GridTrack::MinMax(min_px, m.max(min_px)),
            });
        } else {
            out.push(parse_track(t, font_size, root_size, viewport));
        }
        i += 1;
    }
    out
}

fn parse_track(token: &str, font_size: f32, root_size: f32, viewport: Size) -> GridTrack {
    let t = token.trim();
    if t == "auto" {
        return GridTrack::Auto;
    }
    if t == "min-content" {
        return GridTrack::MinContent;
    }
    if t == "max-content" {
        return GridTrack::MaxContent;
    }
    if let Some(fr) = t.strip_suffix("fr") {
        return GridTrack::Fr(fr.trim().parse::<f32>().unwrap_or(1.0).max(0.0));
    }
    match parse_length(t, font_size, root_size, viewport) {
        Some(Length::Px(v)) => GridTrack::Px(v),
        Some(Length::Percent(p)) => GridTrack::Percent(p),
        Some(Length::Calc { px, pct }) => {
            if px != 0.0 {
                GridTrack::Px(px)
            } else {
                GridTrack::Percent(pct)
            }
        }
        _ => GridTrack::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> StyleCtx {
        StyleCtx {
            font_size: 16.0,
            root_font_size: 16.0,
            viewport: Size::new(1000.0, 800.0),
            current_color: Color::BLACK,
            base_url: Url::parse("about:blank").unwrap(),
        }
    }

    #[test]
    fn applies_box_and_text() {
        let mut s = ComputedStyle::initial();
        let c = ctx();
        assert!(s.apply("display", "block", &c));
        assert!(s.apply("margin", "10px 20px", &c));
        assert_eq!(s.margin.top, Length::Px(10.0));
        assert_eq!(s.margin.right, Length::Px(20.0));
        assert!(s.apply("font-size", "20px", &c));
        assert!(s.apply("line-height", "1.5", &c));
        assert_eq!(s.line_height, 30.0);
        assert!(s.apply("color", "#ff0000", &c));
        assert_eq!(s.color, Color::rgb(255, 0, 0));
        assert!(s.apply("border", "2px solid blue", &c));
        assert_eq!(s.border_style[0], BorderStyle::Solid);
        assert_eq!(s.border_width.top, Length::Px(2.0));
        assert_eq!(s.border_color[0], Color::rgb(0, 0, 255));
        assert!(s.apply("width", "50%", &c));
        assert_eq!(s.width, Length::Percent(50.0));
    }

    #[test]
    fn flex_and_grid_values() {
        let mut s = ComputedStyle::initial();
        let c = ctx();
        s.apply("display", "flex", &c);
        assert_eq!(s.display, Display::Flex);
        s.apply("flex", "1 0 200px", &c);
        assert_eq!(s.flex_grow, 1.0);
        assert_eq!(s.flex_basis, Length::Px(200.0));
        s.apply("grid-template-columns", "repeat(3, 1fr)", &c);
        assert_eq!(s.grid_columns.len(), 3);
        assert_eq!(s.grid_columns[0], GridTrack::Fr(1.0));
        s.apply("justify-content", "space-between", &c);
        assert_eq!(s.justify_content, Align::SpaceBetween);
    }

    #[test]
    fn inheritance() {
        let mut parent = ComputedStyle::initial();
        parent.color = Color::rgb(10, 20, 30);
        parent.font_size = 24.0;
        parent.display = Display::Block;
        let child = ComputedStyle::inherit_base(&parent);
        assert_eq!(child.color, parent.color);
        assert_eq!(child.font_size, 24.0);
        assert_eq!(child.display, Display::Inline);
    }
}
