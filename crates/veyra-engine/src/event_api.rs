//! Event dispatch (bubble phase) and browser default actions.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use veyra_dom::NodeId;
use veyra_js::{Interp, JsError, Value};
use veyra_js::value::{define, define_accessor, own_prop};

use crate::bindings::{arg, strict, wrap, wrappers};
use crate::state::{DomState, Listener, Mutation};

/// Returns the event path from `target` to the document, crossing shadow
/// boundaries through their hosts when the event is composed.
pub(crate) fn event_path(state: &DomState, target: NodeId, composed: bool) -> Vec<NodeId> {
    let mut chain = Vec::new();
    let mut cur = Some(target);
    while let Some(n) = cur {
        chain.push(n);
        if composed {
            if state.doc.is_shadow_root(n) {
                if let Some(host) = state.doc.shadow_hosts.get(&n).copied() {
                    cur = Some(host);
                    continue;
                }
            }
        }
        cur = state.doc.parent(n);
    }
    chain
}

pub fn dispatch_event(
    it: &mut Interp,
    state: &Rc<RefCell<DomState>>,
    target: NodeId,
    event_type: &str,
) -> Result<bool, JsError> {
    dispatch_event_object(it, state, target, event_type, None)
}

/// Dispatches an existing event object (or a fresh one) and preserves its
/// public fields. DOM `dispatchEvent` must use the object the page created so
/// listeners can read `detail`, `bubbles` and `composed`.
pub fn dispatch_event_object(
    it: &mut Interp,
    state: &Rc<RefCell<DomState>>,
    target: NodeId,
    fallback_type: &str,
    supplied: Option<Value>,
) -> Result<bool, JsError> {
    it.begin_turn();
    let fresh = !matches!(&supplied, Some(Value::Object(_)));
    let event = match supplied {
        Some(v @ Value::Object(_)) => v,
        _ => it.object(),
    };
    if fresh {
        if let Some(e) = event.as_object() {
            define(e, "bubbles", Value::Bool(true));
            define(e, "composed", Value::Bool(true));
            define(e, "cancelable", Value::Bool(true));
        }
    }
    let event_type = it
        .get_member(event.clone(), "type")
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| fallback_type.to_string());
    let composed = it
        .get_member(event.clone(), "composed")
        .map(|v| v.truthy())
        .unwrap_or(true);
    let bubbles = it
        .get_member(event.clone(), "bubbles")
        .map(|v| v.truthy())
        .unwrap_or(true);
    let chain = {
        let s = state.borrow();
        event_path(&s, target, composed)
    };
    let walk_chain = if bubbles { chain.clone() } else { vec![target] };
    let handlers: Vec<Listener> = {
        let s = state.borrow();
        s.listeners
            .iter()
            .filter(|l| l.event == event_type && walk_chain.contains(&l.node))
            .cloned()
            .collect()
    };

    let prevented = Rc::new(Cell::new(false));
    let stopped = Rc::new(Cell::new(false));
    if let Some(e) = event.as_object() {
        define(e, "type", Value::string(event_type.clone()));
        define(e, "target", wrap(it, state, target));
        define(e, "currentTarget", wrap(it, state, target));
        if own_prop(e, "timeStamp").is_none() {
            define(e, "timeStamp", Value::Number(it.now_ms()));
        }
        let p = prevented.clone();
        define(e, "preventDefault", it.native("preventDefault", 0, move |_, _, _| {
            p.set(true);
            Ok(Value::Undefined)
        }));
        let sf = stopped.clone();
        define(e, "stopPropagation", it.native("stopPropagation", 0, move |_, _, _| {
            sf.set(true);
            Ok(Value::Undefined)
        }));
        let sf = stopped.clone();
        define(e, "stopImmediatePropagation", it.native("stopImmediatePropagation", 0, move |_, _, _| {
            sf.set(true);
            Ok(Value::Undefined)
        }));
        let st = state.clone();
        let path = chain.clone();
        define(e, "composedPath", it.native("composedPath", 0, move |it, _, _| {
            Ok(wrappers(it, &st, path.iter().copied()))
        }));
        if event_type == "click" {
            if own_prop(e, "button").is_none() {
                define(e, "button", Value::Number(0.0));
            }
            if own_prop(e, "clientX").is_none() {
                define(e, "clientX", Value::Number(0.0));
            }
            if own_prop(e, "clientY").is_none() {
                define(e, "clientY", Value::Number(0.0));
            }
            if own_prop(e, "detail").is_none() {
                define(e, "detail", Value::Number(1.0));
            }
        }
        if event_type == "keydown" || event_type == "keyup" || event_type == "keypress" {
            define(e, "key", Value::string(""));
            define(e, "code", Value::string(""));
            define(e, "repeat", Value::Bool(false));
            define(e, "altKey", Value::Bool(false));
            define(e, "ctrlKey", Value::Bool(false));
            define(e, "metaKey", Value::Bool(false));
            define(e, "shiftKey", Value::Bool(false));
        }
    }

    for l in handlers {
        if stopped.get() {
            break;
        }
        if let Some(e) = event.as_object() {
            define(e, "currentTarget", wrap(it, state, l.node));
        }
        let this = wrap(it, state, l.node);
        let result = it.call_value(&l.callback, this, vec![event.clone()]);
        if l.once {
            state.borrow_mut().listeners.retain(|x| {
                !(x.node == l.node && strict(&x.callback, &l.callback) && x.event == event_type)
            });
        }
        if let Err(e) = result {
            it.output.push(format!("event handler error: {e}"));
        }
    }
    if let Some(e) = event.as_object() {
        define(e, "defaultPrevented", Value::Bool(prevented.get()));
    }

    if !stopped.get() {
        default_action(it, state, target, &event_type)?;
    }
    Ok(prevented.get())
}

