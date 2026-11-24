//! classList, inline style, dataset and getComputedStyle.

use std::cell::RefCell;
use std::rc::Rc;

use veyra_common::{Color, Size};
use veyra_css::values::{Display, Length, Overflow, Position, Visibility};
use veyra_css::style::TextDecoration;
use veyra_css::ComputedStyle;
use veyra_dom::NodeId;
use veyra_js::{Interp, ObjRef, Value};
use veyra_js::value::{define, define_accessor};

use crate::bindings::{arg, getter, getset, method};
use crate::state::DomState;

pub fn make_class_list(it: &mut Interp, state: &Rc<RefCell<DomState>>, node: NodeId) -> Value {
    let obj = it.object();
    let o = obj.as_object().unwrap().clone();
    let st = state.clone();
    method(it, &o, "add", 1, &st, node, |it, st, n, args| {
        for v in &args {
            let class = it.to_string(v)?;
            let mut s = st.borrow_mut();
            let mut classes = s.doc.classes(n);
            if !classes.contains(&class) {
                classes.push(class);
                s.doc.set_attr(n, "class", &classes.join(" "));
                s.mark_dirty();
            }
        }
        Ok(Value::Undefined)
    });
    let st = state.clone();
    method(it, &o, "remove", 1, &st, node, |it, st, n, args| {
        for v in &args {
            let class = it.to_string(v)?;
            let mut s = st.borrow_mut();
            let mut classes = s.doc.classes(n);
            classes.retain(|c| c != &class);
            s.doc.set_attr(n, "class", &classes.join(" "));
            s.mark_dirty();
        }
        Ok(Value::Undefined)
    });
    let st = state.clone();
    method(it, &o, "toggle", 2, &st, node, |it, st, n, args| {
        let class = it.to_string(&arg(&args, 0))?;
        let force = args.get(1).map(|v| v.truthy());
        let mut s = st.borrow_mut();
        let mut classes = s.doc.classes(n);
        let has = classes.contains(&class);
        let want = force.unwrap_or(!has);
        if want && !has {
            classes.push(class);
        } else if !want && has {
            classes.retain(|c| c != &class);
        }
        s.doc.set_attr(n, "class", &classes.join(" "));
        s.mark_dirty();
        Ok(Value::Bool(want))
    });
    let st = state.clone();
    method(it, &o, "contains", 1, &st, node, |it, st, n, args| {
        let class = it.to_string(&arg(&args, 0))?;
        Ok(Value::Bool(st.borrow().doc.classes(n).contains(&class)))
    });
    let st = state.clone();
    method(it, &o, "replace", 2, &st, node, |it, st, n, args| {
        let old = it.to_string(&arg(&args, 0))?;
        let new = it.to_string(&arg(&args, 1))?;
        let mut s = st.borrow_mut();
        let mut classes = s.doc.classes(n);
        let changed = if let Some(pos) = classes.iter().position(|c| c == &old) {
            classes[pos] = new;
            true
        } else {
            false
        };
        if changed {
            s.doc.set_attr(n, "class", &classes.join(" "));
            s.mark_dirty();
        }
        Ok(Value::Bool(changed))
    });
    let st = state.clone();
    let len = it.native("length", 0, move |_, _, _| {
        Ok(Value::Number(st.borrow().doc.classes(node).len() as f64))
    });
    define_accessor(&o, "length", Some(len), None, false);
    let st = state.clone();
    getter(it, &o, "value", &st, node, |_, st, n| {
        Ok(Value::string(st.borrow().doc.attr(n, "class").unwrap_or("").to_string()))
    });
    let st = state.clone();
    method(it, &o, "item", 1, &st, node, |it, st, n, args| {
        let i = arg(&args, 0).to_index();
        Ok(st
            .borrow()
            .doc
            .classes(n)
            .get(i)
            .map(|c| Value::string(c.clone()))
            .unwrap_or(Value::Null))
    });
    let st = state.clone();
    method(it, &o, "toString", 0, &st, node, |_, st, n, _| {
        Ok(Value::string(st.borrow().doc.attr(n, "class").unwrap_or("").to_string()))
    });
    let st = state.clone();
    let iter = it.native("Symbol.iterator", 0, move |it, _, _| {
        let values = st
            .borrow()
            .doc
            .classes(node)
            .into_iter()
            .map(Value::string)
            .collect();
        Ok(it.array_from(values))
    });
    define(&o, "@@symbol:Symbol.iterator", iter);
    obj
}

