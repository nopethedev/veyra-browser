//! Element and Document object methods/properties.

use std::cell::RefCell;
use std::rc::Rc;

use veyra_common::{Point, Rect};
use veyra_dom::{Document, NodeId, NodeType};
use veyra_html::parse_fragment;
use veyra_js::{Interp, JsError, ObjRef, Value};
use veyra_js::value::{define, define_accessor};

use crate::bindings::{arg, getset, getter, method, node_of, set_attribute, strict, wrap, wrappers};
use crate::event_api::dispatch_event;
use crate::state::{DomState, Listener, Mutation};

pub fn define_node_api(it: &mut Interp, obj: &ObjRef, state: &Rc<RefCell<DomState>>, node: NodeId) {
    // ShadyDOM stores its per-node logical-tree data at `node.__shady` and
    // assumes every DOM wrapper already has an object there when its
    // MutationObserver callback runs. Supplying the store at wrapper creation
    // keeps that contract without relying on the polyfill's patched getters.
    if !obj.borrow().props.contains_key("__shady") {
        define(obj, "__shady", it.object());
    }
    getter(it, obj, "nodeType", state, node, |_, st, n| {
        let s = st.borrow();
        let t = s.doc.node(n).map(|x| match &x.node_type {
            NodeType::Element(tag) if tag == "fragment" || tag == "#shadow-root" => 11,
            NodeType::Element(_) => 1,
            NodeType::Text(_) => 3,
            NodeType::Comment(_) => 8,
            NodeType::Document => 9,
            NodeType::Doctype(_) => 10,
        });
        Ok(Value::Number(t.unwrap_or(1) as f64))
    });
    getter(it, obj, "nodeName", state, node, |it, st, n| node_name(it, &st.borrow().doc, n));
    getter(it, obj, "tagName", state, node, |it, st, n| node_name(it, &st.borrow().doc, n));
    getter(it, obj, "localName", state, node, |_, st, n| {
        let s = st.borrow();
        let tag = s.doc.tag_name(n);
        if s.doc.is_element(n) && tag != "fragment" && tag != "#shadow-root" {
            Ok(Value::string(tag.to_ascii_lowercase()))
        } else {
            Ok(Value::Null)
        }
    });
    method(it, obj, "hasAttributes", 0, state, node, |_, st, n, _args| {
        Ok(Value::Bool(!st.borrow().doc.attrs(n).is_empty()))
    });
    getter(it, obj, "nodeValue", state, node, |_, st, n| {
        let s = st.borrow();
        Ok(s.doc
            .node(n)
            .map(|x| match &x.node_type {
                NodeType::Text(t) | NodeType::Comment(t) => Value::string(t.clone()),
                _ => Value::Null,
            })
            .unwrap_or(Value::Null))
    });
    getset(
        it,
        obj,
        "id",
        state,
        node,
        |_, st, n| Ok(Value::string(st.borrow().doc.attr(n, "id").unwrap_or("").to_string())),
        |_, st, n, v| { set_attribute(st, n, "id", &v.to_string_primitive()); Ok(()) },
    );
    getset(
        it,
        obj,
        "className",
        state,
        node,
        |_, st, n| Ok(Value::string(st.borrow().doc.attr(n, "class").unwrap_or("").to_string())),
        |_, st, n, v| { set_attribute(st, n, "class", &v.to_string_primitive()); Ok(()) },
    );
    getset(
        it,
        obj,
        "textContent",
        state,
        node,
        |_, st, n| Ok(Value::string(st.borrow().doc.text_content(n))),
        |_, st, n, v| {
            let text = v.to_string_primitive();
            let mut s = st.borrow_mut();
            s.doc.set_text_content(n, &text);
            s.note_mutation(Mutation { kind: "childList", target: n, added: vec![], removed: vec![], attribute: None });
            Ok(())
        },
    );
    getset(
        it,
        obj,
        "innerHTML",
        state,
        node,
        |_, st, n| Ok(Value::string(st.borrow().doc.inner_html(n))),
        |it, st, n, v| {
            let html = v.to_string_primitive();
            let (nodes, connected) = {
                let mut s = st.borrow_mut();
                for c in s.doc.children(n).to_vec() {
                    s.doc.remove(c);
                }
                let nodes = parse_fragment(&mut s.doc, n, &html);
                for c in &nodes {
                    s.doc.append(n, *c);
                }
                let connected = s.doc.is_in_document(n);
                s.note_mutation(Mutation { kind: "childList", target: n, added: nodes.clone(), removed: vec![], attribute: None });
                (nodes, connected)
            };
            // Parsed subtrees inserted by the innerHTML setter are upgraded and
            // connected exactly like nodes appended through DOM APIs.
            if connected {
                for c in nodes {
                    crate::bindings::connect_subtree(it, st, c);
                }
            }
            Ok(())
        },
    );
    getter(it, obj, "outerHTML", state, node, |_, st, n| {
        Ok(Value::string(st.borrow().doc.outer_html(n)))
    });
    getter(it, obj, "innerText", state, node, |_, st, n| {
        Ok(Value::string(st.borrow().doc.text_content(n)))
    });
    getter(it, obj, "outerText", state, node, |_, st, n| {
        Ok(Value::string(st.borrow().doc.text_content(n)))
    });
    getset(
        it,
        obj,
        "value",
        state,
        node,
        |_, st, n| {
            let s = st.borrow();
            if s.doc.tag_name(n) == "textarea" {
                return Ok(Value::string(s.doc.text_content(n)));
            }
            Ok(Value::string(
                s.form_values
                    .get(&n)
                    .cloned()
                    .or_else(|| s.doc.attr(n, "value").map(|v| v.to_string()))
                    .unwrap_or_default(),
            ))
        },
        |_, st, n, v| {
            let text = v.to_string_primitive();
            let mut s = st.borrow_mut();
            s.form_values.insert(n, text.clone());
            if s.doc.tag_name(n) == "textarea" {
                s.doc.set_text_content(n, &text);
            } else {
                s.doc.set_attr(n, "value", &text);
            }
            s.mark_dirty();
            Ok(())
        },
    );
    getset(
        it,
        obj,
        "checked",
        state,
        node,
        |_, st, n| {
            let s = st.borrow();
            let checked = s.checked.get(&n).copied().unwrap_or_else(|| s.doc.attr(n, "checked").is_some());
            Ok(Value::Bool(checked))
        },
        |_, st, n, v| {
            let mut s = st.borrow_mut();
            let checked = v.truthy();
            s.checked.insert(n, checked);
            if checked {
                s.doc.set_attr(n, "checked", "");
            } else {
                s.doc.remove_attr(n, "checked");
            }
            s.mark_dirty();
            Ok(())
        },
    );
    getset(
        it,
        obj,
        "disabled",
        state,
        node,
        |_, st, n| Ok(Value::Bool(st.borrow().doc.attr(n, "disabled").is_some())),
        |_, st, n, v| {
            let mut s = st.borrow_mut();
            if v.truthy() {
                s.doc.set_attr(n, "disabled", "");
            } else {
                s.doc.remove_attr(n, "disabled");
            }
            s.mark_dirty();
            Ok(())
        },
    );
    getter(it, obj, "href", state, node, |_, st, n| {
        let s = st.borrow();
        Ok(Value::string(s.doc.attr(n, "href").map(|v| s.resolve(v).to_string()).unwrap_or_default()))
    });
    getter(it, obj, "src", state, node, |_, st, n| {
        let s = st.borrow();
        Ok(Value::string(s.doc.attr(n, "src").map(|v| s.resolve(v).to_string()).unwrap_or_default()))
    });
    getset(
        it,
        obj,
        "href",
        state,
        node,
        |_, st, n| {
            let s = st.borrow();
            Ok(Value::string(s.doc.attr(n, "href").map(|v| s.resolve(v).to_string()).unwrap_or_default()))
        },
        |_, st, n, v| { set_attribute(st, n, "href", &v.to_string_primitive()); Ok(()) },
    );
    getset(
        it,
        obj,
        "src",
        state,
        node,
        |_, st, n| {
            let s = st.borrow();
            Ok(Value::string(s.doc.attr(n, "src").map(|v| s.resolve(v).to_string()).unwrap_or_default()))
        },
        |_, st, n, v| { set_attribute(st, n, "src", &v.to_string_primitive()); Ok(()) },
    );

    getter(it, obj, "children", state, node, |it, st, n| {
        let children = st.borrow().element_children(n);
        Ok(wrappers(it, st, children))
    });
    getter(it, obj, "childNodes", state, node, |it, st, n| {
        let children = st.borrow().doc.children(n).to_vec();
        Ok(wrappers(it, st, children))
    });
    getter(it, obj, "content", state, node, |it, st, n| {
        // A template's content is a real DocumentFragment holding inert
        // children. Returning the template itself makes Polymer's template
        // parsing recurse forever and breaks importNode/stamping.
        if st.borrow().doc.tag_name(n) != "template" {
            return Ok(Value::Undefined);
        }
        let frag = {
            let mut s = st.borrow_mut();
            if let Some(frag) = s.template_contents.get(&n).copied() {
                frag
            } else {
                let frag = s.doc.create_element("fragment");
                for child in s.doc.children(n).to_vec() {
                    let copy = clone_dom(&mut s.doc, child, true);
                    s.doc.append(frag, copy);
                }
                s.template_contents.insert(n, frag);
                frag
            }
        };
        Ok(wrap(it, st, frag))
    });
    getter(it, obj, "shadowRoot", state, node, |it, st, n| {
        let found = st.borrow().doc.shadow_root(n);
        match found {
            Some(root) if st.borrow().shadow_modes.get(&root).map(|m| m == "open").unwrap_or(true) => {
                Ok(wrap(it, st, root))
            }
            _ => Ok(Value::Null),
        }
    });
    getset(
        it,
        obj,
        "host",
        state,
        node,
        |it, st, n| {
            let host = st
                .borrow()
                .host_overrides
                .get(&n)
                .copied()
                .or_else(|| st.borrow().doc.host_of(n));
            Ok(host.map(|h| wrap(it, st, h)).unwrap_or(Value::Null))
        },
        |it, st, n, v| {
            if let Ok(host) = node_of(it, &v) {
                st.borrow_mut().host_overrides.insert(n, host);
                st.borrow_mut().doc.set_shadow_host(n, host);
            }
            Ok(())
        },
    );
    getter(it, obj, "mode", state, node, |_, st, n| {
        let mode = st.borrow().shadow_modes.get(&n).cloned().unwrap_or_else(|| "open".into());
        Ok(Value::string(mode))
    });
    method(it, obj, "attachShadow", 1, state, node, |it, st, n, args| {
        let init = arg(&args, 0);
        let mode = it
            .get_member(init, "mode")
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "open".into());
        let root = st.borrow_mut().doc.attach_shadow(n);
        st.borrow_mut().shadow_modes.insert(root, mode);
        st.borrow_mut().doc.distribute_slots();
        st.borrow_mut().mark_dirty();
        Ok(wrap(it, st, root))
    });
    method(it, obj, "assignedNodes", 1, state, node, |it, st, n, args| {
        let flatten = args.first().map(|v| v.truthy()).unwrap_or(false);
        let mut nodes = st.borrow().doc.slot_assignments.get(&n).cloned().unwrap_or_default();
        if flatten {
            let mut out = Vec::new();
            for id in nodes {
                let mut stack = vec![id];
                while let Some(cur) = stack.pop() {
                    out.push(cur);
                    let kids = st.borrow().doc.rendered_children(cur);
                    stack.extend(kids.into_iter().rev());
                }
            }
            nodes = out;
        }
        Ok(wrappers(it, st, nodes))
    });
    method(it, obj, "assignedElements", 1, state, node, |it, st, n, _| {
        let nodes: Vec<NodeId> = st
            .borrow()
            .doc
            .slot_assignments
            .get(&n)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|id| st.borrow().doc.node(*id).map(|x| x.is_element()).unwrap_or(false))
            .collect();
        Ok(wrappers(it, st, nodes))
    });
    getter(it, obj, "childElementCount", state, node, |_, st, n| {
        Ok(Value::Number(st.borrow().element_children(n).len() as f64))
    });
    for (name, which) in [
        ("parentNode", 0u8),
        ("parentElement", 0),
        ("firstChild", 2),
        ("lastChild", 3),
        ("nextSibling", 4),
        ("previousSibling", 5),
        ("firstElementChild", 6),
        ("lastElementChild", 7),
        ("nextElementSibling", 8),
        ("previousElementSibling", 9),
    ] {
        getter(it, obj, name, state, node, move |it, st, n| {
            // Drop the document borrow before `wrap`, which mutates the
            // wrapper cache.
            let found = {
                let s = st.borrow();
                match which {
                    0 => s.doc.parent(n),
                    2 => s.doc.children(n).first().copied(),
                    3 => s.doc.children(n).last().copied(),
                    4 => sibling(&s.doc, n, true),
                    5 => sibling(&s.doc, n, false),
                    6 => s.element_children(n).first().copied(),
                    7 => s.element_children(n).last().copied(),
                    8 => element_sibling(&s, n, true),
                    9 => element_sibling(&s, n, false),
                    _ => None,
                }
            };
            Ok(found.map(|c| wrap(it, st, c)).unwrap_or(Value::Null))
        });
    }
    getter(it, obj, "ownerDocument", state, node, |it, st, _| {
        // Release the document borrow before wrapping: `wrap` mutates the
        // wrapper cache and would otherwise hit an overlapping RefCell borrow.
        let root = st.borrow().doc.root;
        Ok(wrap(it, st, root))
    });
    getter(it, obj, "isConnected", state, node, |_, st, n| {
        Ok(Value::Bool(st.borrow().doc.is_in_document(n)))
    });
    getter(it, obj, "classList", state, node, |it, st, n| Ok(crate::style_api::make_class_list(it, st, n)));
    getter(it, obj, "style", state, node, |it, st, n| Ok(crate::style_api::make_style(it, st, n)));
    getter(it, obj, "dataset", state, node, |it, st, n| Ok(crate::style_api::make_dataset(it, st, n)));
    getter(it, obj, "attributes", state, node, |it, st, n| {
        let attrs = st.borrow().doc.attr_pairs(n).to_vec();
        let mut out = Vec::new();
        for (name, value) in attrs {
            let a = it.object();
            if let Some(o) = a.as_object() {
                let local = name.split_once(':').map(|(_, local)| local).unwrap_or(&name);
                veyra_js::value::define(o, "name", Value::string(name.clone()));
                veyra_js::value::define(o, "localName", Value::string(local.to_string()));
                veyra_js::value::define(o, "value", Value::string(value));
            }
            out.push(a);
        }
        let arr = it.array_from(out);
        if let Some(o) = arr.as_object().cloned() {
            define(&o, "item", it.native("item", 1, |it, this, args| {
                let i = arg(&args, 0).to_number();
                let len = it.get_member(this.clone(), "length")?.to_number();
                if !(0.0..len).contains(&i) { return Ok(Value::Null); }
                it.get_member(this, &(i as usize).to_string())
            }));
            define(&o, "getNamedItem", it.native("getNamedItem", 1, move |it, this, args| {
                let wanted = it.to_string(&arg(&args, 0))?;
                let len = it.get_member(this.clone(), "length")?.to_number() as usize;
                for i in 0..len {
                    let item = it.get_member(this.clone(), &i.to_string())?;
                    let name = it.get_member(item.clone(), "name").ok().and_then(|v| v.as_str().map(str::to_string));
                    if name.as_deref() == Some(wanted.as_str()) {
                        return Ok(item);
                    }
                }
                Ok(Value::Null)
            }));
            define(&o, "getNamedItemNS", it.native("getNamedItemNS", 2, move |it, this, args| {
                let wanted = it.to_string(&arg(&args, 1))?;
                let len = it.get_member(this.clone(), "length")?.to_number() as usize;
                for i in 0..len {
                    let item = it.get_member(this.clone(), &i.to_string())?;
                    let name = it.get_member(item.clone(), "localName").ok().and_then(|v| v.as_str().map(str::to_string));
                    if name.as_deref() == Some(wanted.as_str()) {
                        return Ok(item);
                    }
                }
                Ok(Value::Null)
            }));
        }
        Ok(arr)
    });
    for (name, field) in [
        ("offsetWidth", 0u8),
        ("offsetHeight", 1),
        ("offsetTop", 2),
        ("offsetLeft", 3),
        ("clientWidth", 4),
        ("clientHeight", 5),
        ("scrollWidth", 6),
        ("scrollHeight", 7),
        ("scrollTop", 8),
        ("scrollLeft", 9),
    ] {
        getter(it, obj, name, state, node, move |_, st, n| {
            let s = st.borrow();
            let rect = s.boxes.get(&n).copied().unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
            Ok(Value::Number(match field {
                0 | 4 | 6 => rect.w as f64,
                1 | 5 | 7 => rect.h as f64,
                2 => rect.y as f64,
                3 => rect.x as f64,
                _ => 0.0,
            }))
        });
    }

    method(it, obj, "getAttribute", 1, state, node, |it, st, n, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let v = st.borrow().doc.attr(n, &name).map(|v| v.to_string());
        Ok(v.map(Value::string).unwrap_or(Value::Null))
    });
    method(it, obj, "getAttributeNames", 0, state, node, |it, st, n, _| {
        let names: Vec<Value> = st
            .borrow()
            .doc
            .attrs(n)
            .iter()
            .map(|(k, _)| Value::string(k.clone()))
            .collect();
        Ok(it.array_from(names))
    });
    method(it, obj, "setAttribute", 2, state, node, |it, st, n, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let value = it.to_string(&arg(&args, 1))?;
        set_attribute(st, n, &name, &value);
        Ok(Value::Undefined)
    });
    method(it, obj, "removeAttribute", 1, state, node, |it, st, n, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let mut s = st.borrow_mut();
        let old = s.doc.attr(n, &name).map(|v| v.to_string());
        s.doc.remove_attr(n, &name);
        if s
            .observed_attributes
            .get(&n)
            .map(|attrs| attrs.iter().any(|a| a == &name))
            .unwrap_or(false)
        {
            s.attribute_changes.push((n, name.clone(), old, None));
        }
        s.note_mutation(Mutation {
            kind: "attributes",
            target: n,
            added: vec![],
            removed: vec![],
            attribute: Some(name),
        });
        Ok(Value::Undefined)
    });
    method(it, obj, "hasAttribute", 1, state, node, |it, st, n, args| {
        let name = it.to_string(&arg(&args, 0))?;
        Ok(Value::Bool(st.borrow().doc.attr(n, &name).is_some()))
    });
    method(it, obj, "toggleAttribute", 1, state, node, |it, st, n, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let mut s = st.borrow_mut();
        let has = s.doc.attr(n, &name).is_some();
        if has {
            s.doc.remove_attr(n, &name);
        } else {
            s.doc.set_attr(n, &name, "");
        }
        s.note_mutation(Mutation {
            kind: "attributes",
            target: n,
            added: vec![],
            removed: vec![],
            attribute: Some(name),
        });
        Ok(Value::Bool(!has))
    });
    method(it, obj, "appendChild", 1, state, node, |it, st, n, args| {
        let child = node_of(it, &arg(&args, 0))?;
        append_child(it, st, n, child)?;
        Ok(arg(&args, 0))
    });
    method(it, obj, "insertBefore", 2, state, node, |it, st, n, args| {
        let child = node_of(it, &arg(&args, 0))?;
        let reference = match args.get(1) {
            Some(Value::Null) | Some(Value::Undefined) | None => None,
            Some(v) => Some(node_of(it, v)?),
        };
        // Inserting a DocumentFragment moves its children, not the fragment.
        let fragment_children = {
            let s = st.borrow();
            if s.doc.tag_name(child) == "fragment" {
                Some(s.doc.children(child).to_vec())
            } else {
                None
            }
        };
        if let Some(children) = fragment_children {
            let connected = st.borrow().doc.is_in_document(n);
            for c in children {
                {
                    let mut s = st.borrow_mut();
                    s.doc.insert_before(n, c, reference);
                    s.note_mutation(Mutation { kind: "childList", target: n, added: vec![c], removed: vec![], attribute: None });
                }
                set_shady_parent(it, st, c, n);
                if connected {
                    crate::bindings::connect_subtree(it, st, c);
                    if st.borrow().doc.tag_name(c) == "script" {
                        st.borrow_mut().pending_scripts.push(c);
                    }
                }
            }
            return Ok(arg(&args, 0));
        }
        let connected = st.borrow().doc.is_in_document(n);
        {
            let mut s = st.borrow_mut();
            s.doc.insert_before(n, child, reference);
            s.note_mutation(Mutation { kind: "childList", target: n, added: vec![child], removed: vec![], attribute: None });
        }
        set_shady_parent(it, st, child, n);
        if connected && st.borrow().doc.tag_name(child) == "script" {
            st.borrow_mut().pending_scripts.push(child);
        }
        Ok(arg(&args, 0))
    });
    method(it, obj, "removeChild", 1, state, node, |it, st, n, args| {
        let child = node_of(it, &arg(&args, 0))?;
        retain_slotted_child(st, n, child);
        if st.borrow().doc.tag_name(child) == "script"
            && !st.borrow().executed_scripts.contains(&child)
            && st.borrow().doc.attr(child, "src").is_some()
        {
            st.borrow_mut().pending_scripts.push(child);
        }
        {
            let mut s = st.borrow_mut();
            s.doc.remove(child);
            s.note_mutation(Mutation { kind: "childList", target: n, added: vec![], removed: vec![child], attribute: None });
        }
        clear_shady_parent(it, st, child);
        crate::bindings::disconnect_subtree(it, st, child);
        Ok(arg(&args, 0))
    });
    method(it, obj, "replaceChild", 2, state, node, |it, st, n, args| {
        let new_child = node_of(it, &arg(&args, 0))?;
        let old = node_of(it, &arg(&args, 1))?;
        let mut s = st.borrow_mut();
        s.doc.replace_child(n, new_child, old);
        s.note_mutation(Mutation { kind: "childList", target: n, added: vec![new_child], removed: vec![old], attribute: None });
        drop(s);
        set_shady_parent(it, st, new_child, n);
        clear_shady_parent(it, st, old);
        Ok(arg(&args, 1))
    });
    method(it, obj, "append", 1, state, node, |it, st, n, args| {
        for v in args {
            if let Value::String(text) = &v {
                let mut s = st.borrow_mut();
                let t = s.doc.alloc(NodeType::Text(text.to_string()));
                s.doc.append(n, t);
                s.note_mutation(Mutation { kind: "childList", target: n, added: vec![t], removed: vec![], attribute: None });
            } else {
                let child = node_of(it, &v)?;
                append_child(it, st, n, child)?;
            }
        }
        Ok(Value::Undefined)
    });
    method(it, obj, "prepend", 1, state, node, |it, st, n, args| {
        let first = st.borrow().doc.children(n).first().copied();
        for v in args {
            let child = node_of(it, &v)?;
            {
                let mut s = st.borrow_mut();
                s.doc.insert_before(n, child, first);
                s.note_mutation(Mutation { kind: "childList", target: n, added: vec![child], removed: vec![], attribute: None });
            }
            set_shady_parent(it, st, child, n);
        }
        Ok(Value::Undefined)
    });
    method(it, obj, "remove", 0, state, node, |it, st, n, _| {
        {
            let mut s = st.borrow_mut();
            let parent = s.doc.parent(n);
            s.doc.remove(n);
            if let Some(p) = parent {
                s.note_mutation(Mutation { kind: "childList", target: p, added: vec![], removed: vec![n], attribute: None });
            }
        }
        crate::bindings::disconnect_subtree(it, st, n);
        Ok(Value::Undefined)
    });
    method(it, obj, "replaceWith", 1, state, node, |it, st, n, args| {
        let replacement = node_of(it, &arg(&args, 0))?;
        let mut s = st.borrow_mut();
        if let Some(parent) = s.doc.parent(n) {
            s.doc.replace_child(parent, replacement, n);
            s.note_mutation(Mutation { kind: "childList", target: parent, added: vec![replacement], removed: vec![n], attribute: None });
        }
        Ok(Value::Undefined)
    });
    method(it, obj, "before", 1, state, node, |it, st, n, args| {
        let v = arg(&args, 0);
        let sibling_node = node_of(it, &v)?;
        let mut s = st.borrow_mut();
        if let Some(parent) = s.doc.parent(n) {
            s.doc.insert_before(parent, sibling_node, Some(n));
            s.note_mutation(Mutation { kind: "childList", target: parent, added: vec![sibling_node], removed: vec![], attribute: None });
        }
        Ok(Value::Undefined)
    });
    method(it, obj, "after", 1, state, node, |it, st, n, args| {
        let v = arg(&args, 0);
        let sibling_node = node_of(it, &v)?;
        let mut s = st.borrow_mut();
        if let Some(parent) = s.doc.parent(n) {
            let next = sibling(&s.doc, n, true);
            match next {
                Some(next) => s.doc.insert_before(parent, sibling_node, Some(next)),
                None => s.doc.append(parent, sibling_node),
            }
            s.note_mutation(Mutation { kind: "childList", target: parent, added: vec![sibling_node], removed: vec![], attribute: None });
        }
        Ok(Value::Undefined)
    });
    method(it, obj, "addEventListener", 2, state, node, |it, st, n, args| {
        let event = it.to_string(&arg(&args, 0))?;
        let cb = arg(&args, 1);
        if cb.is_callable() {
            let mut once = false;
            if let Some(Value::Object(o)) = args.get(2) {
                once = it.get_member(Value::Object(o.clone()), "once").map(|v| v.truthy()).unwrap_or(false);
            }
            st.borrow_mut().listeners.push(Listener { node: n, event, callback: cb, once });
        }
        Ok(Value::Undefined)
    });
    method(it, obj, "removeEventListener", 2, state, node, |it, st, n, args| {
        let event = it.to_string(&arg(&args, 0))?;
        let cb = arg(&args, 1);
        st.borrow_mut()
            .listeners
            .retain(|l| !(l.node == n && l.event == event && strict(&l.callback, &cb)));
        Ok(Value::Undefined)
    });
    method(it, obj, "dispatchEvent", 1, state, node, |it, st, n, args| {
        let ev = arg(&args, 0);
        let prevented = crate::event_api::dispatch_event_object(it, st, n, "event", Some(ev))?;
        Ok(Value::Bool(!prevented))
    });
    method(it, obj, "click", 0, state, node, |it, st, n, _| {
        dispatch_event(it, st, n, "click")?;
        Ok(Value::Undefined)
    });
    method(it, obj, "cloneNode", 1, state, node, |it, st, n, args| {
        let deep = args.first().map(|v| v.truthy()).unwrap_or(false);
        let clone = {
            let mut s = st.borrow_mut();
            clone_dom(&mut s.doc, n, deep)
        };
        Ok(wrap(it, st, clone))
    });
    method(it, obj, "querySelector", 1, state, node, |it, st, n, args| {
        let sel = it.to_string(&arg(&args, 0))?;
        let found = st.borrow().doc.query_selector(n, &sel);
        Ok(found.map(|id| wrap(it, st, id)).unwrap_or(Value::Null))
    });
    method(it, obj, "querySelectorAll", 1, state, node, |it, st, n, args| {
        let sel = it.to_string(&arg(&args, 0))?;
        let nodes = st.borrow().doc.query_selector_all(n, &sel);
        Ok(wrappers(it, st, nodes))
    });
    method(it, obj, "closest", 1, state, node, |it, st, n, args| {
        let sel = it.to_string(&arg(&args, 0))?;
        let matches = st.borrow().doc.query_selector_all(st.borrow().doc.root, &sel);
        let mut cur = Some(n);
        while let Some(c) = cur {
            if matches.contains(&c) {
                return Ok(wrap(it, st, c));
            }
            cur = st.borrow().doc.parent(c);
        }
        Ok(Value::Null)
    });
    method(it, obj, "matches", 1, state, node, |it, st, n, args| {
        let sel = it.to_string(&arg(&args, 0))?;
        let m = st.borrow().doc.query_selector_all(st.borrow().doc.root, &sel).contains(&n);
        Ok(Value::Bool(m))
    });
    method(it, obj, "contains", 1, state, node, |it, st, n, args| {
        let other = node_of(it, &arg(&args, 0)).ok();
        let mut cur = other;
        while let Some(c) = cur {
            if c == n {
                return Ok(Value::Bool(true));
            }
            cur = st.borrow().doc.parent(c);
        }
        Ok(Value::Bool(false))
    });
    // ShadyDOM's noPatch wrapper calls these capture points directly. Native
    // DOM nodes expose them through the polyfill's method capture; mirror the
    // standard containment primitive so wrapped trees can stay connected.
    let st = state.clone();
    method(it, obj, "__shady_native_contains", 1, &st, node, |it, st, n, args| {
        let other = node_of(it, &arg(&args, 0)).ok();
        let mut cur = other;
        while let Some(c) = cur {
            if c == n {
                return Ok(Value::Bool(true));
            }
            cur = st.borrow().doc.parent(c);
        }
        Ok(Value::Bool(false))
    });
    method(it, obj, "getRootNode", 0, state, node, |it, st, n, _| {
        let mut cur = n;
        loop {
            let parent = { st.borrow().doc.parent(cur) };
            match parent {
                Some(p) => cur = p,
                None => break,
            }
        }
        Ok(wrap(it, st, cur))
    });
    method(it, obj, "getBoundingClientRect", 0, state, node, |it, st, n, _| {
        let rect = st.borrow().boxes.get(&n).copied().unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        let obj = it.object();
        if let Some(o) = obj.as_object() {
            for (k, v) in [
                ("x", rect.x),
                ("y", rect.y),
                ("left", rect.x),
                ("top", rect.y),
                ("right", rect.right()),
                ("bottom", rect.bottom()),
                ("width", rect.w),
                ("height", rect.h),
            ] {
                veyra_js::value::define(o, k, Value::Number(v as f64));
            }
        }
        Ok(obj)
    });
    method(it, obj, "getElementsByTagName", 1, state, node, |it, st, n, args| {
        let tag = it.to_string(&arg(&args, 0))?.to_lowercase();
        let tags = st.borrow().doc.query_selector_all(n, &tag);
        Ok(wrappers(it, st, tags))
    });
    method(it, obj, "getElementsByClassName", 1, state, node, |it, st, n, args| {
        let class = it.to_string(&arg(&args, 0))?;
        let sel = format!(".{}", class.split_whitespace().next().unwrap_or(""));
        let nodes = st.borrow().doc.query_selector_all(n, &sel);
        Ok(wrappers(it, st, nodes))
    });
    method(it, obj, "focus", 0, state, node, |_, st, n, _| {
        st.borrow_mut().focused = Some(n);
        Ok(Value::Undefined)
    });
    method(it, obj, "blur", 0, state, node, |_, st, n, _| {
        let mut s = st.borrow_mut();
        if s.focused == Some(n) {
            s.focused = None;
        }
        Ok(Value::Undefined)
    });
    method(it, obj, "scrollIntoView", 0, state, node, |_, _, _, _| Ok(Value::Undefined));
    method(it, obj, "animate", 1, state, node, |it, _, _, _| {
        let animation = it.object();
        if let Some(o) = animation.as_object() {
            define(o, "currentTime", Value::Number(0.0));
            define(o, "playState", Value::string("finished"));
            define(o, "onfinish", Value::Null);
            define(o, "oncancel", Value::Null);
            define(o, "effect", it.object());
            for name in ["play", "pause", "cancel", "finish", "reverse", "commitStyles", "persist"] {
                define(o, name, it.native(name, 0, |_, _, _| Ok(Value::Undefined)));
            }
            let (p, d) = it.new_promise();
            it.resolve_promise(&d, animation.clone());
            define(o, "finished", p);
        }
        Ok(animation)
    });
    method(it, obj, "getAnimations", 0, state, node, |it, _, _, _| Ok(it.array_from(vec![])));

    let tag = state.borrow().doc.tag_name(node).to_string();
    if tag == "canvas" {
        method(it, obj, "getContext", 1, state, node, |it, _, _, _| Ok(make_canvas_context(it)));
        method(it, obj, "toDataURL", 0, state, node, |_, _, _, _| {
            Ok(Value::string("data:image/png;base64,"))
        });
        method(it, obj, "toBlob", 1, state, node, |it, _, _, args| {
            let cb = arg(&args, 0);
            if cb.is_callable() {
                it.call_value(&cb, Value::Undefined, vec![Value::Null])?;
            }
            Ok(Value::Undefined)
        });
    }
}