fn default_action(
    it: &mut Interp,
    state: &Rc<RefCell<DomState>>,
    target: NodeId,
    event_type: &str,
) -> Result<(), JsError> {
    if event_type != "click" {
        return Ok(());
    }
    let mut navigation = None;
    let mut toggle: Option<(NodeId, bool)> = None;
    let mut submit_form: Option<NodeId> = None;
    {
        let s = state.borrow();
        let mut cur = Some(target);
        let mut depth = 0;
        while let Some(n) = cur {
            let tag = s.doc.tag_name(n);
            if tag == "a" {
                if let Some(href) = s.doc.attr(n, "href") {
                    if !href.starts_with('#')
                        && !href.to_ascii_lowercase().starts_with("javascript:")
                        && !href.to_ascii_lowercase().starts_with("mailto:")
                    {
                        navigation = Some(s.resolve(href));
                    }
                }
                break;
            }
            if tag == "input" {
                let ty = s.doc.attr(n, "type").unwrap_or("text").to_ascii_lowercase();
                if ty == "checkbox" || ty == "radio" {
                    let current = s
                        .checked
                        .get(&n)
                        .copied()
                        .unwrap_or_else(|| s.doc.attr(n, "checked").is_some());
                    toggle = Some((n, !current));
                }
                if ty == "submit" {
                    if let Some(form) = closest_tag(&s, n, "form") {
                        submit_form = Some(form);
                    }
                }
            }
            if tag == "button" {
                let ty = s.doc.attr(n, "type").unwrap_or("submit").to_ascii_lowercase();
                if ty == "submit" {
                    if let Some(form) = closest_tag(&s, n, "form") {
                        submit_form = Some(form);
                    }
                }
            }
            if tag == "form" {
                break;
            }
            cur = s.doc.parent(n);
            depth += 1;
            if depth > 200 {
                break;
            }
        }
    }
    if let Some((node, checked)) = toggle {
        let mut s = state.borrow_mut();
        s.checked.insert(node, checked);
        if checked {
            s.doc.set_attr(node, "checked", "");
        } else {
            s.doc.remove_attr(node, "checked");
        }
        s.mark_dirty();
    }
    if let Some(form) = submit_form {
        dispatch_event(it, state, form, "submit")?;
    }
    if let Some(url) = navigation {
        state.borrow_mut().navigation = Some(url);
    }
    Ok(())
}

fn closest_tag(state: &DomState, node: NodeId, tag: &str) -> Option<NodeId> {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if state.doc.tag_name(n) == tag {
            return Some(n);
        }
        cur = state.doc.parent(n);
    }
    None
}

pub fn records_to_array(
    it: &mut Interp,
    state: &Rc<RefCell<DomState>>,
    records: &[Mutation],
) -> Value {
    let mut out = Vec::new();
    for m in records {
        let obj = it.object();
        if let Some(o) = obj.as_object() {
            define(o, "type", Value::string(m.kind));
            define(o, "target", wrap(it, state, m.target));
            let added: Vec<Value> = m.added.iter().map(|n| wrap(it, state, *n)).collect();
            let removed: Vec<Value> = m.removed.iter().map(|n| wrap(it, state, *n)).collect();
            define(o, "addedNodes", it.array_from(added));
            define(o, "removedNodes", it.array_from(removed));
            define(
                o,
                "attributeName",
                m.attribute.as_ref().map(|a| Value::string(a.clone())).unwrap_or(Value::Null),
            );
        }
        out.push(obj);
    }
    it.array_from(out)
}

#[allow(dead_code)]
fn button_of(args: &[Value]) -> Value {
    arg(args, 0)
}