const STYLE_PROPS: &[&str] = &[
    "display", "position", "top", "right", "bottom", "left", "width", "height", "minWidth",
    "maxWidth", "minHeight", "maxHeight", "margin", "marginTop", "marginRight", "marginBottom",
    "marginLeft", "padding", "paddingTop", "paddingRight", "paddingBottom", "paddingLeft",
    "color", "backgroundColor", "background", "backgroundImage", "fontSize", "fontWeight",
    "fontFamily", "lineHeight", "letterSpacing", "textAlign", "textDecoration", "border",
    "borderWidth", "borderStyle", "borderColor", "borderRadius", "boxShadow", "opacity",
    "transform", "transformOrigin", "transition", "animation", "zIndex", "flex", "flexDirection",
    "justifyContent", "alignItems", "gap", "overflow", "overflowX", "overflowY", "cursor",
    "visibility", "objectFit", "whiteSpace", "wordBreak", "gridTemplateColumns", "order",
];

pub fn make_style(it: &mut Interp, state: &Rc<RefCell<DomState>>, node: NodeId) -> Value {
    let obj = it.object();
    let o = obj.as_object().unwrap().clone();
    let st = state.clone();
    method(it, &o, "setProperty", 2, &st, node, |it, st, n, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let value = it.to_string(&arg(&args, 1))?;
        set_inline_style(st, n, &name, &value);
        Ok(Value::Undefined)
    });
    let st = state.clone();
    method(it, &o, "getPropertyValue", 1, &st, node, |it, st, n, args| {
        let name = it.to_string(&arg(&args, 0))?;
        Ok(Value::string(get_inline_style(&st.borrow(), n, &name).unwrap_or_default()))
    });
    let st = state.clone();
    method(it, &o, "removeProperty", 1, &st, node, |it, st, n, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let old = get_inline_style(&st.borrow(), n, &name).unwrap_or_default();
        set_inline_style(st, n, &name, "");
        Ok(Value::string(old))
    });
    let st = state.clone();
    getset(
        it,
        &o,
        "cssText",
        &st,
        node,
        |_, st, n| Ok(Value::string(st.borrow().doc.attr(n, "style").unwrap_or("").to_string())),
        |_, st, n, v| {
            let mut s = st.borrow_mut();
            s.doc.set_attr(n, "style", &v.to_string_primitive());
            s.mark_dirty();
            Ok(())
        },
    );
    for prop in STYLE_PROPS {
        let kebab = camel_to_kebab(prop);
        let get_kebab = kebab.clone();
        let set_kebab = kebab.clone();
        getset(
            it,
            &o,
            prop,
            state,
            node,
            move |_, st, n| Ok(Value::string(get_inline_style(&st.borrow(), n, &get_kebab).unwrap_or_default())),
            move |_, st, n, v| {
                set_inline_style(st, n, &set_kebab, &v.to_string_primitive());
                Ok(())
            },
        );
    }
    obj
}

