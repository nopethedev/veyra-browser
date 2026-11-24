//! DOM, CSSOM and window bindings exposed to Veyra's JavaScript engine.

use std::cell::RefCell;
use std::rc::Rc;

use veyra_common::{Point, Size};
use veyra_dom::{Document, NodeId, NodeType};
use veyra_js::{Interp, JsError, ObjRef, Value};
use veyra_js::value::{array_values, define, define_accessor, define_with, new_object, own_prop, ObjKind, Prop};

use crate::dom_api;
use crate::event_api;
use crate::state::{DomState, Listener, Observer};
use crate::style_api;

pub fn install(it: &mut Interp, state: Rc<RefCell<DomState>>) {
    let root = state.borrow().doc.root;
    let document = wrap(it, &state, root);
    it.set_global("document", document.clone());

    // `new Image()` is how scripts preload images and beacons.
    let st = state.clone();
    // Blob is used for feature detection and for measuring serialised bodies;
    // pages only need its size/type surface in this engine.
    install_blob(it);
    it.set_global(
        "Image",
        it.native("Image", 2, move |it, _, args| {
            let node = { st.borrow_mut().doc.create_element("img") };
            if let Some(w) = args.first().filter(|v| !v.is_undefined()) {
                let w = it.to_string(w)?;
                set_attribute(&st, node, "width", &w);
            }
            if let Some(h) = args.get(1).filter(|v| !v.is_undefined()) {
                let h = it.to_string(h)?;
                set_attribute(&st, node, "height", &h);
            }
            Ok(wrap(it, &st, node))
        }),
    );
    // CSSOM utility namespace used by framework event/class helpers.
    let css = it.object();
    if let Some(css_obj) = css.as_object().cloned() {
        define(
            &css_obj,
            "escape",
            it.native("escape", 1, |it, _, args| {
                let raw = it.to_string(&arg(&args, 0))?;
                let mut out = String::new();
                for (i, ch) in raw.chars().enumerate() {
                    match ch {
                        '\0' => out.push('\u{FFFD}'),
                        c if (c as u32) < 0x20 || c as u32 == 0x7F => {
                            out.push_str(&format!("\\{:x} ", c as u32));
                        }
                        c if i == 0 && c.is_ascii_digit() => {
                            out.push_str(&format!("\\{:x} ", c as u32));
                        }
                        c if i == 1
                            && c.is_ascii_digit()
                            && raw.chars().next().map(|f| f == '-').unwrap_or(false) =>
                        {
                            out.push_str(&format!("\\{:x} ", c as u32));
                        }
                        c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => out.push(c),
                        c if (c as u32) >= 0x80 => out.push(c),
                        c => {
                            out.push('\\');
                            out.push(c);
                        }
                    }
                }
                Ok(Value::string(out))
            }),
        );
        define(
            &css_obj,
            "supports",
            it.native("supports", 2, |_, _, _| Ok(Value::Bool(true))),
        );
    }
    it.set_global("CSS", css);

    let xhr_ctor = it.native("XMLHttpRequest", 0, |it, _, _| Ok(make_xhr(it)));
    if let Some(o) = xhr_ctor.as_object() {
        define(o, "prototype", it.object());
    }
    it.set_global("XMLHttpRequest", xhr_ctor);

    // Element/HTML* constructors. Pages feature-detect these and older
    // polyfills patch their prototypes, so the names must exist even though
    // layout objects are plain wrappers rather than class instances.
    for (name, tag) in [
        ("Element", "div"),
        ("HTMLElement", "div"),
        ("HTMLDivElement", "div"),
        ("HTMLSpanElement", "span"),
        ("HTMLParagraphElement", "p"),
        ("HTMLAnchorElement", "a"),
        ("HTMLImageElement", "img"),
        ("HTMLInputElement", "input"),
        ("HTMLFormElement", "form"),
        ("HTMLButtonElement", "button"),
        ("HTMLCanvasElement", "canvas"),
        ("HTMLBodyElement", "body"),
        ("HTMLHeadElement", "head"),
        ("HTMLHtmlElement", "html"),
        ("HTMLScriptElement", "script"),
        ("HTMLStyleElement", "style"),
        ("HTMLTemplateElement", "template"),
        ("HTMLTextAreaElement", "textarea"),
        ("HTMLSelectElement", "select"),
        ("HTMLOptionElement", "option"),
        ("HTMLOptGroupElement", "optgroup"),
        ("HTMLDataListElement", "datalist"),
        ("HTMLLabelElement", "label"),
        ("HTMLFieldSetElement", "fieldset"),
        ("HTMLLegendElement", "legend"),
        ("HTMLOutputElement", "output"),
        ("HTMLProgressElement", "progress"),
        ("HTMLMeterElement", "meter"),
        ("HTMLUListElement", "ul"),
        ("HTMLOListElement", "ol"),
        ("HTMLLIElement", "li"),
        ("HTMLDListElement", "dl"),
        ("HTMLMenuElement", "menu"),
        ("HTMLQuoteElement", "blockquote"),
        ("HTMLPreElement", "pre"),
        ("HTMLBRElement", "br"),
        ("HTMLHRElement", "hr"),
        ("HTMLTableElement", "table"),
        ("HTMLTableCaptionElement", "caption"),
        ("HTMLTableColElement", "col"),
        ("HTMLTableSectionElement", "tbody"),
        ("HTMLTableRowElement", "tr"),
        ("HTMLTableCellElement", "td"),
        ("HTMLMediaElement", "video"),
        ("HTMLVideoElement", "video"),
        ("HTMLAudioElement", "audio"),
        ("HTMLSourceElement", "source"),
        ("HTMLTrackElement", "track"),
        ("HTMLPictureElement", "picture"),
        ("HTMLIFrameElement", "iframe"),
        ("HTMLObjectElement", "object"),
        ("HTMLParamElement", "param"),
        ("HTMLEmbedElement", "embed"),
        ("HTMLMapElement", "map"),
        ("HTMLAreaElement", "area"),
        ("HTMLMetaElement", "meta"),
        ("HTMLLinkElement", "link"),
        ("HTMLTitleElement", "title"),
        ("HTMLBaseElement", "base"),
        ("HTMLDialogElement", "dialog"),
        ("HTMLDetailsElement", "details"),
        ("HTMLSlotElement", "slot"),
        ("HTMLModElement", "ins"),
        ("HTMLTimeElement", "time"),
        ("HTMLDataElement", "data"),
        ("HTMLFrameSetElement", "frameset"),
        ("HTMLMarqueeElement", "marquee"),
        ("SVGElement", "svg"),
        ("HTMLUnknownElement", "div"),
    ] {
        let proto = it.object();
        let st = state.clone();
        let ctor = it.native(name, 0, move |it, this, _| {
            // `super()` in a custom element constructor must initialize the
            // element that is already being upgraded, not manufacture a second
            // detached element. Only allocate when there is no DOM receiver.
            if let Some(obj) = this.as_object() {
                if obj.borrow().props.contains_key("__node") {
                    return Ok(this);
                }
            }
            let node = { st.borrow_mut().doc.create_element(tag) };
            Ok(wrap(it, &st, node))
        });
        if let Some(o) = ctor.as_object() {
            define(o, "prototype", proto.clone());
        }
        {
            let mut s = state.borrow_mut();
            s.interface_protos.insert(name.to_string(), proto.clone());
            // Prefer the specific HTML*Element prototype for each tag; generic
            // Element/HTMLElement entries above must not win this lookup.
            if name.starts_with("HTML") && name != "HTMLUnknownElement" {
                s.element_protos.entry(tag.to_string()).or_insert(proto.clone());
            }
        }
        it.set_global(name, ctor);
    }
    // Interface constructors that feature-detection code reads from before
    // any instance exists. They need a prototype object just like V8's DOM
    // bindings; the instance methods themselves live on each node wrapper.
    let mut window_proto = None;
    for name in [
        "EventTarget",
        "CharacterData",
        "CDATASection",
        "ProcessingInstruction",
        "HTMLSlotElement",
        "ShadowRoot",
        "Window",
        "Range",
        "Selection",
    ] {
        let proto = it.object();
        let ctor = it.native(name, 0, |it, _, _| Ok(it.object()));
        if let Some(o) = ctor.as_object() {
            define(o, "prototype", proto.clone());
        }
        if name == "Window" {
            window_proto = proto.as_object().cloned();
        }
        {
            // The feature-detection constructor replaces the prototype object
            // created earlier. Keep the tag -> prototype table pointing at the
            // linked interface, or dynamically created elements (notably
            // <slot>) fall back to Object.prototype and miss all Node APIs.
            let mut s = state.borrow_mut();
            if name == "HTMLSlotElement" {
                s.element_protos.insert("slot".into(), proto.clone());
            }
            s.interface_protos.insert(name.to_string(), proto.clone());
        }
        it.set_global(name, ctor);
    }
    {
        let st = state.clone();
        let text_ctor = it.native("Text", 1, move |it, _, args| {
            let text = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            let node = st.borrow_mut().doc.alloc(NodeType::Text(text));
            Ok(wrap(it, &st, node))
        });
        let proto = it.object();
        if let Some(o) = text_ctor.as_object() {
            define(o, "prototype", proto.clone());
        }
        state.borrow_mut().interface_protos.insert("Text".into(), proto);
        it.set_global("Text", text_ctor);
        let st = state.clone();
        let comment_ctor = it.native("Comment", 1, move |it, _, args| {
            let text = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            let node = st.borrow_mut().doc.alloc(NodeType::Comment(text));
            Ok(wrap(it, &st, node))
        });
        let proto = it.object();
        if let Some(o) = comment_ctor.as_object() {
            define(o, "prototype", proto.clone());
        }
        state.borrow_mut().interface_protos.insert("Comment".into(), proto);
        it.set_global("Comment", comment_ctor);
        let st = state.clone();
        let fragment_ctor = it.native("DocumentFragment", 0, move |it, _, _| {
            let node = st.borrow_mut().doc.create_element("fragment");
            Ok(wrap(it, &st, node))
        });
        let proto = it.object();
        if let Some(o) = fragment_ctor.as_object() {
            define(o, "prototype", proto.clone());
        }
        state.borrow_mut().interface_protos.insert("DocumentFragment".into(), proto);
        it.set_global("DocumentFragment", fragment_ctor);
        let st = state.clone();
        let doc_ctor = it.native("Document", 0, move |it, _, _| {
            let root = st.borrow().doc.root;
            Ok(wrap(it, &st, root))
        });
        let proto = it.object();
        if let Some(o) = doc_ctor.as_object() {
            define(o, "prototype", proto.clone());
        }
        state.borrow_mut().interface_protos.insert("Document".into(), proto);
        it.set_global("Document", doc_ctor);
        let st2 = state.clone();
        let html_doc_ctor = it.native("HTMLDocument", 0, move |it, _, _| {
            let root = st2.borrow().doc.root;
            Ok(wrap(it, &st2, root))
        });
        let proto = it.object();
        if let Some(o) = html_doc_ctor.as_object() {
            define(o, "prototype", proto.clone());
        }
        state.borrow_mut().interface_protos.insert("HTMLDocument".into(), proto);
        it.set_global("HTMLDocument", html_doc_ctor);
    }

    install_dom_prototypes(it, &state);
    dom_api::install_prototype_methods(it, &state);
    refresh_wrapper_protos(&state);

    // DOM events. Dispatch only reads `type`, but page code constructs events
    // (and reads `detail`, `key`, coordinates) before dispatching them.
    for (name, extra) in [
        ("Event", ""),
        ("CustomEvent", "detail"),
        ("MouseEvent", "clientX,clientY,screenX,screenY,button,buttons"),
        ("KeyboardEvent", "key,code,keyCode,which,ctrlKey,shiftKey,altKey,metaKey,repeat"),
        ("FocusEvent", "relatedTarget"),
        ("InputEvent", "data,dataTransfer"),
        ("PointerEvent", "pointerId,pointerType,clientX,clientY"),
        ("UIEvent", "detail,view"),
        ("AnimationEvent", "animationName,elapsedTime,pseudoElement"),
        ("TransitionEvent", "propertyName,elapsedTime,pseudoElement"),
        ("ClipboardEvent", "clipboardData"),
        ("DragEvent", "dataTransfer"),
        ("WheelEvent", "deltaX,deltaY,deltaZ,deltaMode,clientX,clientY"),
        ("CompositionEvent", "data,locale"),
        ("TouchEvent", "touches,targetTouches,changedTouches,altKey,ctrlKey,metaKey,shiftKey"),
        ("ErrorEvent", "message,filename,lineno,colno,error"),
        ("MessageEvent", "data,origin,lastEventId,source,ports"),
        ("ProgressEvent", "lengthComputable,loaded,total"),
        ("PopStateEvent", "state"),
        ("HashChangeEvent", "oldURL,newURL"),
        ("StorageEvent", "key,oldValue,newValue,url,storageArea"),
        ("PageTransitionEvent", "persisted"),
        ("BeforeUnloadEvent", "returnValue"),
        ("SubmitEvent", "submitter"),
        ("FormDataEvent", "formData"),
        ("PromiseRejectionEvent", "promise,reason"),
        ("MediaQueryListEvent", "matches,media"),
        ("SecurityPolicyViolationEvent", "blockedURI,violatedDirective,effectiveDirective"),
        ("WebGLContextEvent", "statusMessage"),
    ] {
        let kind = name.to_string();
        let extra_fields: Vec<&str> = if extra.is_empty() { Vec::new() } else { extra.split(',').collect() };
        let ctor = it.native(name, 1, move |it, this, args| {
            let obj = match this.as_object() {
                Some(o) if it.constructing => Value::Object(o.clone()),
                _ => it.object(),
            };
            let event_type = match args.first() {
                Some(v) if !v.is_undefined() => it.to_string(v)?,
                _ => String::new(),
            };
            let mut bubbles = false;
            let mut cancelable = false;
            let mut composed = false;
            if let Some(init) = args.get(1).and_then(|v| v.as_object()) {
                let iv = Value::Object(init.clone());
                bubbles = it.get_member(iv.clone(), "bubbles").map(|v| v.truthy()).unwrap_or(false);
                cancelable = it.get_member(iv.clone(), "cancelable").map(|v| v.truthy()).unwrap_or(false);
                composed = it.get_member(iv.clone(), "composed").map(|v| v.truthy()).unwrap_or(false);
                if let Some(o) = obj.as_object() {
                    for field in &extra_fields {
                        if let Ok(v) = it.get_member(iv.clone(), field) {
                            if !v.is_undefined() {
                                define(o, field, v);
                            }
                        }
                    }
                }
            }
            if let Some(o) = obj.as_object() {
                define(o, "type", Value::string(event_type));
                define(o, "bubbles", Value::Bool(bubbles));
                define(o, "cancelable", Value::Bool(cancelable));
                define(o, "composed", Value::Bool(composed));
                define(o, "defaultPrevented", Value::Bool(false));
                define(o, "isTrusted", Value::Bool(true));
                define(o, "target", Value::Null);
                define(o, "currentTarget", Value::Null);
                define(o, "timeStamp", Value::Number(0.0));
                define(o, "preventDefault", it.native("preventDefault", 0, |it, this, _| {
                    it.set_member(this, "defaultPrevented", Value::Bool(true))?;
                    Ok(Value::Undefined)
                }));
                define(o, "stopPropagation", it.native("stopPropagation", 0, |_, _, _| Ok(Value::Undefined)));
                define(o, "stopImmediatePropagation", it.native("stopImmediatePropagation", 0, |_, _, _| Ok(Value::Undefined)));
                define(o, "composedPath", it.native("composedPath", 0, |it, _, _| Ok(it.array_from(vec![]))));
            }
            let _ = &kind;
            Ok(obj)
        });
        if let Some(o) = ctor.as_object() {
            define(o, "prototype", it.object());
        }
        it.set_global(name, ctor);
    }

    // AbortController/AbortSignal: fetch, streams and app boot code use these
    // to cancel work and to feature-detect modern browser APIs.
    let make_signal = |it: &mut Interp, aborted: bool| -> (Value, Rc<RefCell<Vec<Value>>>) {
        let signal = it.object();
        let listeners: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));
        if let Some(o) = signal.as_object().cloned() {
            define(&o, "aborted", Value::Bool(aborted));
            define(&o, "reason", Value::Undefined);
            define(&o, "onabort", Value::Null);
            let ls = listeners.clone();
            define(&o, "addEventListener", it.native("addEventListener", 2, move |_, _, args| {
                if let Some(cb) = args.get(1) {
                    ls.borrow_mut().push(cb.clone());
                }
                Ok(Value::Undefined)
            }));
            let ls = listeners.clone();
            define(&o, "removeEventListener", it.native("removeEventListener", 2, move |_, _, args| {
                if let Some(cb) = args.get(1) {
                    ls.borrow_mut().retain(|c| !strict(c, cb));
                }
                Ok(Value::Undefined)
            }));
            let ls = listeners.clone();
            define(&o, "dispatchEvent", it.native("dispatchEvent", 1, move |it, _, _| {
                let cbs = ls.borrow().clone();
                for cb in cbs {
                    let _ = it.call_value(&cb, Value::Undefined, vec![Value::Undefined]);
                }
                Ok(Value::Bool(true))
            }));
            define(&o, "throwIfAborted", it.native("throwIfAborted", 0, |it, _, _| {
                Ok(Value::Undefined)
            }));
        }
        (signal, listeners)
    };
    {
        let (signal, listeners) = make_signal(it, false);
        let signal_for_abort = signal.clone();
        let controller = it.object();
        if let Some(co) = controller.as_object() {
            define(co, "signal", signal.clone());
            let ls = listeners.clone();
            define(co, "abort", it.native("abort", 1, move |it, _, args| {
                if let Some(so) = signal_for_abort.as_object() {
                    define(so, "aborted", Value::Bool(true));
                    define(so, "reason", args.first().cloned().unwrap_or(Value::Undefined));
                    if let Ok(onabort) = it.get_member(signal_for_abort.clone(), "onabort") {
                        if onabort.is_callable() {
                            let _ = it.call_value(&onabort, signal_for_abort.clone(), vec![Value::Undefined]);
                        }
                    }
                }
                let cbs = ls.borrow().clone();
                for cb in cbs {
                    let _ = it.call_value(&cb, Value::Undefined, vec![Value::Undefined]);
                }
                Ok(Value::Undefined)
            }));
        }
        it.set_global("AbortController", it.native("AbortController", 0, {
            let controller = controller.clone();
            move |_, _, _| Ok(controller.clone())
        }));
        let signal_ctor = it.native("AbortSignal", 0, move |it, _, _| {
            let (signal, _) = make_signal(it, false);
            Ok(signal)
        });
        if let Some(so) = signal_ctor.as_object() {
            let proto = it.object();
            if let Some(po) = proto.as_object() {
                define(po, "throwIfAborted", it.native("throwIfAborted", 0, |it, this, _| {
                    let aborted = it.get_member(this, "aborted").map(|v| v.truthy()).unwrap_or(false);
                    if aborted {
                        return Err(JsError::throw(Value::string("AbortError")));
                    }
                    Ok(Value::Undefined)
                }));
                define(po, "aborted", Value::Bool(false));
                define(po, "reason", Value::Undefined);
            }
            define(so, "prototype", proto);
            define(so, "abort", it.native("abort", 1, move |it, _, args| {
                let (signal, _) = make_signal(it, true);
                if let Some(o) = signal.as_object() {
                    define(o, "reason", args.first().cloned().unwrap_or(Value::Undefined));
                }
                Ok(signal)
            }));
            define(so, "timeout", it.native("timeout", 1, move |it, _, _| {
                let (signal, _) = make_signal(it, false);
                Ok(signal)
            }));
            define(so, "any", it.native("any", 1, move |it, _, _| {
                let (signal, _) = make_signal(it, false);
                Ok(signal)
            }));
        }
        it.set_global("AbortSignal", signal_ctor);
    }

    // TreeWalker / NodeFilter are used by DOM polyfills and libraries for
    // traversal. Constants must match the DOM specification bit values.
    let node_filter = it.object();
    if let Some(nf) = node_filter.as_object() {
        for (name, value) in [
            ("FILTER_ACCEPT", 1.0),
            ("FILTER_REJECT", 2.0),
            ("FILTER_SKIP", 3.0),
            ("SHOW_ALL", 4294967295.0),
            ("SHOW_ELEMENT", 1.0),
            ("SHOW_ATTRIBUTE", 2.0),
            ("SHOW_TEXT", 4.0),
            ("SHOW_CDATA_SECTION", 8.0),
            ("SHOW_ENTITY_REFERENCE", 16.0),
            ("SHOW_ENTITY", 32.0),
            ("SHOW_PROCESSING_INSTRUCTION", 64.0),
            ("SHOW_COMMENT", 128.0),
            ("SHOW_DOCUMENT", 256.0),
            ("SHOW_DOCUMENT_TYPE", 512.0),
            ("SHOW_DOCUMENT_FRAGMENT", 1024.0),
            ("SHOW_NOTATION", 2048.0),
        ] {
            define(nf, name, Value::Number(value));
        }
    }
    it.set_global("NodeFilter", node_filter);

    let window = Value::Object(it.global.clone());
    it.set_global("window", window.clone());
    it.set_global("self", window.clone());

    let g = it.global.clone();
    let st = state.clone();
    let add = it.native("addEventListener", 2, move |it, _, args| {
        let event = it.to_string(&arg(&args, 0))?;
        let cb = arg(&args, 1);
        if cb.is_callable() {
            let once = args
                .get(2)
                .and_then(|o| o.as_object().map(|_| it.get_member(o.clone(), "once").ok().map(|v| v.truthy())))
                .flatten()
                .unwrap_or(false);
            st.borrow_mut()
                .listeners
                .push(Listener { node: root, event, callback: cb, once });
        }
        Ok(Value::Undefined)
    });
    define(&g, "addEventListener", add.clone());
    let st = state.clone();
    let remove = it.native("removeEventListener", 2, move |it, _, args| {
        let event = it.to_string(&arg(&args, 0))?;
        let cb = arg(&args, 1);
        st.borrow_mut()
            .listeners
            .retain(|l| !(l.node == root && l.event == event && strict(&l.callback, &cb)));
        Ok(Value::Undefined)
    });
    define(&g, "removeEventListener", remove.clone());
    let st = state.clone();
    let dispatch = it.native("dispatchEvent", 1, move |it, _, args| {
        let event_type = it
            .get_member(arg(&args, 0), "type")
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "event".to_string());
        let prevented = event_api::dispatch_event(it, &st, st.borrow().doc.root, &event_type)?;
        Ok(Value::Bool(!prevented))
    });
    define(&g, "dispatchEvent", dispatch.clone());
    if let Some(wp) = window_proto.clone() {
        define(&wp, "addEventListener", add);
        define(&wp, "removeEventListener", remove);
        define(&wp, "dispatchEvent", dispatch);
        // The global object is an instance of Window: its prototype chain
        // must expose Window.prototype, as it does in V8.
        it.global.borrow_mut().proto = Some(wp);
    }

    // Element/HTML constructor block is installed above; getComputedStyle is
    // installed later together with the selection helper.

    let st = state.clone();
    define(&g, "matchMedia", it.native("matchMedia", 1, move |it, _, args| {
        let q = it.to_string(&arg(&args, 0))?;
        let (vp, dark) = {
            let s = st.borrow();
            (s.viewport, s.media_dark)
        };
        let m = style_api::media_matches(&q, vp, dark);
        let obj = it.object();
        if let Some(o) = obj.as_object() {
            define(o, "matches", Value::Bool(m));
            define(o, "media", Value::string(q));
            define(o, "onchange", Value::Null);
            for name in ["addListener", "removeListener", "addEventListener", "removeEventListener"] {
                define(o, name, it.native(name, 1, |_, _, _| Ok(Value::Undefined)));
            }
        }
        Ok(obj)
    }));
    define(&g, "requestAnimationFrame", it.native("requestAnimationFrame", 1, |it, _, args| {
        let cb = arg(&args, 0);
        let now = it.now_ms();
        Ok(Value::Number(it.add_timer(cb, 16.0, false, vec![Value::Number(now + 16.0)])))
    }));
    define(&g, "cancelAnimationFrame", it.native("cancelAnimationFrame", 1, |it, _, args| {
        it.clear_timer(arg(&args, 0).to_number());
        Ok(Value::Undefined)
    }));
    define(&g, "setInterval", it.native("setInterval", 2, |it, _, args| {
        let cb = arg(&args, 0);
        let delay = arg(&args, 1).to_number();
        Ok(Value::Number(it.add_timer(cb, delay, true, args.iter().skip(2).cloned().collect())))
    }));
    define(&g, "clearInterval", it.native("clearInterval", 1, |it, _, args| {
        it.clear_timer(arg(&args, 0).to_number());
        Ok(Value::Undefined)
    }));
    define(&g, "clearTimeout", it.native("clearTimeout", 1, |it, _, args| {
        it.clear_timer(arg(&args, 0).to_number());
        Ok(Value::Undefined)
    }));

    let st = state.clone();
    let width = it.native("innerWidth", 0, move |_, _, _| Ok(Value::Number(st.borrow().viewport.w as f64)));
    let st = state.clone();
    let height = it.native("innerHeight", 0, move |_, _, _| Ok(Value::Number(st.borrow().viewport.h as f64)));
    define_accessor(&g, "innerWidth", Some(width), None, false);
    define_accessor(&g, "innerHeight", Some(height), None, false);
    let st = state.clone();
    define(&g, "outerWidth", it.native("outerWidth", 0, move |_, _, _| Ok(Value::Number(st.borrow().viewport.w as f64))));
    let st = state.clone();
    define(&g, "outerHeight", it.native("outerHeight", 0, move |_, _, _| Ok(Value::Number(st.borrow().viewport.h as f64))));
    define(&g, "devicePixelRatio", Value::Number(1.0));
    let st = state.clone();
    let sx = it.native("scrollX", 0, move |_, _, _| Ok(Value::Number(st.borrow().scroll.x as f64)));
    let st = state.clone();
    let sy = it.native("scrollY", 0, move |_, _, _| Ok(Value::Number(st.borrow().scroll.y as f64)));
    define_accessor(&g, "scrollX", Some(sx), None, false);
    define_accessor(&g, "scrollY", Some(sy), None, false);
    let st = state.clone();
    define(&g, "scrollTo", it.native("scrollTo", 2, move |it, _, args| {
        let x = it.to_number_value(&arg(&args, 0)).unwrap_or(0.0);
        let y = it.to_number_value(&arg(&args, 1)).unwrap_or(0.0);
        st.borrow_mut().scroll = Point::new(x as f32, y as f32);
        Ok(Value::Undefined)
    }));
    let st = state.clone();
    define(&g, "scrollBy", it.native("scrollBy", 2, move |it, _, args| {
        let x = it.to_number_value(&arg(&args, 0)).unwrap_or(0.0) as f32;
        let y = it.to_number_value(&arg(&args, 1)).unwrap_or(0.0) as f32;
        let mut s = st.borrow_mut();
        s.scroll = Point::new(s.scroll.x + x, s.scroll.y + y);
        Ok(Value::Undefined)
    }));
    define(&g, "alert", it.native("alert", 1, |it, _, args| {
        let msg = it.to_string(&arg(&args, 0))?;
        it.output.push(format!("alert: {msg}"));
        Ok(Value::Undefined)
    }));
    define(&g, "confirm", it.native("confirm", 1, |_, _, _| Ok(Value::Bool(true))));
    define(&g, "prompt", it.native("prompt", 1, |_, _, _| Ok(Value::Null)));

    let location = make_location(it, &state);
    it.set_global("location", location.clone());
    define(&g, "location", location);
    for name in ["top", "parent"] {
        define(&g, name, Value::Object(g.clone()));
    }

    let history = it.object();
    if let Some(h) = history.as_object() {
        define(h, "length", Value::Number(1.0));
        for name in ["pushState", "replaceState"] {
            define(h, name, it.native(name, 2, |_, _, _| Ok(Value::Undefined)));
        }
        define(h, "back", it.native("back", 0, |_, _, _| Ok(Value::Undefined)));
        define(h, "forward", it.native("forward", 0, |_, _, _| Ok(Value::Undefined)));
        define(h, "go", it.native("go", 1, |_, _, _| Ok(Value::Undefined)));
    }
    it.set_global("history", history);

    let nav = it.object();
    if let Some(n) = nav.as_object() {
        // Clipboard, permissions and service workers are exposed with honest
        // no-op behaviour so feature detection succeeds instead of throwing.
        let clipboard = it.object();
        if let Some(c) = clipboard.as_object() {
            define(c, "readText", it.native("readText", 0, |it, _, _| {
                let (p, d) = it.new_promise();
                it.resolve_promise(&d, Value::string(""));
                Ok(p)
            }));
            define(c, "writeText", it.native("writeText", 1, |it, _, _| {
                let (p, d) = it.new_promise();
                it.resolve_promise(&d, Value::Undefined);
                Ok(p)
            }));
        }
        define(n, "clipboard", clipboard);
        let permissions = it.object();
        if let Some(p) = permissions.as_object() {
            define(p, "query", it.native("query", 1, |it, _, _| {
                let state = it.object();
                if let Some(o) = state.as_object() {
                    define(o, "state", Value::string("granted"));
                    define(o, "onchange", Value::Null);
                }
                let (promise, d) = it.new_promise();
                it.resolve_promise(&d, state);
                Ok(promise)
            }));
        }
        define(n, "permissions", permissions);
        let sw = it.object();
        if let Some(s) = sw.as_object() {
            define(s, "controller", Value::Null);
            define(s, "ready", {
                let (p, d) = it.new_promise();
                it.resolve_promise(&d, it.object());
                p
            });
            define(s, "register", it.native("register", 1, |it, _, _| {
                let (p, d) = it.new_promise();
                it.resolve_promise(&d, it.object());
                Ok(p)
            }));
            define(s, "getRegistrations", it.native("getRegistrations", 0, |it, _, _| {
                let (p, d) = it.new_promise();
                it.resolve_promise(&d, it.array_from(vec![]));
                Ok(p)
            }));
        }
        define(n, "serviceWorker", sw);
        define(n, "userAgent", Value::string(veyra_common::USER_AGENT));
        define(
            n,
            "appVersion",
            Value::string(
                veyra_common::USER_AGENT
                    .strip_prefix("Mozilla/")
                    .unwrap_or(veyra_common::USER_AGENT),
            ),
        );
        define(n, "vendor", Value::string("Google Inc."));
        define(n, "language", Value::string("en-US"));
        define(n, "languages", it.array_from(vec![Value::string("en-US"), Value::string("en")]));
        define(n, "platform", Value::string("Linux x86_64"));
        define(n, "onLine", Value::Bool(true));
        define(n, "cookieEnabled", Value::Bool(true));
        define(n, "hardwareConcurrency", Value::Number(8.0));
    }
    it.set_global("navigator", nav);

    for (name, session) in [("localStorage", false), ("sessionStorage", true)] {
        let storage = make_storage(it, &state, session);
        it.set_global(name, storage);
    }

    let perf = it.object();
    if let Some(p) = perf.as_object() {
        define(p, "now", it.native("now", 0, |it, _, _| Ok(Value::Number(it.now_ms()))));
        define(p, "timeOrigin", Value::Number(0.0));
        for name in ["mark", "measure", "clearMarks", "clearMeasures"] {
            define(p, name, it.native(name, 1, |_, _, _| Ok(Value::Undefined)));
        }
        for name in ["getEntries", "getEntriesByType", "getEntriesByName"] {
            define(p, name, it.native(name, 1, |it, _, _| Ok(it.array_from(vec![]))));
        }
        let timing = it.object();
        if let Some(t) = timing.as_object() {
            for (name, value) in [
                ("navigationStart", 0.0),
                ("fetchStart", 0.0),
                ("domainLookupStart", 0.0),
                ("domainLookupEnd", 0.0),
                ("connectStart", 0.0),
                ("connectEnd", 0.0),
                ("requestStart", 0.0),
                ("responseStart", 1.0),
                ("responseEnd", 2.0),
                ("domLoading", 2.0),
                ("domInteractive", 3.0),
                ("domContentLoadedEventStart", 3.0),
                ("domContentLoadedEventEnd", 4.0),
                ("domComplete", 5.0),
                ("loadEventStart", 5.0),
                ("loadEventEnd", 6.0),
            ] {
                define(t, name, Value::Number(value));
            }
        }
        define(p, "timing", timing);
        let navigation = it.object();
        if let Some(nv) = navigation.as_object() {
            define(nv, "type", Value::Number(0.0));
            define(nv, "redirectCount", Value::Number(0.0));
        }
        define(p, "navigation", navigation);
    }
    it.set_global("performance", perf);
    define(&g, "isSecureContext", Value::Bool(true));
    define(&g, "crossOriginIsolated", Value::Bool(false));
    define(&g, "getSelection", it.native("getSelection", 0, |it, _, _| Ok(make_selection(it))));
    define(&g, "getComputedStyle", {
        let st = state.clone();
        it.native("getComputedStyle", 1, move |it, _, args| {
            let node = node_of(it, &arg(&args, 0)).unwrap_or(st.borrow().doc.root);
            Ok(style_api::make_computed_style(it, &st, node))
        })
    });

    let screen = it.object();
    if let Some(s) = screen.as_object() {
        let st = state.borrow();
        define(s, "width", Value::Number(st.viewport.w as f64));
        define(s, "height", Value::Number(st.viewport.h as f64));
        define(s, "availWidth", Value::Number(st.viewport.w as f64));
        define(s, "availHeight", Value::Number(st.viewport.h as f64));
    }
    it.set_global("screen", screen);

    let st = state.clone();
    let mo = it.native("MutationObserver", 1, move |it, _, args| {
        let callback = arg(&args, 0);
        let obj = it.object();
        let Some(o) = obj.as_object().cloned() else { return Ok(obj) };
        let id = {
            let mut s = st.borrow_mut();
            let id = s.next_observer;
            s.next_observer += 1;
            id
        };
        define(&o, "__observer", Value::Number(id as f64));
        let st2 = st.clone();
        let cb = callback.clone();
        let owner = Some(obj.clone());
        define(&o, "observe", it.native("observe", 1, move |it, _, args| {
            let target = node_of(it, &arg(&args, 0))?;
            let opts = arg(&args, 1);
            let flag = |it: &mut Interp, name: &str| -> bool {
                it.get_member(opts.clone(), name).map(|v| v.truthy()).unwrap_or(false)
            };
            let subtree = flag(it, "subtree");
            let child_list = flag(it, "childList");
            let attributes = flag(it, "attributes");
            let character_data = flag(it, "characterData");
            st2.borrow_mut().observers.push(Observer {
                id,
                target,
                callback: cb.clone(),
                owner: owner.clone(),
                subtree,
                child_list,
                attributes,
                character_data,
            });
            Ok(Value::Undefined)
        }));
        let st3 = st.clone();
        define(&o, "disconnect", it.native("disconnect", 0, move |_, _, _| {
            st3.borrow_mut().observers.retain(|x| x.id != id);
            Ok(Value::Undefined)
        }));
        let st4 = st.clone();
        define(&o, "takeRecords", it.native("takeRecords", 0, move |it, _, _| {
            let mut s = st4.borrow_mut();
            let records = std::mem::take(&mut s.mutations);
            Ok(event_api::records_to_array(it, &st4, &records))
        }));
        Ok(obj)
    });
    it.set_global("MutationObserver", mo);

    let st = state.clone();
    let io = it.native("IntersectionObserver", 1, move |it, _, args| {
        let cb = arg(&args, 0);
        let obj = it.object();
        let Some(o) = obj.as_object().cloned() else { return Ok(obj) };
        define(&o, "root", arg(&args, 1));
        define(&o, "rootMargin", Value::string("0px"));
        define(&o, "thresholds", arg(&args, 2));
        let st_observe = st.clone();
        let cb_observe = cb.clone();
        let observer_for_cb = Value::Object(o.clone());
        define(&o, "observe", it.native("observe", 1, move |it, _, args| {
            let target = arg(&args, 0);
            let node = node_of(it, &target).ok();
            let cb = cb_observe.clone();
            let observer = observer_for_cb.clone();
            let st = st_observe.clone();
            let notify = it.native("io", 0, move |it, _, _| {
                let rect = node
                    .and_then(|n| st.borrow().boxes.get(&n).copied())
                    .unwrap_or(veyra_common::Rect::new(0.0, 0.0, 0.0, 0.0));
                let entry = it.object();
                if let Some(eo) = entry.as_object() {
                    define(eo, "target", target.clone());
                    define(eo, "isIntersecting", Value::Bool(true));
                    define(eo, "intersectionRatio", Value::Number(1.0));
                    define(eo, "time", Value::Number(it.now_ms()));
                    let make_rect = |it: &mut Interp| {
                        let r = it.object();
                        if let Some(ro) = r.as_object() {
                            for (k, v) in [
                                ("x", rect.x), ("y", rect.y), ("top", rect.y), ("left", rect.x),
                                ("right", rect.right()), ("bottom", rect.bottom()),
                                ("width", rect.w), ("height", rect.h),
                            ] {
                                define(ro, k, Value::Number(v as f64));
                            }
                        }
                        r
                    };
                    let bounding = make_rect(it);
                    define(eo, "boundingClientRect", bounding);
                    define(eo, "intersectionRect", make_rect(it));
                    define(eo, "rootBounds", Value::Null);
                }
                let entries = it.array_from(vec![entry]);
                it.call_value(&cb, observer.clone(), vec![entries, observer.clone()])?;
                Ok(Value::Undefined)
            });
            it.add_timer(notify, 0.0, false, vec![]);
            Ok(Value::Undefined)
        }));
        define(&o, "unobserve", it.native("unobserve", 1, |_, _, _| Ok(Value::Undefined)));
        define(&o, "disconnect", it.native("disconnect", 0, |_, _, _| Ok(Value::Undefined)));
        define(&o, "takeRecords", it.native("takeRecords", 0, |it, _, _| Ok(it.array_from(vec![]))));
        Ok(obj)
    });
    it.set_global("IntersectionObserver", io);

    let custom_elements = it.object();
    if let Some(ce) = custom_elements.as_object().cloned() {
        let st = state.clone();
        define(&ce, "define", it.native("define", 2, move |it, _, args| {
            let name = it.to_string(&arg(&args, 0))?.to_ascii_lowercase();
            let ctor = arg(&args, 1);
            if !ctor.is_callable() {
                return Err(JsError::type_error("custom element constructor is not a constructor"));
            }
            if st.borrow().custom_elements.contains_key(&name) {
                let msg = format!("custom element <{name}> is already defined");
                return Err(JsError::throw(it.make_error("NotSupportedError", &msg)));
            }
            st.borrow_mut().custom_elements.insert(name.clone(), ctor.clone());
            // Upgrade elements already in the tree, including shadow trees.
            let nodes = collect_elements(&st.borrow().doc);
            for n in nodes {
                if st.borrow().doc.tag_name(n) == name {
                    upgrade_element(it, &st, n);
                }
            }
            Ok(Value::Undefined)
        }));
        let st = state.clone();
        define(&ce, "get", it.native("get", 1, move |it, _, args| {
            let name = it.to_string(&arg(&args, 0))?.to_ascii_lowercase();
            Ok(st.borrow().custom_elements.get(&name).cloned().unwrap_or(Value::Undefined))
        }));
        let st = state.clone();
        define(&ce, "whenDefined", it.native("whenDefined", 1, move |it, _, args| {
            let name = it.to_string(&arg(&args, 0))?.to_ascii_lowercase();
            let (p, d) = it.new_promise();
            match st.borrow().custom_elements.get(&name).cloned() {
                Some(ctor) => it.resolve_promise(&d, ctor),
                None => it.resolve_promise(&d, Value::Undefined),
            }
            Ok(p)
        }));
        let st = state.clone();
        define(&ce, "upgrade", it.native("upgrade", 1, move |it, _, args| {
            if let Ok(node) = node_of(it, &arg(&args, 0)) {
                upgrade_element(it, &st, node);
            }
            Ok(Value::Undefined)
        }));
        let st = state.clone();
        define(&ce, "getName", it.native("getName", 1, move |it, _, args| {
            let ctor = arg(&args, 0);
            let name = st.borrow().custom_elements.iter().find(|(_, v)| strict(v, &ctor)).map(|(k, _)| k.clone());
            Ok(name.map(Value::string).unwrap_or(Value::Null))
        }));
    }
    it.set_global("customElements", custom_elements);

    let st = state.clone();
    it.set_global("ResizeObserver", it.native("ResizeObserver", 1, move |it, _, args| {
        let cb = arg(&args, 0);
        let obj = it.object();
        let Some(o) = obj.as_object().cloned() else { return Ok(obj) };
        let st_observe = st.clone();
        define(&o, "observe", it.native("observe", 1, move |it, _, args| {
            let target = arg(&args, 0);
            let node = node_of(it, &target).ok();
            let cb = cb.clone();
            let st = st_observe.clone();
            let notify = it.native("ro", 0, move |it, _, _| {
                let rect = node
                    .and_then(|n| st.borrow().boxes.get(&n).copied())
                    .unwrap_or(veyra_common::Rect::new(0.0, 0.0, 0.0, 0.0));
                let entry = it.object();
                if let Some(eo) = entry.as_object() {
                    define(eo, "target", target.clone());
                    let content = it.object();
                    if let Some(co) = content.as_object() {
                        for (k, v) in [("width", rect.w), ("height", rect.h)] {
                            define(co, k, Value::Number(v as f64));
                        }
                    }
                    define(eo, "contentRect", content);
                }
                let entries = it.array_from(vec![entry]);
                it.call_value(&cb, Value::Undefined, vec![entries, Value::Undefined])?;
                Ok(Value::Undefined)
            });
            it.add_timer(notify, 0.0, false, vec![]);
            Ok(Value::Undefined)
        }));
        define(&o, "unobserve", it.native("unobserve", 1, |_, _, _| Ok(Value::Undefined)));
        define(&o, "disconnect", it.native("disconnect", 0, |_, _, _| Ok(Value::Undefined)));
        Ok(obj)
    }));

    let _ = document;
}