/// A 2D canvas context with the drawing surface stubbed out. Applications can
/// query it, measure text and run their drawing calls without crashing; the
/// commands do not rasterize. Honest limitation: canvas pixels stay blank.
fn make_canvas_context(it: &mut Interp) -> Value {
    let obj = it.object();
    if let Some(o) = obj.as_object() {
        for (name, value) in [
            ("fillStyle", "#000000"),
            ("strokeStyle", "#000000"),
            ("lineWidth", "1"),
            ("font", "10px sans-serif"),
            ("textAlign", "start"),
            ("textBaseline", "alphabetic"),
            ("globalAlpha", "1"),
            ("globalCompositeOperation", "source-over"),
        ] {
            define(o, name, Value::string(value));
        }
        for name in [
            "save", "restore", "scale", "rotate", "translate", "transform", "setTransform",
            "resetTransform", "clearRect", "fillRect", "strokeRect", "beginPath", "closePath",
            "moveTo", "lineTo", "bezierCurveTo", "quadraticCurveTo", "arc", "arcTo", "rect",
            "ellipse", "fill", "stroke", "clip", "fillText", "strokeText", "drawImage",
            "putImageData", "setLineDash", "createPattern", "drawFocusIfNeeded",
        ] {
            define(o, name, it.native(name, 0, |_, _, _| Ok(Value::Undefined)));
        }
        define(o, "measureText", it.native("measureText", 1, |it, _, args| {
            let text = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            let width = text.chars().count() as f64 * 7.0;
            let m = it.object();
            if let Some(mo) = m.as_object() {
                define(mo, "width", Value::Number(width));
                define(mo, "actualBoundingBoxAscent", Value::Number(9.0));
                define(mo, "actualBoundingBoxDescent", Value::Number(3.0));
            }
            Ok(m)
        }));
        define(o, "getImageData", it.native("getImageData", 4, |it, _, args| {
            let w = arg(&args, 2).to_number().max(0.0) as usize;
            let h = arg(&args, 3).to_number().max(0.0) as usize;
            let data = it.array_from(vec![Value::Number(0.0); w.saturating_mul(h).saturating_mul(4).min(4_000_000)]);
            let out = it.object();
            if let Some(oo) = out.as_object() {
                define(oo, "data", data);
                define(oo, "width", Value::Number(w as f64));
                define(oo, "height", Value::Number(h as f64));
            }
            Ok(out)
        }));
        define(o, "createImageData", it.native("createImageData", 2, |it, _, args| {
            let w = arg(&args, 0).to_number().max(0.0) as usize;
            let h = arg(&args, 1).to_number().max(0.0) as usize;
            let data = it.array_from(vec![Value::Number(0.0); w.saturating_mul(h).saturating_mul(4).min(4_000_000)]);
            let out = it.object();
            if let Some(oo) = out.as_object() {
                define(oo, "data", data);
                define(oo, "width", Value::Number(w as f64));
                define(oo, "height", Value::Number(h as f64));
            }
            Ok(out)
        }));
        define(o, "createLinearGradient", it.native("createLinearGradient", 4, |it, _, _| {
            let g = it.object();
            if let Some(go) = g.as_object() {
                define(go, "addColorStop", it.native("addColorStop", 2, |_, _, _| Ok(Value::Undefined)));
            }
            Ok(g)
        }));
        define(o, "createRadialGradient", it.native("createRadialGradient", 6, |it, _, _| Ok(it.object())));
    }
    obj
}