fn set_inline_style(state: &Rc<RefCell<DomState>>, node: NodeId, prop: &str, value: &str) {
    let mut s = state.borrow_mut();
    let attr = s.doc.attr(node, "style").unwrap_or("").to_string();
    let mut decls: Vec<(String, String)> = attr
        .split(';')
        .filter_map(|d| d.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .filter(|(k, _)| !k.is_empty())
        .collect();
    if let Some(entry) = decls.iter_mut().find(|(k, _)| k == prop) {
        if value.is_empty() {
            decls.retain(|(k, _)| k != prop);
        } else {
            entry.1 = value.to_string();
        }
    } else if !value.is_empty() {
        decls.push((prop.to_string(), value.to_string()));
    }
    let text = decls.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("; ");
    s.doc.set_attr(node, "style", &text);
    s.mark_dirty();
}

fn get_inline_style(state: &DomState, node: NodeId, prop: &str) -> Option<String> {
    let attr = state.doc.attr(node, "style")?;
    let mut found = None;
    for d in attr.split(';') {
        if let Some((k, v)) = d.split_once(':') {
            if k.trim() == prop {
                found = Some(v.trim().to_string());
            }
        }
    }
    found
}

pub fn make_dataset(it: &mut Interp, state: &Rc<RefCell<DomState>>, node: NodeId) -> Value {
    let obj = it.object();
    let pairs = state.borrow().doc.attr_pairs(node).to_vec();
    if let Some(o) = obj.as_object() {
        for (k, v) in pairs {
            if let Some(rest) = k.strip_prefix("data-") {
                let key = kebab_to_camel(rest);
                define(o, &key, Value::string(v));
            }
        }
    }
    obj
}

pub fn make_computed_style(it: &mut Interp, state: &Rc<RefCell<DomState>>, node: NodeId) -> Value {
    let obj = it.object();
    let o = obj.as_object().unwrap().clone();
    let st = state.clone();
    method(it, &o, "getPropertyValue", 1, &st, node, |it, st, n, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let s = st.borrow();
        let style = s.styles.style(n);
        Ok(Value::string(computed_value(style, &name)))
    });
    for prop in [
        "display", "position", "color", "backgroundColor", "fontSize", "fontWeight", "width",
        "height", "margin", "marginTop", "marginRight", "marginBottom", "marginLeft",
        "padding", "paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "opacity",
        "visibility", "zIndex", "borderRadius", "overflow", "transform", "textDecoration",
        "textDecorationLine", "gridTemplateColumns", "gridTemplateRows", "rowGap", "columnGap",
    ] {
        let kebab = camel_to_kebab(prop);
        getter(it, &o, prop, state, node, move |_, st, n| {
            let s = st.borrow();
            Ok(Value::string(computed_value(s.styles.style(n), &kebab)))
        });
    }
    obj
}