fn make_storage(it: &mut Interp, state: &Rc<RefCell<DomState>>, session: bool) -> Value {
    let obj = it.object();
    let Some(o) = obj.as_object().cloned() else { return obj };
    let st = state.clone();
    define(&o, "getItem", it.native("getItem", 1, move |it, _, args| {
        let key = it.to_string(&arg(&args, 0))?;
        let s = st.borrow();
        let map = if session { &s.session_storage } else { &s.storage };
        Ok(map.get(&key).map(|v| Value::string(v.clone())).unwrap_or(Value::Null))
    }));
    let st = state.clone();
    define(&o, "setItem", it.native("setItem", 2, move |it, _, args| {
        let key = it.to_string(&arg(&args, 0))?;
        let value = it.to_string(&arg(&args, 1))?;
        let mut s = st.borrow_mut();
        let map = if session { &mut s.session_storage } else { &mut s.storage };
        map.insert(key, value);
        Ok(Value::Undefined)
    }));
    let st = state.clone();
    define(&o, "removeItem", it.native("removeItem", 1, move |it, _, args| {
        let key = it.to_string(&arg(&args, 0))?;
        let mut s = st.borrow_mut();
        let map = if session { &mut s.session_storage } else { &mut s.storage };
        map.remove(&key);
        Ok(Value::Undefined)
    }));
    let st = state.clone();
    define(&o, "clear", it.native("clear", 0, move |_, _, _| {
        let mut s = st.borrow_mut();
        let map = if session { &mut s.session_storage } else { &mut s.storage };
        map.clear();
        Ok(Value::Undefined)
    }));
    let st = state.clone();
    define(&o, "key", it.native("key", 1, move |it, _, args| {
        let i = arg(&args, 0).to_index();
        let s = st.borrow();
        let map = if session { &s.session_storage } else { &s.storage };
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        Ok(keys.get(i).map(|k| Value::string((*k).clone())).unwrap_or(Value::Null))
    }));
    let st = state.clone();
    let len = it.native("length", 0, move |_, _, _| {
        let s = st.borrow();
        let map = if session { &s.session_storage } else { &s.storage };
        Ok(Value::Number(map.len() as f64))
    });
    define_accessor(&o, "length", Some(len), None, false);
    obj
}