pub fn define_document_api(it: &mut Interp, obj: &ObjRef, state: &Rc<RefCell<DomState>>, root: NodeId) {
    // `document.fonts` — FontFaceSet loading is not implemented, but callers
    // (Google's page is one) only await `load()` and inspect `status`.
    let fonts = it.object();
    if let Some(f) = fonts.as_object().cloned() {
        define(&f, "status", Value::string("loaded"));
        define(&f, "size", Value::Number(0.0));
        define(&f, "load", it.native("load", 1, |it, _, _| {
            let (p, d) = it.new_promise();
            let empty = it.array_from(vec![]);
            it.resolve_promise(&d, empty);
            Ok(p)
        }));
        define(&f, "check", it.native("check", 1, |_, _, _| Ok(Value::Bool(true))));
        define(&f, "add", it.native("add", 1, |_, _, _| Ok(Value::Undefined)));
        define(&f, "delete", it.native("delete", 1, |_, _, _| Ok(Value::Bool(false))));
        define(&f, "forEach", it.native("forEach", 1, |_, _, _| Ok(Value::Undefined)));
        let (p, d) = it.new_promise();
        it.resolve_promise(&d, fonts.clone());
        define(&f, "ready", p);
    }
    define(obj, "fonts", fonts);

    method(it, obj, "getElementById", 1, state, root, |it, st, _, args| {
        let id = it.to_string(&arg(&args, 0))?;
        let sel = format!("#{}", css_escape(&id));
        let found = st.borrow().doc.query_selector(st.borrow().doc.root, &sel);
        Ok(found.map(|n| wrap(it, st, n)).unwrap_or(Value::Null))
    });
    method(it, obj, "getElementsByTagName", 1, state, root, |it, st, _, args| {
        let tag = it.to_string(&arg(&args, 0))?.to_lowercase();
        let nodes = st.borrow().doc.query_selector_all(st.borrow().doc.root, &tag);
        Ok(wrappers(it, st, nodes))
    });
    method(it, obj, "getElementsByClassName", 1, state, root, |it, st, _, args| {
        let class = it.to_string(&arg(&args, 0))?;
        let sel = format!(".{}", class.split_whitespace().next().unwrap_or(""));
        let nodes = st.borrow().doc.query_selector_all(st.borrow().doc.root, &sel);
        Ok(wrappers(it, st, nodes))
    });
    method(it, obj, "createElement", 1, state, root, |it, st, _, args| {
        let tag = it.to_string(&arg(&args, 0))?.to_lowercase();
        let id = st.borrow_mut().doc.create_element(&tag);
        let wrapper = wrap(it, st, id);
        crate::bindings::upgrade_element(it, st, id);
        Ok(wrapper)
    });
    method(it, obj, "createElementNS", 2, state, root, |it, st, _, args| {
        let tag = it.to_string(&arg(&args, 1))?;
        let local = tag.rsplit(':').next().unwrap_or(&tag).to_lowercase();
        let id = st.borrow_mut().doc.create_element(&local);
        let wrapper = wrap(it, st, id);
        crate::bindings::upgrade_element(it, st, id);
        Ok(wrapper)
    });
    method(it, obj, "createComment", 1, state, root, |it, st, _, args| {
        let text = it.to_string(&arg(&args, 0))?;
        let id = st.borrow_mut().doc.alloc(NodeType::Comment(text));
        Ok(wrap(it, st, id))
    });
    method(it, obj, "createAttribute", 1, state, root, |it, _, _, _| {
        Ok(it.object())
    });
    method(it, obj, "getSelection", 0, state, root, |it, _, _, _| Ok(crate::bindings::make_selection(it)));
    method(it, obj, "hasFocus", 0, state, root, |_, _, _, _| Ok(Value::Bool(true)));
    getter(it, obj, "activeElement", state, root, |it, st, _| {
        let focused = st.borrow().focused;
        match focused {
            Some(n) => Ok(wrap(it, st, n)),
            None => {
                let body = st.borrow().doc.body();
                Ok(body.map(|n| wrap(it, st, n)).unwrap_or(Value::Null))
            }
        }
    });
    getter(it, obj, "visibilityState", state, root, |_, _, _| Ok(Value::string("visible")));
    getter(it, obj, "hidden", state, root, |_, _, _| Ok(Value::Bool(false)));
    method(it, obj, "createTextNode", 1, state, root, |it, st, _, args| {
        let text = it.to_string(&arg(&args, 0))?;
        let id = st.borrow_mut().doc.alloc(NodeType::Text(text));
        Ok(wrap(it, st, id))
    });
    method(it, obj, "createDocumentFragment", 0, state, root, |it, st, _, _| {
        let id = st.borrow_mut().doc.create_element("fragment");
        Ok(wrap(it, st, id))
    });
    method(it, obj, "elementFromPoint", 2, state, root, |it, st, _, args| {
        let x = arg(&args, 0).to_number() as f32;
        let y = arg(&args, 1).to_number() as f32;
        let mut best: Option<(usize, NodeId)> = None;
        {
            let s = st.borrow();
            for (n, r) in &s.boxes {
                if r.contains(Point::new(x, y)) && n.index() > 0 {
                    let mut depth = 0;
                    let mut cur = Some(*n);
                    while let Some(c) = cur {
                        depth += 1;
                        cur = s.doc.parent(c);
                    }
                    if best.map(|(bd, _)| depth > bd).unwrap_or(true) {
                        best = Some((depth, *n));
                    }
                }
            }
        }
        Ok(best.map(|(_, n)| wrap(it, st, n)).unwrap_or(Value::Null))
    });
    getter(it, obj, "documentElement", state, root, |it, st, _| {
        let n = st.borrow().doc.document_element();
        Ok(n.map(|n| wrap(it, st, n)).unwrap_or(Value::Null))
    });
    let implementation = it.object();
    if let Some(imp) = implementation.as_object().cloned() {
        let st = state.clone();
        define(&imp, "createHTMLDocument", it.native("createHTMLDocument", 1, move |it, _, args| {
            let title = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            Ok(make_inert_document(it, &st, &title))
        }));
        let st2 = state.clone();
        define(&imp, "createDocument", it.native("createDocument", 3, move |it, _, args| {
            let title = args.get(2).map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            Ok(make_inert_document(it, &st2, &title))
        }));
    }
    define(obj, "implementation", implementation);
    method(it, obj, "importNode", 2, state, root, |it, st, _, args| {
        let source = node_of(it, &arg(&args, 0))?;
        let deep = args.get(1).map(|v| v.truthy()).unwrap_or(false);
        let copy = {
            let mut s = st.borrow_mut();
            clone_dom(&mut s.doc, source, deep)
        };
        Ok(wrap(it, st, copy))
    });
    method(it, obj, "createTreeWalker", 4, state, root, |it, st, _, args| {
        let start = node_of(it, &arg(&args, 0))?;
        let what = args.get(1).map(|v| v.to_number() as u32).unwrap_or(0xFFFF_FFFF);
        let filter = arg(&args, 2);
        Ok(make_tree_walker(it, st, start, what, filter))
    });
    method(it, obj, "open", 0, state, root, |it, st, _, _| {
        Ok(wrap(it, st, st.borrow().doc.root))
    });
    method(it, obj, "close", 0, state, root, |_, _, _, _| Ok(Value::Undefined));
    method(it, obj, "write", 1, state, root, |it, st, _, args| {
        let text = it.to_string(&arg(&args, 0))?;
        if let Some(body) = st.borrow().doc.body() {
            let nodes = {
                let mut s = st.borrow_mut();
                let nodes = veyra_html::parse_fragment(&mut s.doc, body, &text);
                for node in &nodes {
                    s.doc.append(body, *node);
                }
                s.mark_dirty();
                nodes
            };
            for node in nodes {
                crate::bindings::connect_subtree(it, st, node);
            }
        }
        Ok(Value::Undefined)
    });
    getter(it, obj, "scripts", state, root, |it, st, n| {
        let nodes = st.borrow().doc.query_selector_all(n, "script");
        Ok(wrappers(it, st, nodes))
    });
    getter(it, obj, "head", state, root, |it, st, _| {
        let n = st.borrow().doc.head();
        Ok(n.map(|n| wrap(it, st, n)).unwrap_or(Value::Null))
    });
    getter(it, obj, "body", state, root, |it, st, _| {
        let n = st.borrow().doc.body();
        Ok(n.map(|n| wrap(it, st, n)).unwrap_or(Value::Null))
    });
    getset(
        it,
        obj,
        "cookie",
        state,
        root,
        |_, st, _| {
            let s = st.borrow();
            let value = s
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            Ok(Value::string(value))
        },
        |it, st, _, v| {
            let raw = it.to_string(&v)?;
            let pair = raw.split(';').next().unwrap_or("").trim();
            if let Some((name, value)) = pair.split_once('=') {
                let name = name.trim().to_string();
                let value = value.trim().to_string();
                let mut s = st.borrow_mut();
                if let Some(entry) = s.cookies.iter_mut().find(|(k, _)| *k == name) {
                    entry.1 = value;
                } else {
                    s.cookies.push((name, value));
                }
            }
            Ok(())
        },
    );
    getset(
        it,
        obj,
        "title",
        state,
        root,
        |_, st, _| Ok(Value::string(st.borrow().title.clone())),
        |_, st, _, v| {
            let t = v.to_string_primitive();
            let mut s = st.borrow_mut();
            s.title = t.clone();
            s.doc.title = t;
            Ok(())
        },
    );
    getter(it, obj, "readyState", state, root, |_, st, _| {
        Ok(Value::string(st.borrow().doc.ready_state.clone()))
    });
    getter(it, obj, "URL", state, root, |_, st, _| Ok(Value::string(st.borrow().base.to_string())));
    getter(it, obj, "location", state, root, |it, _, _| {
        Ok(it.get_global("location").unwrap_or(Value::Null))
    });
    getter(it, obj, "defaultView", state, root, |it, _, _| Ok(Value::Object(it.global.clone())));
    getter(it, obj, "styleSheets", state, root, |it, st, _| {
        let tags: Vec<NodeId> = {
            let s = st.borrow();
            s.doc
                .elements_in_document_order()
                .into_iter()
                .filter(|n| s.doc.tag_name(*n) == "style")
                .collect()
        };
        let mut sheets = Vec::new();
        for tag in tags {
            let sheet = it.object();
            if let Some(sh) = sheet.as_object().cloned() {
                let st2 = st.clone();
                define(&sh, "insertRule", it.native("insertRule", 1, move |it, _, args| {
                    let rule = it.to_string(&arg(&args, 0))?;
                    let mut s = st2.borrow_mut();
                    let existing = s.doc.text_content(tag);
                    let css = format!("{existing}\n{rule}");
                    s.doc.set_text_content(tag, &css);
                    s.mark_dirty();
                    Ok(Value::Number(0.0))
                }));
                define(&sh, "deleteRule", it.native("deleteRule", 1, |_, _, _| Ok(Value::Undefined)));
                define(&sh, "cssRules", it.array_from(vec![]));
            }
            sheets.push(sheet);
        }
        Ok(it.array_from(sheets))
    });
    method(it, obj, "write", 1, state, root, |it, st, _, args| {
        let html = it.to_string(&arg(&args, 0))?;
        let body = st.borrow().doc.body();
        if let Some(b) = body {
            let nodes = {
                let mut s = st.borrow_mut();
                let nodes = parse_fragment(&mut s.doc, b, &html);
                for n in &nodes {
                    s.doc.append(b, *n);
                }
                s.note_mutation(Mutation { kind: "childList", target: b, added: nodes.clone(), removed: vec![], attribute: None });
                nodes
            };
            for n in nodes {
                crate::bindings::connect_subtree(it, st, n);
            }
        }
        Ok(Value::Undefined)
    });
}

