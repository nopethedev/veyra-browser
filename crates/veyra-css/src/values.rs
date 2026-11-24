//! CSS value types shared by the parser, cascade and layout.

use veyra_common::{Color, Size};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    Auto,
    Px(f32),
    Percent(f32),
    /// A reduced `calc()` of the form `px + pct%`.
    Calc { px: f32, pct: f32 },
}

impl Default for Length {
    fn default() -> Self {
        Length::Auto
    }
}

impl Length {
    pub const ZERO: Length = Length::Px(0.0);
    pub fn px(v: f32) -> Length {
        Length::Px(if v.is_finite() { v } else { 0.0 })
    }
    pub fn percent(v: f32) -> Length {
        Length::Percent(v)
    }
    pub fn is_auto(&self) -> bool {
        matches!(self, Length::Auto)
    }
    /// Resolves against a percentage basis. `None` for `auto`.
    pub fn resolve(&self, base: f32) -> Option<f32> {
        match *self {
            Length::Auto => None,
            Length::Px(v) => Some(v),
            Length::Percent(p) => Some(p * base / 100.0),
            Length::Calc { px, pct } => Some(px + pct * base / 100.0),
        }
    }
    pub fn resolve_or(&self, base: f32, fallback: f32) -> f32 {
        self.resolve(base).unwrap_or(fallback)
    }
    pub fn is_zero(&self) -> bool {
        matches!(self, Length::Px(v) if *v == 0.0) || matches!(self, Length::Percent(v) if *v == 0.0)
    }
    pub fn scale(&self, factor: f32) -> Length {
        match *self {
            Length::Auto => Length::Auto,
            Length::Px(v) => Length::Px(v * factor),
            Length::Percent(p) => Length::Percent(p),
            Length::Calc { px, pct } => Length::Calc { px: px * factor, pct },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgesLength {
    pub top: Length,
    pub right: Length,
    pub bottom: Length,
    pub left: Length,
}

impl EdgesLength {
    pub fn all(l: Length) -> Self {
        EdgesLength { top: l, right: l, bottom: l, left: l }
    }
    pub fn zero() -> Self {
        EdgesLength::all(Length::ZERO)
    }
    pub fn is_zero(&self) -> bool {
        self.top.is_zero() && self.right.is_zero() && self.bottom.is_zero() && self.left.is_zero()
    }
    pub fn horizontal(&self, base: f32) -> f32 {
        self.left.resolve_or(base, 0.0) + self.right.resolve_or(base, 0.0)
    }
    pub fn vertical(&self, base: f32) -> f32 {
        self.top.resolve_or(base, 0.0) + self.bottom.resolve_or(base, 0.0)
    }
    pub fn to_edges(&self, base: f32) -> veyra_common::Edges {
        veyra_common::Edges {
            top: self.top.resolve_or(base, 0.0),
            right: self.right.resolve_or(base, 0.0),
            bottom: self.bottom.resolve_or(base, 0.0),
            left: self.left.resolve_or(base, 0.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    None,
    Block,
    Inline,
    InlineBlock,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    ListItem,
    Table,
    InlineTable,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
    TableColumn,
    TableColumnGroup,
    TableCaption,
    Contents,
}

impl Display {
    pub fn parse(s: &str) -> Option<Display> {
        Some(match s {
            "none" => Display::None,
            "block" | "flow-root" | "flow" => Display::Block,
            "inline" => Display::Inline,
            "inline-block" => Display::InlineBlock,
            "flex" => Display::Flex,
            "inline-flex" => Display::InlineFlex,
            "grid" => Display::Grid,
            "inline-grid" => Display::InlineGrid,
            "list-item" => Display::ListItem,
            "table" => Display::Table,
            "inline-table" => Display::InlineTable,
            "table-row-group" => Display::TableRowGroup,
            "table-header-group" => Display::TableHeaderGroup,
            "table-footer-group" => Display::TableFooterGroup,
            "table-row" => Display::TableRow,
            "table-cell" => Display::TableCell,
            "table-column" => Display::TableColumn,
            "table-column-group" => Display::TableColumnGroup,
            "table-caption" => Display::TableCaption,
            "contents" => Display::Contents,
            _ => return None,
        })
    }
    /// Multi-keyword `display: inline flex` support.
    pub fn parse_multi(words: &[&str]) -> Option<Display> {
        if words.is_empty() {
            return None;
        }
        if words.len() == 1 {
            return Display::parse(words[0]);
        }
        let outer = words[0];
        let inner = words[1];
        let inline = outer == "inline";
        match inner {
            "flex" => Some(if inline { Display::InlineFlex } else { Display::Flex }),
            "grid" => Some(if inline { Display::InlineGrid } else { Display::Grid }),
            "flow-root" | "block" | "flow" => Some(if inline {
                Display::InlineBlock
            } else {
                Display::Block
            }),
            "table" => Some(if inline { Display::InlineTable } else { Display::Table }),
            _ => Display::parse(words[0]),
        }
    }
    pub fn is_block_level(&self) -> bool {
        matches!(
            self,
            Display::Block
                | Display::Flex
                | Display::Grid
                | Display::ListItem
                | Display::Table
                | Display::TableRowGroup
                | Display::TableHeaderGroup
                | Display::TableFooterGroup
                | Display::TableRow
                | Display::TableCell
                | Display::TableCaption
        )
    }
    pub fn is_inline_level(&self) -> bool {
        matches!(
            self,
            Display::Inline
                | Display::InlineBlock
                | Display::InlineFlex
                | Display::InlineGrid
                | Display::InlineTable
        )
    }
    pub fn generates_box(&self) -> bool {
        !matches!(self, Display::None | Display::Contents)
    }
    pub fn is_flex_or_grid(&self) -> bool {
        matches!(self, Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

impl Position {
    pub fn parse(s: &str) -> Option<Position> {
        Some(match s {
            "static" => Position::Static,
            "relative" => Position::Relative,
            "absolute" => Position::Absolute,
            "fixed" => Position::Fixed,
            "sticky" => Position::Sticky,
            _ => return None,
        })
    }
    pub fn is_out_of_flow(&self) -> bool {
        matches!(self, Position::Absolute | Position::Fixed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Float {
    None,
    Left,
    Right,
}

impl Float {
    pub fn parse(s: &str) -> Option<Float> {
        Some(match s {
            "none" => Float::None,
            "left" => Float::Left,
            "right" => Float::Right,
            "inline-start" => Float::Left,
            "inline-end" => Float::Right,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clear {
    None,
    Left,
    Right,
    Both,
}

impl Clear {
    pub fn parse(s: &str) -> Option<Clear> {
        Some(match s {
            "none" => Clear::None,
            "left" => Clear::Left,
            "right" => Clear::Right,
            "both" => Clear::Both,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
    Justify,
    Start,
    End,
}

impl TextAlign {
    pub fn parse(s: &str) -> Option<TextAlign> {
        Some(match s {
            "left" | "-webkit-left" => TextAlign::Left,
            "right" | "-webkit-right" => TextAlign::Right,
            "center" | "-webkit-center" => TextAlign::Center,
            "justify" => TextAlign::Justify,
            "start" => TextAlign::Start,
            "end" => TextAlign::End,
            _ => return None,
        })
    }
    pub fn resolve(self, rtl: bool) -> TextAlign {
        match self {
            TextAlign::Start => {
                if rtl {
                    TextAlign::Right
                } else {
                    TextAlign::Left
                }
            }
            TextAlign::End => {
                if rtl {
                    TextAlign::Left
                } else {
                    TextAlign::Right
                }
            }
            other => other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpace {
    Normal,
    Nowrap,
    Pre,
    PreWrap,
    PreLine,
    BreakSpaces,
}

impl WhiteSpace {
    pub fn parse(s: &str) -> Option<WhiteSpace> {
        Some(match s {
            "normal" => WhiteSpace::Normal,
            "nowrap" => WhiteSpace::Nowrap,
            "pre" => WhiteSpace::Pre,
            "pre-wrap" => WhiteSpace::PreWrap,
            "pre-line" => WhiteSpace::PreLine,
            "break-spaces" => WhiteSpace::BreakSpaces,
            _ => return None,
        })
    }
    pub fn preserves_newlines(&self) -> bool {
        !matches!(self, WhiteSpace::Normal | WhiteSpace::Nowrap)
    }
    pub fn collapses_spaces(&self) -> bool {
        matches!(self, WhiteSpace::Normal | WhiteSpace::Nowrap | WhiteSpace::PreLine)
    }
    pub fn wraps(&self) -> bool {
        !matches!(self, WhiteSpace::Nowrap | WhiteSpace::Pre)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    None,
    Hidden,
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl BorderStyle {
    pub fn parse(s: &str) -> Option<BorderStyle> {
        Some(match s {
            "none" => BorderStyle::None,
            "hidden" => BorderStyle::Hidden,
            "solid" => BorderStyle::Solid,
            "dashed" => BorderStyle::Dashed,
            "dotted" => BorderStyle::Dotted,
            "double" => BorderStyle::Double,
            "groove" => BorderStyle::Groove,
            "ridge" => BorderStyle::Ridge,
            "inset" => BorderStyle::Inset,
            "outset" => BorderStyle::Outset,
            _ => return None,
        })
    }
    pub fn visible(&self) -> bool {
        !matches!(self, BorderStyle::None | BorderStyle::Hidden)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Visible,
    Hidden,
    Scroll,
    Auto,
    Clip,
}

impl Overflow {
    pub fn parse(s: &str) -> Option<Overflow> {
        Some(match s {
            "visible" => Overflow::Visible,
            "hidden" => Overflow::Hidden,
            "scroll" => Overflow::Scroll,
            "auto" => Overflow::Auto,
            "clip" => Overflow::Clip,
            _ => return None,
        })
    }
    pub fn clips(&self) -> bool {
        !matches!(self, Overflow::Visible)
    }
    pub fn scrolls(&self) -> bool {
        matches!(self, Overflow::Scroll | Overflow::Auto)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl FlexDirection {
    pub fn parse(s: &str) -> Option<FlexDirection> {
        Some(match s {
            "row" => FlexDirection::Row,
            "row-reverse" => FlexDirection::RowReverse,
            "column" => FlexDirection::Column,
            "column-reverse" => FlexDirection::ColumnReverse,
            _ => return None,
        })
    }
    pub fn is_row(&self) -> bool {
        matches!(self, FlexDirection::Row | FlexDirection::RowReverse)
    }
    pub fn reversed(&self) -> bool {
        matches!(self, FlexDirection::RowReverse | FlexDirection::ColumnReverse)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
    WrapReverse,
}

impl FlexWrap {
    pub fn parse(s: &str) -> Option<FlexWrap> {
        Some(match s {
            "nowrap" => FlexWrap::NoWrap,
            "wrap" => FlexWrap::Wrap,
            "wrap-reverse" => FlexWrap::WrapReverse,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Auto,
    Start,
    End,
    Center,
    Stretch,
    Baseline,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

impl Align {
    pub fn parse(s: &str) -> Option<Align> {
        Some(match s {
            "auto" => Align::Auto,
            "flex-start" | "start" | "left" => Align::Start,
            "flex-end" | "end" | "right" => Align::End,
            "center" => Align::Center,
            "stretch" | "normal" => Align::Stretch,
            "baseline" | "first baseline" => Align::Baseline,
            "space-between" => Align::SpaceBetween,
            "space-around" => Align::SpaceAround,
            "space-evenly" => Align::SpaceEvenly,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyleType {
    Disc,
    Circle,
    Square,
    Decimal,
    DecimalLeadingZero,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    None,
}

impl ListStyleType {
    pub fn parse(s: &str) -> Option<ListStyleType> {
        Some(match s {
            "disc" => ListStyleType::Disc,
            "circle" => ListStyleType::Circle,
            "square" => ListStyleType::Square,
            "decimal" => ListStyleType::Decimal,
            "decimal-leading-zero" => ListStyleType::DecimalLeadingZero,
            "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
            "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
            "lower-roman" => ListStyleType::LowerRoman,
            "upper-roman" => ListStyleType::UpperRoman,
            "none" => ListStyleType::None,
            _ => return None,
        })
    }

    pub fn marker(&self, index: usize) -> String {
        let n = index + 1;
        match self {
            ListStyleType::Disc => "•".into(),
            ListStyleType::Circle => "◦".into(),
            ListStyleType::Square => "▪".into(),
            ListStyleType::None => String::new(),
            ListStyleType::Decimal => format!("{n}."),
            ListStyleType::DecimalLeadingZero => format!("{:02}.", n),
            ListStyleType::LowerAlpha => format!("{}.", alpha(n, false)),
            ListStyleType::UpperAlpha => format!("{}.", alpha(n, true)),
            ListStyleType::LowerRoman => format!("{}.", roman(n).to_lowercase()),
            ListStyleType::UpperRoman => format!("{}.", roman(n)),
        }
    }
}

fn alpha(mut n: usize, upper: bool) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut out = String::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.insert(0, (b'a' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    if upper {
        out.to_ascii_uppercase()
    } else {
        out
    }
}

fn roman(mut n: usize) -> String {
    const TABLE: &[(usize, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, sym) in TABLE {
        while n >= *value {
            out.push_str(sym);
            n -= *value;
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
    Collapse,
}

impl Visibility {
    pub fn parse(s: &str) -> Option<Visibility> {
        Some(match s {
            "visible" => Visibility::Visible,
            "hidden" => Visibility::Hidden,
            "collapse" => Visibility::Collapse,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSizing {
    ContentBox,
    BorderBox,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrack {
    Px(f32),
    Percent(f32),
    Fr(f32),
    /// A `minmax(min, max)` track with fixed px bounds.
    MinMax(f32, f32),
    Auto,
    MinContent,
    MaxContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridPlacement {
    Auto,
    Line(i32),
    Span(u16),
}

impl GridPlacement {
    pub fn parse(s: &str) -> Option<GridPlacement> {
        let s = s.trim();
        if s == "auto" {
            return Some(GridPlacement::Auto);
        }
        if let Some(rest) = s.strip_prefix("span") {
            let n = rest.trim().parse::<u16>().unwrap_or(1);
            return Some(GridPlacement::Span(n.max(1)));
        }
        s.parse::<i32>().ok().map(GridPlacement::Line)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

/// `MediaEnv` decides which media queries apply.
#[derive(Debug, Clone)]
pub struct MediaEnv {
    pub viewport: Size,
    pub prefers_dark: bool,
    pub reduced_motion: bool,
    pub print: bool,
}

impl Default for MediaEnv {
    fn default() -> Self {
        MediaEnv {
            viewport: Size::new(1024.0, 768.0),
            prefers_dark: false,
            reduced_motion: false,
            print: false,
        }
    }
}

/// Extracts a numeric prefix (px value) from simple math expressions.
pub fn parse_length(text: &str, font_size: f32, root_size: f32, viewport: Size) -> Option<Length> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower == "auto" {
        return Some(Length::Auto);
    }
    if lower.starts_with("calc(") && lower.ends_with(')') {
        return parse_calc(&lower[5..lower.len() - 1], font_size, root_size, viewport);
    }
    parse_single_length(&lower, font_size, root_size, viewport)
}

fn parse_single_length(s: &str, font_size: f32, root_size: f32, viewport: Size) -> Option<Length> {
    if let Some(v) = s.strip_suffix("px") {
        return v.trim().parse::<f32>().ok().map(Length::px);
    }
    if let Some(v) = s.strip_suffix('%') {
        return v.trim().parse::<f32>().ok().map(Length::percent);
    }
    if let Some(v) = s.strip_suffix("rem") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * root_size));
    }
    if let Some(v) = s.strip_suffix("em") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * font_size));
    }
    if let Some(v) = s.strip_suffix("ex") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * font_size * 0.5));
    }
    if let Some(v) = s.strip_suffix("ch") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * font_size * 0.5));
    }
    if let Some(v) = s.strip_suffix("vw") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * viewport.w / 100.0));
    }
    if let Some(v) = s.strip_suffix("vh") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * viewport.h / 100.0));
    }
    if let Some(v) = s.strip_suffix("vmin") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * viewport.w.min(viewport.h) / 100.0));
    }
    if let Some(v) = s.strip_suffix("vmax") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * viewport.w.max(viewport.h) / 100.0));
    }
    if let Some(v) = s.strip_suffix("pt") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * 96.0 / 72.0));
    }
    if let Some(v) = s.strip_suffix("pc") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * 16.0));
    }
    if let Some(v) = s.strip_suffix("cm") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * 96.0 / 2.54));
    }
    if let Some(v) = s.strip_suffix("mm") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * 96.0 / 25.4));
    }
    if let Some(v) = s.strip_suffix("in") {
        return v.trim().parse::<f32>().ok().map(|n| Length::px(n * 96.0));
    }
    if s == "0" {
        return Some(Length::ZERO);
    }
    // Unitless zero is valid; other unitless numbers are not lengths.
    if let Ok(v) = s.parse::<f32>() {
        if v == 0.0 {
            return Some(Length::ZERO);
        }
    }
    None
}

/// A reduced calc value: sums of px and percentage terms plus a unitless
/// factor for expressions such as `calc(var(--spacing) * 8)`.
#[derive(Clone, Copy, Debug, Default)]
struct CalcValue {
    px: f32,
    pct: f32,
    num: f32,
}

fn calc_tokens(expr: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, tokens: &mut Vec<String>| {
        if !current.trim().is_empty() {
            tokens.push(current.trim().to_string());
        }
        current.clear();
    };
    for ch in expr.chars() {
        match ch {
            '+' | '-' | '*' | '/' | '(' | ')' => {
                flush(&mut current, &mut tokens);
                tokens.push(ch.to_string());
            }
            c if c.is_whitespace() => flush(&mut current, &mut tokens),
            c => current.push(c),
        }
    }
    flush(&mut current, &mut tokens);
    tokens
}

fn calc_atom(
    token: &str,
    font_size: f32,
    root_size: f32,
    viewport: Size,
) -> Option<CalcValue> {
    if let Ok(n) = token.parse::<f32>() {
        return Some(CalcValue { num: n, ..CalcValue::default() });
    }
    let len = parse_single_length(token, font_size, root_size, viewport)?;
    match len {
        Length::Px(v) => Some(CalcValue { px: v, ..CalcValue::default() }),
        Length::Percent(p) => Some(CalcValue { pct: p, ..CalcValue::default() }),
        Length::Calc { px, pct } => Some(CalcValue { px, pct, num: 0.0 }),
        Length::Auto => None,
    }
}

fn calc_primary(
    tokens: &[String],
    pos: &mut usize,
    font_size: f32,
    root_size: f32,
    viewport: Size,
) -> Option<CalcValue> {
    let token = tokens.get(*pos)?.clone();
    *pos += 1;
    if token == "(" {
        let value = calc_add(tokens, pos, font_size, root_size, viewport)?;
        if tokens.get(*pos).map(|t| t.as_str()) != Some(")") {
            return None;
        }
        *pos += 1;
        return Some(value);
    }
    calc_atom(&token, font_size, root_size, viewport)
}

fn calc_mul(
    tokens: &[String],
    pos: &mut usize,
    font_size: f32,
    root_size: f32,
    viewport: Size,
) -> Option<CalcValue> {
    let mut left = calc_primary(tokens, pos, font_size, root_size, viewport)?;
    while let Some(op) = tokens.get(*pos).map(|t| t.as_str()) {
        if op != "*" && op != "/" {
            break;
        }
        *pos += 1;
        let right = calc_primary(tokens, pos, font_size, root_size, viewport)?;
        if op == "*" {
            if left.pct != 0.0 || left.px != 0.0 {
                if right.num == 0.0 {
                    return None;
                }
                left.px *= right.num;
                left.pct *= right.num;
            } else {
                if right.pct != 0.0 || right.px != 0.0 {
                    return None;
                }
                left.num *= right.num;
            }
        } else {
            if right.num == 0.0 {
                return None;
            }
            left.px /= right.num;
            left.pct /= right.num;
            if left.px == 0.0 && left.pct == 0.0 && left.num != 0.0 {
                left.num /= right.num;
            }
        }
    }
    Some(left)
}

fn calc_add(
    tokens: &[String],
    pos: &mut usize,
    font_size: f32,
    root_size: f32,
    viewport: Size,
) -> Option<CalcValue> {
    let mut left = calc_mul(tokens, pos, font_size, root_size, viewport)?;
    while let Some(op) = tokens.get(*pos).map(|t| t.as_str()) {
        if op != "+" && op != "-" {
            break;
        }
        *pos += 1;
        let right = calc_mul(tokens, pos, font_size, root_size, viewport)?;
        let sign = if op == "+" { 1.0 } else { -1.0 };
        left.px += sign * right.px;
        left.pct += sign * right.pct;
        left.num += sign * right.num;
    }
    Some(left)
}

/// Evaluates the arithmetic subset of `calc()` used by modern CSS frameworks,
/// including multiplication and division by unitless values.
fn parse_calc(expr: &str, font_size: f32, root_size: f32, viewport: Size) -> Option<Length> {
    let tokens = calc_tokens(expr);
    if tokens.is_empty() {
        return None;
    }
    let mut pos = 0usize;
    let value = calc_add(&tokens, &mut pos, font_size, root_size, viewport)?;
    if pos != tokens.len() || value.num != 0.0 {
        return None;
    }
    if value.pct == 0.0 {
        Some(Length::px(value.px))
    } else if value.px == 0.0 {
        Some(Length::percent(value.pct))
    } else {
        Some(Length::Calc { px: value.px, pct: value.pct })
    }
}

/// Parses 1–4 length values using CSS's TRBL expansion.
pub fn parse_four_lengths(text: &str, font_size: f32, root_size: f32, viewport: Size) -> Option<EdgesLength> {
    let mut parts: Vec<&str> = Vec::new();
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    for (i, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => {
                if let Some(s) = start.take() {
                    parts.push(&text[s..i]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(s) = start {
        parts.push(&text[s..]);
    }
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut vals = Vec::new();
    for p in parts {
        vals.push(parse_length(p, font_size, root_size, viewport)?);
    }
    let (top, right, bottom, left) = match vals.len() {
        1 => (vals[0], vals[0], vals[0], vals[0]),
        2 => (vals[0], vals[1], vals[0], vals[1]),
        3 => (vals[0], vals[1], vals[2], vals[1]),
        _ => (vals[0], vals[1], vals[2], vals[3]),
    };
    Some(EdgesLength { top, right, bottom, left })
}

/// Splits a value on top-level whitespace, keeping functions intact.
pub fn split_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(q) => {
                current.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    current.push(c);
                }
                '(' => {
                    depth += 1;
                    current.push(c);
                }
                ')' => {
                    depth -= 1;
                    current.push(c);
                }
                c if c.is_whitespace() && depth == 0 => {
                    if !current.is_empty() {
                        out.push(current.clone());
                        current.clear();
                    }
                }
                _ => current.push(c),
            },
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Splits on top-level commas (used by `font-family`, gradients, etc).
pub fn split_commas(text: &str) -> Vec<String> {
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
                out.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

pub fn strip_quotes(s: &str) -> String {
    let t = s.trim();
    if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2)
    {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

pub fn parse_color_value(text: &str, current_color: Color) -> Option<Color> {
    let t = text.trim();
    if t.eq_ignore_ascii_case("currentcolor") {
        return Some(current_color);
    }
    Color::parse(t)
}

/// Basic CSS colour functions used by gradients (returns stop list).
#[derive(Debug, Clone, Copy)]
pub struct GradientStop {
    pub color: Color,
    pub position: Option<f32>,
}

pub fn parse_linear_gradient(text: &str) -> Option<(f32, Vec<GradientStop>)> {
    let t = text.trim();
    let inner = t
        .strip_prefix("linear-gradient(")
        .or_else(|| t.strip_prefix("repeating-linear-gradient("))?
        .strip_suffix(')')?;
    let mut parts = split_commas(inner);
    if parts.is_empty() {
        return None;
    }
    let mut angle = 180.0f32;
    if let Some(first) = parts.first() {
        let f = first.trim().to_ascii_lowercase();
        if let Some(deg) = f.strip_suffix("deg") {
            if let Ok(v) = deg.trim().parse::<f32>() {
                angle = v;
                parts.remove(0);
            }
        } else if f == "to bottom" {
            angle = 180.0;
            parts.remove(0);
        } else if f == "to top" {
            angle = 0.0;
            parts.remove(0);
        } else if f == "to right" {
            angle = 90.0;
            parts.remove(0);
        } else if f == "to left" {
            angle = 270.0;
            parts.remove(0);
        }
    }
    let mut stops = Vec::new();
    for part in parts {
        let tokens = split_tokens(&part);
        if tokens.is_empty() {
            continue;
        }
        let color = Color::parse(&tokens[0])?;
        let position = tokens.get(1).and_then(|p| {
            p.strip_suffix('%')
                .and_then(|v| v.trim().parse::<f32>().ok())
                .map(|v| v / 100.0)
        });
        stops.push(GradientStop { color, position });
    }
    if stops.is_empty() {
        None
    } else {
        Some((angle, stops))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        let vp = Size::new(1000.0, 500.0);
        assert_eq!(parse_length("10px", 16.0, 16.0, vp), Some(Length::Px(10.0)));
        assert_eq!(parse_length("2em", 16.0, 16.0, vp), Some(Length::Px(32.0)));
        assert_eq!(parse_length("50%", 16.0, 16.0, vp), Some(Length::Percent(50.0)));
        assert_eq!(parse_length("10vw", 16.0, 16.0, vp), Some(Length::Px(100.0)));
        assert_eq!(
            parse_length("calc(100% - 20px)", 16.0, 16.0, vp),
            Some(Length::Calc { px: -20.0, pct: 100.0 })
        );
        assert!(parse_length("red", 16.0, 16.0, vp).is_none());
    }

    #[test]
    fn calc_multiplication_and_division() {
        let vp = Size::new(1000.0, 500.0);
        assert_eq!(
            parse_length("calc(0.25rem * 8)", 16.0, 16.0, vp),
            Some(Length::Px(32.0))
        );
        assert_eq!(
            parse_length("calc((100% - 20px) / 2)", 16.0, 16.0, vp),
            Some(Length::Calc { px: -10.0, pct: 50.0 })
        );
        let edges = parse_four_lengths("calc(4px * 8) 2px", 16.0, 16.0, vp).unwrap();
        assert_eq!(edges.top, Length::Px(32.0));
        assert_eq!(edges.left, Length::Px(2.0));
    }

    #[test]
    fn four_values() {
        let vp = Size::new(1000.0, 500.0);
        let e = parse_four_lengths("1px 2px 3px", 16.0, 16.0, vp).unwrap();
        assert_eq!(e.top, Length::Px(1.0));
        assert_eq!(e.right, Length::Px(2.0));
        assert_eq!(e.bottom, Length::Px(3.0));
        assert_eq!(e.left, Length::Px(2.0));
    }

    #[test]
    fn lists() {
        let t = split_tokens("1px solid \"Helvetica Neue\"");
        assert_eq!(t.len(), 3);
        let c = split_commas("a, rgb(1,2,3), b");
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn markers() {
        assert_eq!(ListStyleType::UpperRoman.marker(3), "IV.");
        assert_eq!(ListStyleType::LowerAlpha.marker(1), "b.");
    }
}