// ----------------------------------------------------------------------
fn blob_part_size(it: &mut Interp, value: &Value) -> usize {
    match value {
        Value::String(s) => s.len(),
        Value::Object(_) => {
            if let Ok(size) = it.get_member(value.clone(), "size") {
                if matches!(size, Value::Number(_)) {
                    return size.to_number().max(0.0) as usize;
                }
            }
            if let Ok(len) = it.get_member(value.clone(), "byteLength") {
                if matches!(len, Value::Number(_)) {
                    return len.to_number().max(0.0) as usize;
                }
            }
            if let Ok(items) = it.iterate(value) {
                return items.iter().map(|item| blob_part_size(it, item)).sum();
            }
            0
        }
        _ => 0,
    }
}

/// Minimal Blob surface. Pages use it for feature detection and body sizing;
/// full byte storage is intentionally not retained by this engine.
fn install_blob(it: &mut Interp) {
    let ctor = it.native("Blob", 2, |it, this, args| {
        let parts = args.first().cloned().unwrap_or(Value::Undefined);
        let mut size = 0usize;
        if !parts.is_nullish() {
            match it.iterate(&parts) {
                Ok(items) => size = items.iter().map(|item| blob_part_size(it, item)).sum(),
                Err(_) => size = blob_part_size(it, &parts),
            }
        }
        let ty = args
            .get(1)
            .filter(|v| v.is_object())
            .and_then(|opts| it.get_member(opts.clone(), "type").ok())
            .map(|v| it.to_string(&v).unwrap_or_default().to_ascii_lowercase())
            .unwrap_or_default();
        if let Value::Object(obj) = &this {
            define(obj, "size", Value::Number(size as f64));
            define(obj, "type", Value::string(ty));
        }
        Ok(this)
    });
    let proto = it.object();
    if let (Some(func), Some(proto_obj)) = (ctor.as_object(), proto.as_object()) {
        define(func, "prototype", Value::Object(proto_obj.clone()));
        define(proto_obj, "constructor", ctor.clone());
        let slice = it.native("slice", 2, |it, this, args| {
            let size = it
                .get_member(this.clone(), "size")
                .map(|v| v.to_number())
                .unwrap_or(0.0);
            let start = args.first().map(|v| v.to_number()).unwrap_or(0.0).max(0.0);
            let end = args.get(1).filter(|v| !v.is_undefined()).map(|v| v.to_number());
            let end = end.unwrap_or(size).min(size).max(start);
            let out = new_object(None);
            define(&out, "size", Value::Number((end - start).max(0.0)));
            let ty = it
                .get_member(this, "type")
                .ok()
                .and_then(|v| it.to_string(&v).ok())
                .unwrap_or_default();
            define(&out, "type", Value::string(ty));
            Ok(Value::Object(out))
        });
        define(proto_obj, "slice", slice);
    }
    it.set_global("Blob", ctor);
}