fn make_inert_document(it: &mut Interp, state: &Rc<RefCell<DomState>>, title: &str) -> Value {
    let (html, head, body) = {
        let mut s = state.borrow_mut();
        let html = s.doc.create_element("html");
        let head = s.doc.create_element("head");
        let body = s.doc.create_element("body");
        if !title.is_empty() {
            let title_node = s.doc.create_element("title");
            let text = s.doc.alloc(veyra_dom::NodeType::Text(title.to_string()));
            s.doc.append(title_node, text);
            s.doc.append(head, title_node);
        }
        s.doc.append(html, head);
        s.doc.append(html, body);
        (html, head, body)
    };
    let wrapper = wrap(it, state, html);
    if let Some(o) = wrapper.as_object().cloned() {
        define(&o, "documentElement", wrapper.clone());
        define(&o, "head", wrap(it, state, head));
        define(&o, "body", wrap(it, state, body));
        define(&o, "title", Value::string(title.to_string()));
        define(&o, "readyState", Value::string("complete"));
        let st = state.clone();
        define(&o, "createElement", it.native("createElement", 1, move |it, _, args| {
            let tag = it.to_string(&arg(&args, 0))?.to_lowercase();
            let id = st.borrow_mut().doc.create_element(&tag);
            Ok(wrap(it, &st, id))
        }));
        let st = state.clone();
        define(&o, "createElementNS", it.native("createElementNS", 2, move |it, _, args| {
            let tag = it.to_string(&arg(&args, 1))?;
            let id = st.borrow_mut().doc.create_element(&tag.to_lowercase());
            Ok(wrap(it, &st, id))
        }));
        let st = state.clone();
        define(&o, "createTextNode", it.native("createTextNode", 1, move |it, _, args| {
            let text = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            let id = st.borrow_mut().doc.alloc(veyra_dom::NodeType::Text(text));
            Ok(wrap(it, &st, id))
        }));
        let st = state.clone();
        define(&o, "createComment", it.native("createComment", 1, move |it, _, args| {
            let text = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            let id = st.borrow_mut().doc.alloc(veyra_dom::NodeType::Comment(text));
            Ok(wrap(it, &st, id))
        }));
    }
    wrapper
}