fn computed_value(style: &ComputedStyle, name: &str) -> String {
    match name {
        "display" => display_name(style.display).into(),
        "position" => match style.position {
            Position::Static => "static",
            Position::Relative => "relative",
            Position::Absolute => "absolute",
            Position::Fixed => "fixed",
            Position::Sticky => "sticky",
        }
        .into(),
        "color" => color_css(style.color),
        "background-color" => color_css(style.background_color),
        "font-size" => format!("{}px", style.font_size),
        "font-weight" => style.font_weight.to_string(),
        "width" | "height" => {
            let v = if name == "width" { style.width } else { style.height };
            length_css(v)
        }
        "margin" => format!(
            "{} {} {} {}",
            length_css(style.margin.top), length_css(style.margin.right),
            length_css(style.margin.bottom), length_css(style.margin.left)
        ),
        "margin-top" => length_css(style.margin.top),
        "margin-right" => length_css(style.margin.right),
        "margin-bottom" => length_css(style.margin.bottom),
        "margin-left" => length_css(style.margin.left),
        "padding" => format!(
            "{} {} {} {}",
            length_css(style.padding.top), length_css(style.padding.right),
            length_css(style.padding.bottom), length_css(style.padding.left)
        ),
        "padding-top" => length_css(style.padding.top),
        "padding-right" => length_css(style.padding.right),
        "padding-bottom" => length_css(style.padding.bottom),
        "padding-left" => length_css(style.padding.left),
        "grid-template-columns" | "grid-template-rows" => {
            let tracks = if name == "grid-template-columns" { &style.grid_columns } else { &style.grid_rows };
            tracks
                .iter()
                .map(|t| match t {
                    veyra_css::values::GridTrack::Px(v) => format!("{v}px"),
                    veyra_css::values::GridTrack::Percent(p) => format!("{p}%"),
                    veyra_css::values::GridTrack::Fr(f) => format!("{f}fr"),
                    veyra_css::values::GridTrack::MinMax(min, max) => format!("minmax({min}px, {max}px)"),
                    veyra_css::values::GridTrack::Auto => "auto".into(),
                    veyra_css::values::GridTrack::MinContent => "min-content".into(),
                    veyra_css::values::GridTrack::MaxContent => "max-content".into(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
        "grid-area" => style.grid_area.clone().unwrap_or_else(|| "auto".into()),
        "grid-template-areas" => style
            .grid_areas
            .iter()
            .map(|row| format!("\"{}\"", row.join(" ")))
            .collect::<Vec<_>>()
            .join(" "),
        "grid-column-start" => format_placement(style.grid_column_start),
        "grid-column-end" => format_placement(style.grid_column_end),
        "grid-row-start" => format_placement(style.grid_row_start),
        "grid-row-end" => format_placement(style.grid_row_end),
        "grid-column" => format!("{} / {}", format_placement(style.grid_column_start), format_placement(style.grid_column_end)),
        "justify-content" => format!("{:?}", style.justify_content).to_lowercase(),
        "row-gap" => format!("{}px", style.row_gap.resolve_or(0.0, 0.0)),
        "column-gap" => format!("{}px", style.column_gap.resolve_or(0.0, 0.0)),
        "opacity" => format!("{}", style.opacity),
        "visibility" => match style.visibility {
            Visibility::Visible => "visible",
            Visibility::Hidden => "hidden",
            Visibility::Collapse => "collapse",
        }
        .into(),
        "text-decoration" | "text-decoration-line" => match style.text_decoration {
            TextDecoration::Underline => "underline",
            TextDecoration::LineThrough => "line-through",
            TextDecoration::Overline => "overline",
            TextDecoration::None => "none",
        }
        .into(),
        "z-index" => style.z_index.to_string(),
        "border-radius" => format!("{}px", style.border_radius),
        "overflow" => match style.overflow_x {
            Overflow::Visible => "visible",
            Overflow::Hidden => "hidden",
            Overflow::Scroll => "scroll",
            Overflow::Auto => "auto",
            Overflow::Clip => "clip",
        }
        .into(),
        "transform" => style
            .transform
            .map(|t| format!("matrix({},{},{},{},{},{})", t.a, t.b, t.c, t.d, t.e, t.f))
            .unwrap_or_else(|| "none".into()),
        other => style.custom_properties.get(other).cloned().unwrap_or_default(),
    }
}

fn format_placement(p: veyra_css::values::GridPlacement) -> String {
    match p {
        veyra_css::values::GridPlacement::Auto => "auto".into(),
        veyra_css::values::GridPlacement::Line(n) => n.to_string(),
        veyra_css::values::GridPlacement::Span(n) => format!("span {n}"),
    }
}

fn display_name(d: Display) -> &'static str {
    match d {
        Display::None => "none",
        Display::Block => "block",
        Display::Inline => "inline",
        Display::InlineBlock => "inline-block",
        Display::Flex => "flex",
        Display::InlineFlex => "inline-flex",
        Display::Grid => "grid",
        Display::InlineGrid => "inline-grid",
        Display::ListItem => "list-item",
        Display::Table => "table",
        Display::InlineTable => "inline-table",
        Display::TableRow => "table-row",
        Display::TableCell => "table-cell",
        Display::TableRowGroup => "table-row-group",
        Display::TableHeaderGroup => "table-header-group",
        Display::TableFooterGroup => "table-footer-group",
        Display::TableColumn => "table-column",
        Display::TableColumnGroup => "table-column-group",
        Display::TableCaption => "table-caption",
        Display::Contents => "contents",
    }
}

fn color_css(c: Color) -> String {
    if c.a == 255 {
        format!("rgb({}, {}, {})", c.r, c.g, c.b)
    } else {
        format!("rgba({}, {}, {}, {:.3})", c.r, c.g, c.b, c.a as f32 / 255.0)
    }
}

fn length_css(l: Length) -> String {
    match l {
        Length::Auto => "auto".into(),
        Length::Px(px) => format!("{px}px"),
        Length::Percent(p) => format!("{p}%"),
        Length::Calc { px, pct } => format!("calc({px}px + {pct}%)"),
    }
}

pub fn camel_to_kebab(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn kebab_to_camel(s: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in s.chars() {
        if c == '-' {
            upper = true;
        } else if upper {
            out.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

pub fn media_matches(query: &str, viewport: Size, dark: bool) -> bool {
    // Delegate to the stylesheet media-query parser so `screen and (…)`,
    // `not`, `only` and ranges behave the same as they do for CSS rules.
    let queries = veyra_css::parser::parse_media_list(query);
    let env = veyra_css::values::MediaEnv {
        viewport,
        prefers_dark: dark,
        ..Default::default()
    };
    veyra_css::parser::media_matches(&queries, &env)
}