// Wrappers
// ----------------------------------------------------------------------

/// Links the interface prototype objects into the inheritance chain browsers
/// expose: HTML*Element -> HTMLElement -> Element -> Node -> EventTarget.
fn install_dom_prototypes(it: &mut Interp, state: &Rc<RefCell<DomState>>) {
    if let Some(ctor) = it.get_global("Node") {
        if let Ok(proto) = it.get_member(ctor, "prototype") {
            if proto.as_object().is_some() {
                state.borrow_mut().interface_protos.insert("Node".into(), proto);
            }
        }
    }
    let links = [
        ("Node", "EventTarget"),
        ("Element", "Node"),
        ("HTMLElement", "Element"),
        ("SVGElement", "Element"),
        ("HTMLUnknownElement", "HTMLElement"),
        ("Document", "Node"),
        ("HTMLDocument", "Document"),
        ("DocumentFragment", "Node"),
        ("ShadowRoot", "DocumentFragment"),
        ("CharacterData", "Node"),
        ("Text", "CharacterData"),
        ("Comment", "CharacterData"),
        ("CDATASection", "Text"),
        ("ProcessingInstruction", "CharacterData"),
    ];
    for (child, parent) in links {
        let (child, parent) = {
            let s = state.borrow();
            (s.interface_protos.get(child).cloned(), s.interface_protos.get(parent).cloned())
        };
        if let (Some(Value::Object(c)), Some(Value::Object(p))) = (child, parent) {
            c.borrow_mut().proto = Some(p);
        }
    }
    // Every concrete HTML interface inherits from HTMLElement.
    let html_protos: Vec<Value> = {
        let s = state.borrow();
        s.interface_protos
            .iter()
            .filter(|(name, _)| name.starts_with("HTML") && name.as_str() != "HTMLDocument")
            .map(|(_, v)| v.clone())
            .collect()
    };
    let html_element = state.borrow().interface_protos.get("HTMLElement").cloned();
    if let Some(Value::Object(parent)) = html_element {
        for value in html_protos {
            if let Value::Object(child) = value {
                if !Rc::ptr_eq(&child, &parent) {
                    child.borrow_mut().proto = Some(parent.clone());
                }
            }
        }
    }
    let forward = |it: &mut Interp, name: &'static str, proto: Option<Value>| {
        if let Some(Value::Object(proto)) = proto {
            let f = it.native(name, 0, move |it, this, args| {
                let f = it.get_member(this.clone(), name)?;
                if f.is_callable() {
                    it.call_value(&f, this, args)
                } else {
                    Ok(Value::Undefined)
                }
            });
            if let Some(obj) = f.as_object() {
                define(obj, "__veyra_forward", Value::Bool(true));
            }
            define(&proto, name, f);
        }
    };
    let s = state.borrow();
    let element = s.interface_protos.get("Element").cloned();
    let node = s.interface_protos.get("Node").cloned();
    let document = s.interface_protos.get("Document").cloned();
    let fragment = s.interface_protos.get("DocumentFragment").cloned();
    let event_target = s.interface_protos.get("EventTarget").cloned();
    drop(s);
    forward(it, "attachShadow", element.clone());
    forward(it, "getRootNode", node.clone());
    for name in ["addEventListener", "removeEventListener", "dispatchEvent"] {
        forward(it, name, event_target.clone());
    }
    // Standard Node/Element methods belong on their interfaces, not only on
    // individual wrapper objects. Polyfills (notably ShadyDOM's noPatch mode)
    // capture and call these prototype methods directly.
    for name in [
        "appendChild", "insertBefore", "removeChild", "replaceChild",
        "contains", "hasChildNodes", "cloneNode", "isSameNode",
        "isEqualNode", "compareDocumentPosition", "normalize",
    ] {
        forward(it, name, node.clone());
    }
    for name in [
        "querySelector", "querySelectorAll", "closest", "matches",
        "getAttribute", "setAttribute", "removeAttribute", "hasAttribute",
        "toggleAttribute", "getAttributeNames", "getElementsByTagName",
        "getElementsByClassName", "insertAdjacentHTML",
        "insertAdjacentElement", "insertAdjacentText", "remove",
    ] {
        forward(it, name, element.clone());
    }
    for name in [
        "querySelector", "querySelectorAll", "getElementById",
        "getElementsByTagName", "getElementsByClassName",
        "getElementsByName", "createElement", "createElementNS",
        "createTextNode", "createComment", "createDocumentFragment",
        "importNode", "adoptNode", "createRange", "createTreeWalker",
    ] {
        forward(it, name, document.clone());
    }
    for name in [
        "querySelector", "querySelectorAll", "getElementById",
        "getElementsByTagName", "getElementsByClassName",
        "appendChild", "insertBefore", "removeChild", "replaceChild",
    ] {
        forward(it, name, fragment.clone());
    }
}