fn node_type_bit(doc: &Document, node: NodeId) -> u32 {
    use veyra_dom::NodeType;
    match doc.node(node).map(|n| &n.node_type) {
        Some(NodeType::Element(_)) => 1,
        Some(NodeType::Text(_)) => 4,
        Some(NodeType::Comment(_)) => 128,
        Some(NodeType::Document) => 256,
        Some(NodeType::Doctype(_)) => 512,
        None => 0,
    }
}

fn next_after(doc: &Document, node: NodeId, root: NodeId) -> Option<NodeId> {
    let mut cur = node;
    loop {
        if cur == root {
            return None;
        }
        let parent = doc.parent(cur)?;
        if let Some(idx) = doc.children(parent).iter().position(|c| *c == cur) {
            if let Some(next) = doc.children(parent).get(idx + 1) {
                return Some(*next);
            }
        }
        cur = parent;
    }
}

fn make_tree_walker(it: &mut Interp, state: &Rc<RefCell<DomState>>, root: NodeId, what: u32, filter: Value) -> Value {
    let walker = it.object();
    let Some(o) = walker.as_object().cloned() else { return walker };
    let current = std::rc::Rc::new(std::cell::RefCell::new(root));
    define(&o, "root", wrap(it, state, root));
    define(&o, "whatToShow", Value::Number(what as f64));
    define(&o, "filter", filter.clone());
    let next_current = current.clone();
    let st_next = state.clone();
    define(&o, "nextNode", it.native("nextNode", 0, move |it, _, _| {
        let mut cur = *next_current.borrow();
        let doc = st_next.borrow().doc.clone();
        loop {
            let candidate = if let Some(first) = doc.children(cur).first() {
                Some(*first)
            } else {
                next_after(&doc, cur, root)
            };
            let Some(candidate) = candidate else { return Ok(Value::Null) };
            cur = candidate;
            let bit = node_type_bit(&doc, candidate);
            if (what & bit) == 0 {
                continue;
            }
            if filter.is_callable() {
                let wrapped = wrap(it, &st_next, candidate);
                let result = it.call_value(&filter, Value::Undefined, vec![wrapped])?;
                let decision = result.to_number() as i32;
                if decision == 2 {
                    // FILTER_REJECT: skip this subtree.
                    cur = candidate;
                    if let Some(next) = next_after(&doc, candidate, root) {
                        cur = next;
                        let bit = node_type_bit(&doc, cur);
                        if (what & bit) != 0 {
                            *next_current.borrow_mut() = cur;
                            return Ok(wrap(it, &st_next, cur));
                        }
                        continue;
                    }
                    return Ok(Value::Null);
                }
                if decision != 1 {
                    continue;
                }
            }
            *next_current.borrow_mut() = candidate;
            return Ok(wrap(it, &st_next, candidate));
        }
    }));
    let st_parent = state.clone();
    let cur_parent = current.clone();
    define(&o, "parentNode", it.native("parentNode", 0, move |it, _, _| {
        let doc = st_parent.borrow().doc.clone();
        let cur = *cur_parent.borrow();
        match doc.parent(cur) {
            Some(p) if p != root => {
                *cur_parent.borrow_mut() = p;
                Ok(wrap(it, &st_parent, p))
            }
            _ => Ok(Value::Null),
        }
    }));
    let st_first = state.clone();
    let cur_first = current.clone();
    define(&o, "firstChild", it.native("firstChild", 0, move |it, _, _| {
        let doc = st_first.borrow().doc.clone();
        let cur = *cur_first.borrow();
        match doc.children(cur).first() {
            Some(c) => {
                *cur_first.borrow_mut() = *c;
                Ok(wrap(it, &st_first, *c))
            }
            None => Ok(Value::Null),
        }
    }));
    let st_last = state.clone();
    let cur_last = current.clone();
    define(&o, "lastChild", it.native("lastChild", 0, move |it, _, _| {
        let doc = st_last.borrow().doc.clone();
        let cur = *cur_last.borrow();
        match doc.children(cur).last() {
            Some(c) => {
                *cur_last.borrow_mut() = *c;
                Ok(wrap(it, &st_last, *c))
            }
            None => Ok(Value::Null),
        }
    }));
    let st_sib = state.clone();
    let cur_sib = current.clone();
    define(&o, "nextSibling", it.native("nextSibling", 0, move |it, _, _| {
        let doc = st_sib.borrow().doc.clone();
        let cur = *cur_sib.borrow();
        let Some(parent) = doc.parent(cur) else { return Ok(Value::Null) };
        let siblings = doc.children(parent);
        match siblings.iter().position(|n| *n == cur).and_then(|i| siblings.get(i + 1)) {
            Some(next) => {
                *cur_sib.borrow_mut() = *next;
                Ok(wrap(it, &st_sib, *next))
            }
            None => Ok(Value::Null),
        }
    }));
    let st_prev = state.clone();
    let cur_prev = current.clone();
    define(&o, "previousSibling", it.native("previousSibling", 0, move |it, _, _| {
        let doc = st_prev.borrow().doc.clone();
        let cur = *cur_prev.borrow();
        let Some(parent) = doc.parent(cur) else { return Ok(Value::Null) };
        let siblings = doc.children(parent);
        match siblings.iter().position(|n| *n == cur).and_then(|i| i.checked_sub(1)).and_then(|i| siblings.get(i)) {
            Some(prev) => {
                *cur_prev.borrow_mut() = *prev;
                Ok(wrap(it, &st_prev, *prev))
            }
            None => Ok(Value::Null),
        }
    }));
    let st_get = state.clone();
    let cur_get = current.clone();
    define_accessor(
        &o,
        "currentNode",
        Some(it.native("get currentNode", 0, move |it, _, _| Ok(wrap(it, &st_get, *cur_get.borrow())))),
        Some(it.native("set currentNode", 1, move |it, _, args| {
            if let Ok(n) = node_of(it, &arg(&args, 0)) {
                *current.borrow_mut() = n;
            }
            Ok(Value::Undefined)
        })),
        false,
    );
    walker
}