/// Picks the standard prototype for a node wrapper based on its DOM type.
fn wrapper_proto(state: &Rc<RefCell<DomState>>, node: NodeId) -> Option<Value> {
    let s = state.borrow();
    if s.doc.is_shadow_root(node) {
        return s.interface_protos.get("ShadowRoot").cloned();
    }
    let n = s.doc.node(node)?;
    let key = match &n.node_type {
        NodeType::Document => "HTMLDocument",
        NodeType::Doctype(_) => "Node",
        NodeType::Text(_) => "Text",
        NodeType::Comment(_) => "Comment",
        NodeType::Element(tag) => {
            let tag = tag.to_ascii_lowercase();
            if tag == "#shadow-root" {
                "ShadowRoot"
            } else if tag == "fragment" {
                "DocumentFragment"
            } else {
                return s
                    .element_protos
                    .get(&tag)
                    .cloned()
                    .or_else(|| s.interface_protos.get("HTMLElement").cloned());
            }
        }
    };
    s.interface_protos.get(key).cloned()
}

/// Re-applies the prototype after the interface table was populated. The
/// document wrapper exists before install() has built the constructors.
fn refresh_wrapper_protos(state: &Rc<RefCell<DomState>>) {
    let entries: Vec<(NodeId, Value)> = state.borrow().wrappers.iter().map(|(k, v)| (*k, v.clone())).collect();
    for (node, value) in entries {
        let proto = wrapper_proto(state, node).and_then(|p| p.as_object().cloned());
        if let (Some(obj), Some(proto)) = (value.as_object().cloned(), proto) {
            obj.borrow_mut().proto = Some(proto);
        }
    }
}