fn clone_dom(doc: &mut Document, node: NodeId, deep: bool) -> NodeId {
    use veyra_dom::NodeType;
    let kind = doc.node(node).map(|n| n.node_type.clone());
    let new_node = match kind {
        Some(NodeType::Text(text)) => doc.alloc(NodeType::Text(text)),
        Some(NodeType::Comment(text)) => doc.alloc(NodeType::Comment(text)),
        Some(NodeType::Doctype(text)) => doc.alloc(NodeType::Doctype(text)),
        Some(NodeType::Element(tag)) => {
            let id = doc.create_element(&tag);
            for (name, value) in doc.attrs(node).to_vec() {
                doc.set_attr(id, &name, &value);
            }
            id
        }
        Some(NodeType::Document) | None => doc.root,
    };
    if deep && new_node != doc.root {
        for child in doc.children(node).to_vec() {
            let child = clone_dom(doc, child, true);
            doc.append(new_node, child);
        }
    }
    new_node
}

fn find_slot(state: &Rc<RefCell<DomState>>, host: NodeId, name: &str) -> Option<NodeId> {
    let s = state.borrow();
    let mut stack = vec![host];
    while let Some(n) = stack.pop() {
        if s.doc.tag_name(n) == "slot" && s.doc.attr(n, "name").unwrap_or("") == name {
            return Some(n);
        }
        stack.extend(s.doc.children(n).iter().copied());
    }
    None
}