/// Names ShadyDOM replaces on the DOM interface prototypes. A node wrapper's
/// own methods/accessors otherwise shadow the polyfill's logical-tree layers.
const SHADY_METHODS: &[&str] = &[
    "appendChild", "insertBefore", "removeChild", "replaceChild",
    "cloneNode", "contains", "getRootNode", "attachShadow",
    "addEventListener", "removeEventListener", "dispatchEvent",
];

const SHADY_ACCESSORS: &[&str] = &[
    "parentNode", "parentElement", "firstChild", "lastChild",
    "nextSibling", "previousSibling", "childNodes", "children",
    "firstElementChild", "lastElementChild", "childElementCount",
    "nextElementSibling", "previousElementSibling", "shadowRoot",
    "textContent", "innerHTML", "className",
];

fn shady_active(it: &mut Interp) -> bool {
    match it.get_global("ShadyDOM") {
        Some(v) => it.get_member(v, "inUse").map(|x| x.truthy()).unwrap_or(false),
        None => false,
    }
}

fn nearest_proto_prop(obj: &ObjRef, name: &str) -> Option<Prop> {
    let mut proto = obj.borrow().proto.clone();
    while let Some(p) = proto {
        if let Some(prop) = own_prop(&p, name) {
            return Some(prop);
        }
        proto = p.borrow().proto.clone();
    }
    None
}

fn same_function(a: &Option<Value>, b: &Option<Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Object(x)), Some(Value::Object(y))) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

fn copy_shady_props(it: &mut Interp, obj: &ObjRef) {
    if !shady_active(it) || obj.borrow().props.contains_key("__veyra_shady_synced") {
        return;
    }
    for name in SHADY_METHODS {
        let Some(proto_prop) = nearest_proto_prop(obj, name) else { continue };
        let Some(value) = proto_prop.value.as_object() else { continue };
        if own_prop(&value, "__veyra_forward").is_some() {
            continue;
        }
        define_with(obj, name, proto_prop.value, true, false, true);
    }
    for name in SHADY_ACCESSORS {
        // ShadyDOM's logical getters fall back to `__shady_native_*` when a
        // node has no logical entry yet. Native browser interfaces provide
        // those aliases from their own accessor descriptors; our wrappers
        // carry the accessors themselves, so preserve them here.
        if let Some(own) = own_prop(obj, name) {
            let alias = format!("__shady_native_{name}");
            if own_prop(obj, &alias).is_none() && own.get.is_some() {
                define_accessor(obj, &alias, own.get.clone(), own.set.clone(), false);
            }
        }
        let Some(proto_prop) = nearest_proto_prop(obj, name) else { continue };
        if proto_prop.get.is_none() {
            continue;
        }
        let own = own_prop(obj, name);
        if let Some(own) = own {
            if same_function(&own.get, &proto_prop.get) && same_function(&own.set, &proto_prop.set) {
                continue;
            }
        }
        define_accessor(obj, name, proto_prop.get, proto_prop.set, false);
    }
    if own_prop(obj, "__shady_native_attachShadow").is_some() {
        let bridge = it.native("__shady_attachShadow", 1, move |it, this, args| {
            let native = it.get_member(this.clone(), "__shady_native_attachShadow")?;
            let root = it.call_value(&native, this.clone(), args.clone())?;
            let init = args.first().cloned().unwrap_or(Value::Undefined);
            let fragment = it.get_member(init, "shadyUpgradeFragment").ok();
            if let (Some(root_obj), Some(Value::Object(_))) = (root.as_object(), fragment.as_ref()) {
                let append = it.get_member(root.clone(), "__shady_native_appendChild")?;
                if append.is_callable() {
                    let _ = it.call_value(&append, root.clone(), vec![fragment.unwrap()]);
                }
                let _ = root_obj;
            }
            if let Some(host) = this.as_object() {
                if let Some(shady) = host
                    .borrow()
                    .props
                    .get("__shady")
                    .and_then(|p| p.value.as_object().cloned())
                {
                    define(&shady, "Xb", root.clone());
                    define(&shady, "root", root.clone());
                }
            }
            Ok(root)
        });
        define_with(obj, "__shady_attachShadow", bridge, true, false, true);
    }
    define_with(obj, "__veyra_shady_synced", Value::Bool(true), true, false, true);
}

/// Re-applies ShadyDOM's patched tree layer to wrappers that already exist
/// when the polyfill finishes installing itself.
pub fn sync_shady_wrappers(it: &mut Interp, state: &Rc<RefCell<DomState>>) {
    if !shady_active(it) {
        return;
    }
    let wrappers: Vec<ObjRef> = state
        .borrow()
        .wrappers
        .values()
        .filter_map(|v| v.as_object().cloned())
        .collect();
    for obj in wrappers {
        copy_shady_props(it, &obj);
    }
}

pub fn wrap(it: &mut Interp, state: &Rc<RefCell<DomState>>, node: NodeId) -> Value {
    if let Some(v) = state.borrow().wrappers.get(&node) {
        return v.clone();
    }
    let obj = it.object();
    let obj_ref = obj.as_object().unwrap().clone();
    if let Some(proto) = wrapper_proto(state, node) {
        if let Some(proto_ref) = proto.as_object().cloned() {
            obj_ref.borrow_mut().proto = Some(proto_ref);
        }
    }
    define(&obj_ref, "__node", Value::Number(node.0 as f64));
    dom_api::define_node_api(it, &obj_ref, state, node);
    let is_doc = state
        .borrow()
        .doc
        .node(node)
        .map(|n| matches!(n.node_type, NodeType::Document))
        .unwrap_or(false);
    if is_doc {
        dom_api::define_document_api(it, &obj_ref, state, node);
        let st = state.clone();
        getter(it, &obj_ref, "currentScript", &st, node, |it, st, _| {
            let current = st.borrow().current_script;
            match current {
                Some(id) => Ok(wrap(it, st, id)),
                None => Ok(Value::Null),
            }
        });
    }
    let is_style = state.borrow().doc.tag_name(node) == "style";
    if is_style {
        let st = state.clone();
        let sheet = it.object();
        if let Some(sh) = sheet.as_object().cloned() {
            let st2 = st.clone();
            define(&sh, "insertRule", it.native("insertRule", 1, move |it, _, args| {
                let rule = it.to_string(&arg(&args, 0))?;
                st2.borrow_mut().extra_css.push(rule);
                st2.borrow_mut().mark_dirty();
                Ok(Value::Number(0.0))
            }));
            define(&sh, "deleteRule", it.native("deleteRule", 1, |_, _, _| Ok(Value::Undefined)));
            define(&sh, "cssRules", it.array_from(vec![]));
            define(&sh, "disabled", Value::Bool(false));
        }
        define(&obj_ref, "sheet", sheet);
    }
    copy_shady_props(it, &obj_ref);
    state.borrow_mut().wrappers.insert(node, obj.clone());
    obj
}

/// Builds a JavaScript array of node wrappers without overlapping borrows.
pub(crate) fn wrappers(
    it: &mut Interp,
    state: &Rc<RefCell<DomState>>,
    nodes: impl IntoIterator<Item = NodeId>,
) -> Value {
    let mut values = Vec::new();
    for n in nodes {
        values.push(wrap(it, state, n));
    }
    it.array_from(values)
}

/// Every element in the document, including shadow trees.
pub(crate) fn collect_elements(doc: &Document) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = doc.children(doc.root).iter().rev().copied().collect();
    while let Some(n) = stack.pop() {
        if doc.node(n).map(|node| node.is_element()).unwrap_or(false) {
            out.push(n);
        }
        if let Some(root) = doc.shadow_root(n) {
            stack.extend(doc.children(root).iter().rev().copied());
        }
        stack.extend(doc.children(n).iter().rev().copied());
    }
    out
}

fn call_lifecycle(it: &mut Interp, target: &Value, name: &str, args: Vec<Value>) -> Result<(), JsError> {
    let f = it.get_member(target.clone(), name)?;
    if f.is_callable() {
        let _ = it.call_value(&f, target.clone(), args);
        return Ok(());
    }
    // PolySi's sink elements register a mount wrapper without the standard
    // custom-element lifecycle methods on its prototype. Their controller
    // (`polymerController`) owns connectedCallback/disconnectedCallback and the
    // patched DOM path is what normally invokes it. Native DOM operations in
    // this engine bypass that patch, so forward the lifecycle to the
    // controller when the wrapper has no method of its own.
    let pc = it.get_member(target.clone(), "polymerController")?;
    if pc.is_object() {
        let f = it.get_member(pc.clone(), name)?;
        if f.is_callable() {
            let _ = it.call_value(&f, pc, args);
        }
    }
    Ok(())
}

/// Runs a custom element constructor against the existing DOM wrapper. This
/// keeps the wrapper identity stable while giving `this` the class prototype
/// and running field initializers and the constructor body.
pub(crate) fn upgrade_element(it: &mut Interp, state: &Rc<RefCell<DomState>>, node: NodeId) {
    if state.borrow().upgraded_elements.contains(&node) {
        return;
    }
    let (tag, ctor) = {
        let s = state.borrow();
        let tag = s.doc.tag_name(node).to_string();
        let ctor = s.custom_elements.get(&tag).cloned();
        (tag, ctor)
    };
    let Some(ctor) = ctor else { return };
    let _ = tag;
    let proto = it
        .get_member(ctor.clone(), "prototype")
        .ok()
        .and_then(|v| v.as_object().cloned());
    let wrapper = wrap(it, state, node);
    // Polymer's property finalisation reads generated accessors from the
    // prototype while it is still being built. Those accessors read
    // `this.__data`, so the prototype needs its data store before any
    // constructor runs; JavaScript does not guard that read on its own.
    if let Some(proto_obj) = proto.as_ref() {
        if it
            .get_member(Value::Object(proto_obj.clone()), "__data")
            .map(|v| v.is_undefined())
            .unwrap_or(true)
        {
            define(proto_obj, "__data", Value::Object(new_object(None)));
        }
    }
    if let (Some(obj), Some(proto)) = (wrapper.as_object().cloned(), proto) {
        obj.borrow_mut().proto = Some(proto);
    }
    // Observed attributes are read once so attribute changes can be filtered.
    let observed = it
        .get_member(ctor.clone(), "observedAttributes")
        .ok()
        .and_then(|v| it.iterate(&v).ok())
        .map(|vals| {
            vals.into_iter()
                .filter_map(|v| it.to_string(&v).ok())
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    {
        let mut s = state.borrow_mut();
        s.upgraded_elements.insert(node);
        if !observed.is_empty() {
            s.observed_attributes.insert(node, observed.clone());
        }
    }
    // Calling a class constructor as a function is exactly what an upgrade
    // needs: `this` is already the wrapped element, so `super()` and all DOM
    // methods operate on the real node.
    it.allow_class_call = true;
    let construct_result = it.call_value(&ctor, wrapper.clone(), Vec::new());
    it.allow_class_call = false;
    if let Err(err) = construct_result {
        // A constructor exception during upgrade is reported to the page's
        // error channel (as the HTML standard requires) rather than swallowed.
        it.host.log(
            "error",
            &format!(
                "custom element <{}> upgrade failed: {}: {} [line {}] [stack: {}]",
                tag,
                err.kind,
                err.message,
                err.line,
                err.stack.join("; ")
            ),
        );
    }
    let attrs: Vec<(String, String)> = {
        let s = state.borrow();
        s.doc.attrs(node).iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };
    for (name, value) in attrs {
        if observed.iter().any(|a| a == &name) {
            let _ = call_lifecycle(
                it,
                &wrapper,
                "attributeChangedCallback",
                vec![Value::string(name), Value::Null, Value::string(value)],
            );
        }
    }
    if state.borrow().doc.is_in_document(node) {
        let _ = call_lifecycle(it, &wrapper, "connectedCallback", Vec::new());
        state.borrow_mut().connected_elements.insert(node);
    }
}

fn subtree_elements(doc: &Document, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if doc.is_element(n) {
            out.push(n);
        }
        if let Some(shadow) = doc.shadow_root(n) {
            stack.extend(doc.children(shadow).iter().rev().copied());
        }
        stack.extend(doc.children(n).iter().rev().copied());
    }
    out
}

pub(crate) fn connect_subtree(it: &mut Interp, state: &Rc<RefCell<DomState>>, root: NodeId) {
    if !state.borrow().doc.is_in_document(root) {
        return;
    }
    let nodes = subtree_elements(&state.borrow().doc, root);
    for node in nodes {
        let tag = state.borrow().doc.tag_name(node).to_string();
        if state.borrow().custom_elements.contains_key(&tag) {
            upgrade_element(it, state, node);
        }
        if state.borrow().upgraded_elements.contains(&node)
            && !state.borrow().connected_elements.contains(&node)
        {
            state.borrow_mut().connected_elements.insert(node);
            let wrapper = wrap(it, state, node);
            let _ = call_lifecycle(it, &wrapper, "connectedCallback", Vec::new());
        }
    }
}

pub(crate) fn disconnect_subtree(it: &mut Interp, state: &Rc<RefCell<DomState>>, root: NodeId) {
    let mut nodes = subtree_elements(&state.borrow().doc, root);
    nodes.reverse();
    for node in nodes {
        let was_connected = state.borrow_mut().connected_elements.remove(&node);
        if was_connected {
            let wrapper = wrap(it, state, node);
            let _ = call_lifecycle(it, &wrapper, "disconnectedCallback", Vec::new());
        }
    }
}

/// Runs queued attribute callbacks and connected/disconnected callbacks. Called
/// from the page pump after script turns so DOM changes settle in order.
pub(crate) fn run_custom_element_lifecycle(it: &mut Interp, state: &Rc<RefCell<DomState>>) {
    if state.borrow().custom_elements.is_empty() {
        state.borrow_mut().attribute_changes.clear();
        return;
    }
    let changes = std::mem::take(&mut state.borrow_mut().attribute_changes);
    for (node, name, old, new) in changes {
        if !state.borrow().upgraded_elements.contains(&node) {
            continue;
        }
        let wrapper = wrap(it, state, node);
        let _ = call_lifecycle(
            it,
            &wrapper,
            "attributeChangedCallback",
            vec![
                Value::string(name),
                old.map(Value::string).unwrap_or(Value::Null),
                new.map(Value::string).unwrap_or(Value::Null),
            ],
        );
    }
    let elements = collect_elements(&state.borrow().doc);
    for node in elements {
        let connected = state.borrow().doc.is_in_document(node);
        let active = state.borrow().connected_elements.contains(&node);
        if connected && !active {
            let wrapper = wrap(it, state, node);
            let own = it.get_member(wrapper.clone(), "connectedCallback").unwrap_or(Value::Undefined);
            let pc = it.get_member(wrapper.clone(), "polymerController").unwrap_or(Value::Undefined);
            let controller_lifecycle = pc.is_object()
                && it
                    .get_member(pc.clone(), "connectedCallback")
                    .map(|f| f.is_callable())
                    .unwrap_or(false);
            let upgraded = state.borrow().upgraded_elements.contains(&node);
            if upgraded || (!own.is_callable() && controller_lifecycle) {
                state.borrow_mut().connected_elements.insert(node);
                let _ = call_lifecycle(it, &wrapper, "connectedCallback", Vec::new());
            }
        } else if !connected && active {
            state.borrow_mut().connected_elements.remove(&node);
            let wrapper = wrap(it, state, node);
            let _ = call_lifecycle(it, &wrapper, "disconnectedCallback", Vec::new());
        }
    }
}

pub(crate) fn node_of(it: &mut Interp, v: &Value) -> Result<NodeId, JsError> {
    let n = it.get_member(v.clone(), "__node")?;
    match n {
        Value::Number(n) => Ok(NodeId(n as u32)),
        _ => Err(JsError::type_error("expected a DOM node")),
    }
}

pub(crate) fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

pub(crate) fn strict(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) | (Value::Undefined, Value::Undefined) => true,
        _ => false,
    }
}