/// ShadyDOM removes a host's light children before stamping its template. A
/// child carrying a `slot` attribute belongs in the matching slot, so keep it
/// and insert it once the slot exists rather than losing the server-rendered
/// content.
fn retain_slotted_child(state: &Rc<RefCell<DomState>>, host: NodeId, child: NodeId) {
    let name = state.borrow().doc.attr(child, "slot").map(|v| v.to_string());
    let Some(name) = name else { return };
    if let Some(slot) = find_slot(state, host, &name) {
        let mut s = state.borrow_mut();
        s.doc.append(slot, child);
        s.note_mutation(Mutation { kind: "childList", target: slot, added: vec![child], removed: vec![], attribute: None });
        return;
    }
    state
        .borrow_mut()
        .pending_slotted
        .entry(host)
        .or_default()
        .push(child);
}

fn place_pending_slots(state: &Rc<RefCell<DomState>>, host: NodeId) {
    let pending = {
        let s = state.borrow();
        s.pending_slotted.get(&host).cloned().unwrap_or_default()
    };
    if pending.is_empty() {
        return;
    }
    let mut remaining = Vec::new();
    for child in pending {
        let name = state.borrow().doc.attr(child, "slot").unwrap_or_default().to_string();
        if let Some(slot) = find_slot(state, host, &name) {
            let mut s = state.borrow_mut();
            s.doc.append(slot, child);
            s.note_mutation(Mutation { kind: "childList", target: slot, added: vec![child], removed: vec![], attribute: None });
        } else {
            remaining.push(child);
        }
    }
    let mut s = state.borrow_mut();
    if remaining.is_empty() {
        s.pending_slotted.remove(&host);
    } else {
        s.pending_slotted.insert(host, remaining);
    }
}

fn set_shady_parent(it: &mut Interp, state: &Rc<RefCell<DomState>>, child: NodeId, parent: NodeId) {
    let child_value = wrap(it, state, child);
    let parent_value = wrap(it, state, parent);
    if let Some(obj) = child_value.as_object() {
        let shady = obj
            .borrow()
            .props
            .get("__shady")
            .and_then(|p| p.value.as_object().cloned());
        if let Some(shady) = shady {
            define(&shady, "parentNode", parent_value);
        }
    }
}

fn clear_shady_parent(it: &mut Interp, state: &Rc<RefCell<DomState>>, child: NodeId) {
    let child_value = wrap(it, state, child);
    if let Some(obj) = child_value.as_object() {
        let shady = obj
            .borrow()
            .props
            .get("__shady")
            .and_then(|p| p.value.as_object().cloned());
        if let Some(shady) = shady {
            define(&shady, "parentNode", Value::Undefined);
        }
    }
}

fn append_child(it: &mut Interp, state: &Rc<RefCell<DomState>>, parent: NodeId, child: NodeId) -> Result<(), JsError> {
    if child == parent {
        // Renderers that mount component output opportunistically can hand
        // back the host itself among its children. Appending it would be a
        // no-op in their bookkeeping; rejecting it aborts the whole mount.
        return Ok(());
    }
    // Inserting a DocumentFragment moves its children, not the fragment.
    let fragment_children = {
        let s = state.borrow();
        if s.doc.tag_name(child) == "fragment" {
            Some(s.doc.children(child).to_vec())
        } else {
            None
        }
    };
    if let Some(children) = fragment_children {
        for c in children {
            append_child(it, state, parent, c)?;
        }
        return Ok(());
    }
    {
        let mut s = state.borrow_mut();
        let mut cur = Some(parent);
        while let Some(c) = cur {
            if c == child {
                return Err(JsError::type_error("cannot append an ancestor"));
            }
            cur = s.doc.parent(c);
        }
        let was_connected = s.doc.is_in_document(child);
        s.doc.append(parent, child);
        s.note_mutation(Mutation { kind: "childList", target: parent, added: vec![child], removed: vec![], attribute: None });
        if was_connected && !s.doc.is_in_document(parent) {
            drop(s);
            crate::bindings::disconnect_subtree(it, state, child);
        }
    }
    set_shady_parent(it, state, child, parent);
    place_pending_slots(state, parent);
    if state.borrow().doc.is_in_document(parent) {
        crate::bindings::connect_subtree(it, state, child);
        // Dynamic script elements loaded by the page (Google's module loader,
        // JSONP, analytics, ...) execute once they are connected. The parser's
        // initial script batch is handled separately by `run_scripts`.
        let is_script = state.borrow().doc.tag_name(child) == "script";
        if is_script {
            state.borrow_mut().pending_scripts.push(child);
        }
    }
    Ok(())
}

fn sibling(doc: &Document, node: NodeId, next: bool) -> Option<NodeId> {
    let parent = doc.parent(node)?;
    let children = doc.children(parent);
    let idx = children.iter().position(|c| *c == node)?;
    if next {
        children.get(idx + 1).copied()
    } else if idx > 0 {
        children.get(idx - 1).copied()
    } else {
        None
    }
}

fn element_sibling(state: &DomState, node: NodeId, next: bool) -> Option<NodeId> {
    let parent = state.doc.parent(node)?;
    let children = state.element_children(parent);
    let idx = children.iter().position(|c| *c == node)?;
    if next {
        children.get(idx + 1).copied()
    } else if idx > 0 {
        children.get(idx - 1).copied()
    } else {
        None
    }
}

fn node_name(_it: &mut Interp, doc: &Document, node: NodeId) -> Result<Value, JsError> {
    let name = doc.node(node).map(|n| match &n.node_type {
        NodeType::Element(tag) => tag.to_uppercase(),
        NodeType::Text(_) => "#text".into(),
        NodeType::Comment(_) => "#comment".into(),
        NodeType::Document => "#document".into(),
        NodeType::Doctype(d) => format!("{d}"),
    });
    Ok(Value::string(name.unwrap_or_default()))
}

fn css_escape(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '\\' })
        .collect()
}

/// Installs the tree/query methods that polyfills capture from interfaces.
/// These implementations operate on the receiver node directly; they must not
/// forward through instance methods, because a polyfill may have replaced
/// those with wrappers that call back into the captured prototype method.
pub fn install_prototype_methods(it: &mut Interp, state: &Rc<RefCell<DomState>>) {
    let (node_proto, element_proto, document_proto, fragment_proto) = {
        let s = state.borrow();
        (
            s.interface_protos.get("Node").and_then(|v| v.as_object().cloned()),
            s.interface_protos.get("Element").and_then(|v| v.as_object().cloned()),
            s.interface_protos.get("Document").and_then(|v| v.as_object().cloned()),
            s.interface_protos.get("DocumentFragment").and_then(|v| v.as_object().cloned()),
        )
    };
    if let Some(obj) = node_proto {
        define(&obj, "appendChild", it.native("appendChild", 1, {
            let st = state.clone();
            move |it, this, args| {
                let parent = node_of(it, &this)?;
                let value = arg(&args, 0);
                let child = node_of(it, &value)?;
                append_child(it, &st, parent, child)?;
                Ok(value)
            }
        }));
        define(&obj, "insertBefore", it.native("insertBefore", 2, {
            let st = state.clone();
            move |it, this, args| {
                let parent = node_of(it, &this)?;
                let value = arg(&args, 0);
                let child = node_of(it, &value)?;
                let reference = match args.get(1) {
                    Some(Value::Null) | Some(Value::Undefined) | None => None,
                    Some(v) => Some(node_of(it, v)?),
                };
                let fragment_children = {
                    let s = st.borrow();
                    if s.doc.tag_name(child) == "fragment" {
                        Some(s.doc.children(child).to_vec())
                    } else {
                        None
                    }
                };
                if let Some(children) = fragment_children {
                    for c in children {
                        {
                            let mut s = st.borrow_mut();
                            s.doc.insert_before(parent, c, reference);
                            s.note_mutation(Mutation { kind: "childList", target: parent, added: vec![c], removed: vec![], attribute: None });
                        }
                        if st.borrow().doc.is_in_document(parent) {
                            crate::bindings::connect_subtree(it, &st, c);
                            if st.borrow().doc.tag_name(c) == "script" {
                                st.borrow_mut().pending_scripts.push(c);
                            }
                        }
                    }
                    return Ok(value);
                }
                let connected = st.borrow().doc.is_in_document(parent);
                {
                    let mut s = st.borrow_mut();
                    s.doc.insert_before(parent, child, reference);
                    s.note_mutation(Mutation { kind: "childList", target: parent, added: vec![child], removed: vec![], attribute: None });
                }
                if connected {
                    crate::bindings::connect_subtree(it, &st, child);
                    if st.borrow().doc.tag_name(child) == "script" {
                        st.borrow_mut().pending_scripts.push(child);
                    }
                }
                Ok(value)
            }
        }));
        define(&obj, "removeChild", it.native("removeChild", 1, {
            let st = state.clone();
            move |it, this, args| {
                let parent = node_of(it, &this)?;
                let value = arg(&args, 0);
                let child = node_of(it, &value)?;
                retain_slotted_child(&st, parent, child);
                if st.borrow().doc.tag_name(child) == "script"
                    && !st.borrow().executed_scripts.contains(&child)
                    && st.borrow().doc.attr(child, "src").is_some()
                {
                    st.borrow_mut().pending_scripts.push(child);
                }
                {
                    let mut s = st.borrow_mut();
                    s.doc.remove(child);
                    s.note_mutation(Mutation { kind: "childList", target: parent, added: vec![], removed: vec![child], attribute: None });
                }
                crate::bindings::disconnect_subtree(it, &st, child);
                Ok(value)
            }
        }));
        define(&obj, "replaceChild", it.native("replaceChild", 2, {
            let st = state.clone();
            move |it, this, args| {
                let parent = node_of(it, &this)?;
                let value = arg(&args, 0);
                let new_child = node_of(it, &value)?;
                let old = node_of(it, &arg(&args, 1))?;
                {
                    let mut s = st.borrow_mut();
                    s.doc.replace_child(parent, new_child, old);
                    s.note_mutation(Mutation { kind: "childList", target: parent, added: vec![new_child], removed: vec![old], attribute: None });
                }
                if st.borrow().doc.is_in_document(parent) {
                    crate::bindings::connect_subtree(it, &st, new_child);
                }
                Ok(value)
            }
        }));
    }
    if let Some(obj) = element_proto.clone() {
        define(&obj, "querySelector", it.native("querySelector", 1, {
            let st = state.clone();
            move |it, this, args| {
                let root = node_of(it, &this)?;
                let sel = it.to_string(&arg(&args, 0))?;
                let found = st.borrow().doc.query_selector(root, &sel);
                Ok(found.map(|id| wrap(it, &st, id)).unwrap_or(Value::Null))
            }
        }));
        define(&obj, "querySelectorAll", it.native("querySelectorAll", 1, {
            let st = state.clone();
            move |it, this, args| {
                let root = node_of(it, &this)?;
                let sel = it.to_string(&arg(&args, 0))?;
                let nodes = st.borrow().doc.query_selector_all(root, &sel);
                Ok(wrappers(it, &st, nodes))
            }
        }));
        define(&obj, "matches", it.native("matches", 1, {
            let st = state.clone();
            move |it, this, args| {
                let node = node_of(it, &this)?;
                let sel = it.to_string(&arg(&args, 0))?;
                let (root, matches) = {
                    let s = st.borrow();
                    (s.doc.root, s.doc.query_selector_all(s.doc.root, &sel))
                };
                let _ = root;
                Ok(Value::Bool(matches.contains(&node)))
            }
        }));
        define(&obj, "closest", it.native("closest", 1, {
            let st = state.clone();
            move |it, this, args| {
                let node = node_of(it, &this)?;
                let sel = it.to_string(&arg(&args, 0))?;
                let matches = {
                    let s = st.borrow();
                    s.doc.query_selector_all(s.doc.root, &sel)
                };
                let mut cur = Some(node);
                while let Some(c) = cur {
                    if matches.contains(&c) {
                        return Ok(wrap(it, &st, c));
                    }
                    cur = st.borrow().doc.parent(c);
                }
                Ok(Value::Null)
            }
        }));
    }
    for obj in [document_proto, fragment_proto].into_iter().flatten() {
        define(&obj, "querySelector", it.native("querySelector", 1, {
            let st = state.clone();
            move |it, this, args| {
                let root = node_of(it, &this)?;
                let sel = it.to_string(&arg(&args, 0))?;
                let found = st.borrow().doc.query_selector(root, &sel);
                Ok(found.map(|id| wrap(it, &st, id)).unwrap_or(Value::Null))
            }
        }));
        define(&obj, "querySelectorAll", it.native("querySelectorAll", 1, {
            let st = state.clone();
            move |it, this, args| {
                let root = node_of(it, &this)?;
                let sel = it.to_string(&arg(&args, 0))?;
                let nodes = st.borrow().doc.query_selector_all(root, &sel);
                Ok(wrappers(it, &st, nodes))
            }
        }));
    }
}