pub(crate) fn getter<F>(it: &mut Interp, obj: &ObjRef, name: &str, state: &Rc<RefCell<DomState>>, node: NodeId, f: F)
where
    F: Fn(&mut Interp, &Rc<RefCell<DomState>>, NodeId) -> Result<Value, JsError> + 'static,
{
    let st = state.clone();
    let fun = it.native(name, 0, move |it, _, _| f(it, &st, node));
    define_accessor(obj, name, Some(fun), None, false);
}

pub(crate) fn getset<F, S>(
    it: &mut Interp,
    obj: &ObjRef,
    name: &str,
    state: &Rc<RefCell<DomState>>,
    node: NodeId,
    get: F,
    set: S,
) where
    F: Fn(&mut Interp, &Rc<RefCell<DomState>>, NodeId) -> Result<Value, JsError> + 'static,
    S: Fn(&mut Interp, &Rc<RefCell<DomState>>, NodeId, Value) -> Result<(), JsError> + 'static,
{
    let st = state.clone();
    let g = it.native(name, 0, move |it, _, _| get(it, &st, node));
    let st = state.clone();
    let s = it.native(name, 1, move |it, _, args| set(it, &st, node, arg(&args, 0)).map(|_| Value::Undefined));
    define_accessor(obj, name, Some(g), Some(s), false);
}

pub(crate) fn method<F>(
    it: &mut Interp,
    obj: &ObjRef,
    name: &str,
    len: usize,
    state: &Rc<RefCell<DomState>>,
    node: NodeId,
    f: F,
) where
    F: Fn(&mut Interp, &Rc<RefCell<DomState>>, NodeId, Vec<Value>) -> Result<Value, JsError> + 'static,
{
    let st = state.clone();
    let fun = it.native(name, len, move |it, _, args| f(it, &st, node, args));
    define_with(obj, &format!("__shady_native_{name}"), fun.clone(), true, false, true);
    define(obj, name, fun);
}

/// A minimal synchronous `XMLHttpRequest` used by older page scripts and
/// analytics beacons. Supports open/send/setRequestHeader and the common
/// callback properties; it completes before `send` returns.
fn make_xhr(it: &mut Interp) -> Value {
    let obj = it.object();
    let o = obj.as_object().unwrap().clone();
    define(&o, "readyState", Value::Number(0.0));
    define(&o, "status", Value::Number(0.0));
    define(&o, "statusText", Value::string(""));
    define(&o, "responseText", Value::string(""));
    define(&o, "response", Value::string(""));
    define(&o, "onload", Value::Null);
    define(&o, "onerror", Value::Null);
    define(&o, "onreadystatechange", Value::Null);
    define(&o, "open", it.native("open", 2, |it, this, args| {
        let method = it.to_string(&arg(&args, 0))?.to_uppercase();
        let url = it.to_string(&arg(&args, 1))?;
        it.set_member(this.clone(), "__method", Value::string(method))?;
        it.set_member(this.clone(), "__url", Value::string(url))?;
        it.set_member(this.clone(), "readyState", Value::Number(1.0))?;
        Ok(Value::Undefined)
    }));
    define(&o, "setRequestHeader", it.native("setRequestHeader", 2, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let value = it.to_string(&arg(&args, 1))?;
        let current = it.get_member(this.clone(), "__headers").unwrap_or(Value::Undefined);
        let array = if current.is_object() {
            current
        } else {
            let a = it.array_from(vec![]);
            it.set_member(this.clone(), "__headers", a.clone())?;
            a
        };
        if let Some(a) = array.as_object() {
            if let ObjKind::Array(values) = &mut a.borrow_mut().kind {
                values.push(Value::string(format!("{name}\u{1}{value}")));
            }
        }
        Ok(Value::Undefined)
    }));
    define(&o, "getAllResponseHeaders", it.native("getAllResponseHeaders", 0, |it, this, _| {
        Ok(it
            .get_member(this, "__responseHeaders")
            .unwrap_or(Value::Undefined))
    }));
    define(&o, "send", it.native("send", 1, |it, this, args| {
        let method = it
            .get_member(this.clone(), "__method")
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "GET".into());
        let url = it
            .get_member(this.clone(), "__url")
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let mut headers = Vec::new();
        if let Some(arr) = it
            .get_member(this.clone(), "__headers")
            .ok()
            .and_then(|v| v.as_object().cloned())
        {
            let entries = array_values(&Value::Object(arr)).unwrap_or_default();
            for entry in entries {
                if let Some(s) = entry.as_str() {
                    if let Some((k, v)) = s.split_once('\u{1}') {
                        headers.push((k.to_string(), v.to_string()));
                    }
                }
            }
        }
        let body = match args.first().filter(|v| !v.is_nullish()) {
            Some(v) => Some(it.to_string(v)?),
            None => None,
        };
        match it.host.fetch(&method, &url, &headers, body.as_deref()) {
            Ok(resp) => {
                let text = resp.body;
                let mut header_text = String::new();
                for (k, v) in &resp.headers {
                    header_text.push_str(k);
                    header_text.push_str(": ");
                    header_text.push_str(v);
                    header_text.push_str("\r\n");
                }
                it.set_member(this.clone(), "status", Value::Number(resp.status as f64))?;
                it.set_member(this.clone(), "responseText", Value::string(text.clone()))?;
                it.set_member(this.clone(), "response", Value::string(text))?;
                it.set_member(this.clone(), "__responseHeaders", Value::string(header_text))?;
                it.set_member(this.clone(), "readyState", Value::Number(4.0))?;
                for name in ["onreadystatechange", "onload"] {
                    let cb = it.get_member(this.clone(), name)?;
                    if cb.is_callable() {
                        it.call_value(&cb, this.clone(), vec![])?;
                    }
                }
            }
            Err(msg) => {
                it.set_member(this.clone(), "readyState", Value::Number(4.0))?;
                let cb = it.get_member(this.clone(), "onerror")?;
                if cb.is_callable() {
                    it.call_value(&cb, this.clone(), vec![Value::string(msg)])?;
                }
            }
        }
        Ok(Value::Undefined)
    }));
    define(&o, "abort", it.native("abort", 0, |_, _, _| Ok(Value::Undefined)));
    obj
}

pub(crate) fn set_attribute(state: &Rc<RefCell<DomState>>, node: NodeId, name: &str, value: &str) {
    let mut s = state.borrow_mut();
    if (name == "src" || name == "srcset") && s.doc.tag_name(node) == "img" {
        s.images_dirty = true;
    }
    let old = s.doc.attr(node, name).map(|v| v.to_string());
    s.doc.set_attr(node, name, value);
    if s
        .observed_attributes
        .get(&node)
        .map(|attrs| attrs.iter().any(|a| a == name))
        .unwrap_or(false)
    {
        s.attribute_changes.push((
            node,
            name.to_string(),
            old,
            Some(value.to_string()),
        ));
    }
    s.note_mutation(crate::state::Mutation {
        kind: "attributes",
        target: node,
        added: vec![],
        removed: vec![],
        attribute: Some(name.to_string()),
    });
}

pub(crate) fn make_selection(it: &mut Interp) -> Value {
    let obj = it.object();
    if let Some(o) = obj.as_object() {
        for name in ["removeAllRanges", "addRange", "empty", "collapse", "setPosition", "selectAllChildren"] {
            define(o, name, it.native(name, 1, |_, _, _| Ok(Value::Undefined)));
        }
        define(o, "toString", it.native("toString", 0, |_, _, _| Ok(Value::string(""))));
        define(o, "rangeCount", Value::Number(0.0));
        define(o, "isCollapsed", Value::Bool(true));
    }
    obj
}

fn make_location(it: &mut Interp, state: &Rc<RefCell<DomState>>) -> Value {
    let obj = it.object();
    let url = state.borrow().base.clone();
    if let Some(o) = obj.as_object() {
        let host = if let Some(port) = url.port {
            format!("{}:{port}", url.host)
        } else {
            url.host.clone()
        };
        let origin = url
            .origin()
            .map(|o| format!("{}://{}", o.scheme, o.host))
            .unwrap_or_default();
        let st_href = state.clone();
        let get_href = it.native("href", 0, move |_, _, _| {
            Ok(Value::string(st_href.borrow().base.to_string()))
        });
        let st_href_set = state.clone();
        let set_href = it.native("href", 1, move |it, _, args| {
            let s = it.to_string(&arg(&args, 0))?;
            let u = st_href_set.borrow().resolve(&s);
            st_href_set.borrow_mut().navigation = Some(u);
            Ok(Value::Undefined)
        });
        define_accessor(o, "href", Some(get_href), Some(set_href), true);
        for (name, value) in [
            ("protocol", format!("{}:", url.scheme)),
            ("host", host),
            ("hostname", url.host.clone()),
            ("pathname", url.path.clone()),
            ("search", url.query.as_ref().map(|q| format!("?{q}")).unwrap_or_default()),
            ("hash", url.fragment.as_ref().map(|f| format!("#{f}")).unwrap_or_default()),
            ("origin", origin),
        ] {
            define(o, name, Value::string(value));
        }
        let st = state.clone();
        define(o, "assign", it.native("assign", 1, move |it, _, args| {
            let s = it.to_string(&arg(&args, 0))?;
            let u = st.borrow().resolve(&s);
            st.borrow_mut().navigation = Some(u);
            Ok(Value::Undefined)
        }));
        let st = state.clone();
        define(o, "replace", it.native("replace", 1, move |it, _, args| {
            let s = it.to_string(&arg(&args, 0))?;
            let u = st.borrow().resolve(&s);
            st.borrow_mut().navigation = Some(u);
            Ok(Value::Undefined)
        }));
        let st = state.clone();
        define(o, "reload", it.native("reload", 0, move |_, _, _| {
            st.borrow_mut().reload = true;
            Ok(Value::Undefined)
        }));
        let st = state.clone();
        define(o, "toString", it.native("toString", 0, move |_, _, _| {
            Ok(Value::string(st.borrow().base.to_string()))
        }));
    }
    obj
}
