//! Built-in objects and functions of the Veyra JavaScript engine.

use std::cell::RefCell;
use std::rc::Rc;

use crate::runtime::Interp;
use crate::value::*;


/// A protocol-level iterator: `next()` plus `Symbol.iterator` returning itself.
fn make_iterator(it: &mut Interp, values: Vec<Value>) -> Value {
    let obj = it.object();
    let Some(o) = obj.as_object().cloned() else { return obj };
    let cursor = Rc::new(RefCell::new(0usize));
    let items = Rc::new(values);
    let next = it.native("next", 0, {
        let cursor = cursor.clone();
        let items = items.clone();
        move |it, _, _| {
            let i = *cursor.borrow();
            let result = it.object();
            if let Some(ro) = result.as_object() {
                if i < items.len() {
                    define(ro, "value", items[i].clone());
                    define(ro, "done", Value::Bool(false));
                } else {
                    define(ro, "value", Value::Undefined);
                    define(ro, "done", Value::Bool(true));
                }
            }
            *cursor.borrow_mut() = i + 1;
            Ok(result)
        }
    });
    define(&o, "next", next);
    let self_iter = it.native("@@iterator", 0, |_, this, _| Ok(this));
    define(&o, "@@symbol:Symbol.iterator", self_iter);
    obj
}

pub fn install(it: &mut Interp) {
    install_eval(it);
    let object_proto = it.intrinsics.object_proto.clone();
    let function_proto = it.intrinsics.function_proto.clone();
    let array_proto = it.intrinsics.array_proto.clone();
    let string_proto = it.intrinsics.string_proto.clone();
    let number_proto = it.intrinsics.number_proto.clone();
    let boolean_proto = it.intrinsics.boolean_proto.clone();
    let error_proto = it.intrinsics.error_proto.clone();
    let type_error_proto = it.intrinsics.type_error_proto.clone();
    let range_error_proto = it.intrinsics.range_error_proto.clone();
    let syntax_error_proto = it.intrinsics.syntax_error_proto.clone();
    let reference_error_proto = it.intrinsics.reference_error_proto.clone();
    let date_proto = it.intrinsics.date_proto.clone();
    let regexp_proto = it.intrinsics.regexp_proto.clone();
    let map_proto = it.intrinsics.map_proto.clone();
    let set_proto = it.intrinsics.set_proto.clone();
    let promise_proto = it.intrinsics.promise_proto.clone();

    macro_rules! method {
        ($obj:expr, $name:expr, $len:expr, $f:expr) => {{
            let f = it.native($name, $len, $f);
            define(&$obj, $name, f);
        }};
    }
    macro_rules! getter {
        ($obj:expr, $name:expr, $f:expr) => {{
            let f = it.native($name, 0, $f);
            define_accessor(&$obj, $name, Some(f), None, false);
        }};
    }

    // ------------------------------------------------------------------
    // Object
    // ------------------------------------------------------------------
    method!(object_proto, "toString", 0, |_, this, _| {
        Ok(Value::string(format!("[object {}]", tag(&this))))
    });
    method!(object_proto, "toLocaleString", 0, |it, this, _| Ok(Value::string(it.to_string(&this)?)));
    method!(object_proto, "valueOf", 0, |_, this, _| Ok(this));
    let proto_get = it.native("get __proto__", 0, |_, this, _| {
        Ok(this
            .as_object()
            .and_then(|o| o.borrow().proto.clone())
            .map(Value::Object)
            .unwrap_or(Value::Null))
    });
    let proto_set = it.native("set __proto__", 1, |_, this, args| {
        if let Some(o) = this.as_object() {
            match arg(&args, 0) {
                Value::Null => o.borrow_mut().proto = None,
                Value::Object(proto) => o.borrow_mut().proto = Some(proto),
                _ => {}
            }
        }
        Ok(Value::Undefined)
    });
    define_accessor(&object_proto, "__proto__", Some(proto_get), Some(proto_set), false);
    method!(object_proto, "hasOwnProperty", 1, |it, this, args| {
        let key = it.to_property_key(&arg(&args, 0))?;
        let obj = to_object(it, &this)?;
        Ok(Value::Bool(own_prop(&obj, &key).is_some()))
    });
    method!(object_proto, "propertyIsEnumerable", 1, |it, this, args| {
        let key = it.to_property_key(&arg(&args, 0))?;
        let obj = to_object(it, &this)?;
        let enumerable = own_prop(&obj, &key).map(|p| p.enumerable).unwrap_or(false);
        Ok(Value::Bool(enumerable))
    });
    method!(object_proto, "isPrototypeOf", 1, |_, this, args| {
        let proto = match this.as_object() {
            Some(p) => p.clone(),
            None => return Ok(Value::Bool(false)),
        };
        let mut cur = arg(&args, 0).as_object().and_then(|o| o.borrow().proto.clone());
        let mut depth = 0;
        while let Some(o) = cur {
            if Rc::ptr_eq(&o, &proto) {
                return Ok(Value::Bool(true));
            }
            cur = o.borrow().proto.clone();
            depth += 1;
            if depth > 200 {
                break;
            }
        }
        Ok(Value::Bool(false))
    });

    let object_ctor = it.native("Object", 1, |it, _, args| {
        match args.first() {
            None | Some(Value::Null) | Some(Value::Undefined) => Ok(it.object()),
            Some(Value::Object(_)) => Ok(args[0].clone()),
            Some(v) => Ok(Value::Object(to_object(it, v)?)),
        }
    });
    define(&object_proto, "constructor", object_ctor.clone());
    if let Some(o) = object_ctor.as_object() {
        define(o, "prototype", Value::Object(object_proto.clone()));
    }

    let object_ctor_ref = object_ctor.clone();
    macro_rules! object_static {
        ($name:expr, $len:expr, $f:expr) => {{
            let f = it.native($name, $len, $f);
            if let Some(o) = object_ctor_ref.as_object() {
                define(o, $name, f);
            }
        }};
    }
    object_static!("keys", 1, |it, _, args| {
        let obj = arg(&args, 0);
        let keys = obj
            .as_object()
            .map(|o| own_keys(o).into_iter().map(Value::string).collect())
            .unwrap_or_default();
        Ok(it.array_from(keys))
    });
    object_static!("values", 1, |it, _, args| {
        let obj = arg(&args, 0);
        let values = obj
            .as_object()
            .map(|o| own_key_values(o).into_iter().map(|(_, v)| v).collect())
            .unwrap_or_default();
        Ok(it.array_from(values))
    });
    object_static!("entries", 1, |it, _, args| {
        let obj = arg(&args, 0);
        let mut out = Vec::new();
        if let Some(o) = obj.as_object() {
            for (k, v) in own_key_values(o) {
                out.push(it.array_from(vec![Value::string(k), v]));
            }
        }
        Ok(it.array_from(out))
    });
    object_static!("assign", 2, |it, _, args| {
        let target = arg(&args, 0);
        let tobj = to_object(it, &target)?;
        for src in args.iter().skip(1) {
            if let Some(so) = src.as_object() {
                for (k, v) in own_key_values(so) {
                    define(&tobj, &k, v);
                }
            }
        }
        Ok(Value::Object(tobj))
    });
    object_static!("create", 2, |it, _, args| {
        let proto = match args.first() {
            Some(Value::Null) => None,
            Some(Value::Object(o)) => Some(o.clone()),
            // Bundler helper functions sometimes pass an undefined superclass
            // when transpiling mixins; treat it like a null prototype rather
            // than aborting the whole bundle.
            _ => None,
        };
        let obj = new_object(proto);
        if let Some(props) = args.get(1).and_then(|v| v.as_object()) {
            let names = own_keys(props);
            for name in names {
                let desc = it.get_member(Value::Object(props.clone()), &name)?;
                apply_descriptor(it, &obj, &name, &desc)?;
            }
        }
        Ok(Value::Object(obj))
    });
    object_static!("getPrototypeOf", 1, |it, _, args| {
        let obj = to_object(it, &arg(&args, 0))?;
        let p = obj.borrow().proto.clone();
        Ok(p.map(Value::Object).unwrap_or(Value::Null))
    });
    object_static!("setPrototypeOf", 2, |it, _, args| {
        let obj = to_object(it, &arg(&args, 0))?;
        let proto = match arg(&args, 1) {
            Value::Null => None,
            Value::Object(o) => Some(o),
            _ => return Err(JsError::type_error("prototype must be an object or null")),
        };
        obj.borrow_mut().proto = proto;
        Ok(Value::Object(obj))
    });
    object_static!("defineProperty", 3, |it, _, args| {
        let obj = to_object(it, &arg(&args, 0))?;
        let key = it.to_property_key(&arg(&args, 1))?;
        apply_descriptor(it, &obj, &key, &arg(&args, 2))?;
        Ok(Value::Object(obj))
    });
    object_static!("defineProperties", 2, |it, _, args| {
        let obj = to_object(it, &arg(&args, 0))?;
        if let Some(props) = arg(&args, 1).as_object() {
            let keys = own_keys(&props);
            for key in keys {
                let desc = it.get_member(Value::Object(props.clone()), &key)?;
                apply_descriptor(it, &obj, &key, &desc)?;
            }
        }
        Ok(Value::Object(obj))
    });
    object_static!("getOwnPropertyDescriptor", 2, |it, _, args| {
        let obj = to_object(it, &arg(&args, 0))?;
        let key = it.to_property_key(&arg(&args, 1))?;
        let prop = match own_prop(&obj, &key) {
            Some(p) => p,
            None => return Ok(Value::Undefined),
        };
        let d = it.object();
        if let Some(o) = d.as_object() {
            if let Some(get) = prop.get.clone() {
                define(o, "get", get);
            }
            if let Some(set) = prop.set.clone() {
                define(o, "set", set);
            }
            if prop.get.is_none() {
                define(o, "value", prop.value.clone());
                define(o, "writable", Value::Bool(prop.writable));
            }
            define(o, "enumerable", Value::Bool(prop.enumerable));
            define(o, "configurable", Value::Bool(prop.configurable));
        }
        Ok(d)
    });
    object_static!("getOwnPropertyNames", 1, |it, _, args| {
        let obj = arg(&args, 0);
        let mut names = Vec::new();
        if let Some(o) = obj.as_object() {
            for k in own_keys(o) {
                names.push(Value::string(k));
            }
            for (k, _) in o.borrow().props.iter() {
                if !names.iter().any(|n| n.as_str() == Some(k.as_str())) {
                    names.push(Value::string(k.clone()));
                }
            }
        }
        Ok(it.array_from(names))
    });
    object_static!("getOwnPropertySymbols", 1, |it, _, _| {
        Ok(it.array_from(Vec::new()))
    });
    object_static!("getOwnPropertyDescriptors", 1, |it, _, args| {
        let out = it.object();
        if let Some(o) = out.as_object() {
            if let Some(obj) = arg(&args, 0).as_object() {
                for k in own_keys(obj) {
                    if let Some(p) = own_prop(obj, &k) {
                        let desc = it.object();
                        if let Some(d) = desc.as_object() {
                            define(d, "value", p.value);
                            define(d, "writable", Value::Bool(p.writable));
                            define(d, "enumerable", Value::Bool(p.enumerable));
                            define(d, "configurable", Value::Bool(p.configurable));
                        }
                        define(o, &k, desc);
                    }
                }
            }
        }
        Ok(out)
    });
    object_static!("freeze", 1, |_, _, args| {
        if let Some(o) = arg(&args, 0).as_object() {
            let mut obj = o.borrow_mut();
            obj.extensible = false;
            let keys: Vec<String> = obj.props.keys().cloned().collect();
            for k in keys {
                if let Some(p) = obj.props.get_mut(&k) {
                    p.writable = false;
                    p.configurable = false;
                }
            }
            if let ObjKind::Array(_) = obj.kind {
                // Array elements live outside `props`; blocking growth below is
                // enough for the common freeze-and-mutate pattern.
            }
        }
        Ok(arg(&args, 0))
    });
    object_static!("seal", 1, |_, _, args| {
        if let Some(o) = arg(&args, 0).as_object() {
            let mut obj = o.borrow_mut();
            obj.extensible = false;
            let keys: Vec<String> = obj.props.keys().cloned().collect();
            for k in keys {
                if let Some(p) = obj.props.get_mut(&k) {
                    p.configurable = false;
                }
            }
        }
        Ok(arg(&args, 0))
    });
    object_static!("preventExtensions", 1, |_, _, args| {
        if let Some(o) = arg(&args, 0).as_object() {
            o.borrow_mut().extensible = false;
        }
        Ok(arg(&args, 0))
    });
    object_static!("isFrozen", 1, |_, _, args| {
        let frozen = arg(&args, 0)
            .as_object()
            .map(|o| !o.borrow().extensible)
            .unwrap_or(true);
        Ok(Value::Bool(frozen))
    });
    object_static!("fromEntries", 1, |it, _, args| {
        let entries = it.iterate(&arg(&args, 0))?;
        let obj = it.object();
        if let Some(o) = obj.as_object() {
            for e in entries {
                let pair = it.iterate(&e)?;
                let k = it.to_property_key(&pair.first().cloned().unwrap_or(Value::Undefined))?;
                define(o, &k, pair.get(1).cloned().unwrap_or(Value::Undefined));
            }
        }
        Ok(obj)
    });
    object_static!("hasOwn", 2, |it, _, args| {
        let obj = to_object(it, &arg(&args, 0))?;
        let key = it.to_property_key(&arg(&args, 1))?;
        Ok(Value::Bool(own_prop(&obj, &key).is_some()))
    });
    object_static!("is", 2, |_, _, args| {
        let (a, b) = (arg(&args, 0), arg(&args, 1));
        let same = match (&a, &b) {
            (Value::Number(x), Value::Number(y)) => {
                (x.is_nan() && y.is_nan()) || (x == y && x.is_sign_negative() == y.is_sign_negative())
            }
            _ => strict_eq(&a, &b),
        };
        Ok(Value::Bool(same))
    });

    // ------------------------------------------------------------------
    // Function
    // ------------------------------------------------------------------
    method!(function_proto, "call", 1, |it, this, args| {
        if !this.is_callable() {
            return Err(JsError::type_error("Function.prototype.call called on non-function"));
        }
        let this_arg = arg(&args, 0);
        let rest = args.iter().skip(1).cloned().collect();
        it.call_value(&this, this_arg, rest)
    });
    method!(function_proto, "apply", 2, |it, this, args| {
        if !this.is_callable() {
            return Err(JsError::type_error("Function.prototype.apply called on non-function"));
        }
        let this_arg = arg(&args, 0);
        let list = match args.get(1) {
            Some(Value::Null) | Some(Value::Undefined) | None => Vec::new(),
            Some(v) => array_like_values(v).unwrap_or_default(),
        };
        it.call_value(&this, this_arg, list)
    });
    method!(function_proto, "bind", 1, |it, this, args| {
        if !this.is_callable() {
            return Err(JsError::type_error("Function.prototype.bind called on non-function"));
        }
        let target = this.clone();
        let bound_this = arg(&args, 0);
        let bound_args: Vec<Value> = args.iter().skip(1).cloned().collect();
        Ok(it.native("bound", 0, move |it, _this, call_args| {
            let mut all = bound_args.clone();
            all.extend(call_args);
            it.call_value(&target, bound_this.clone(), all)
        }))
    });
    method!(function_proto, "toString", 0, |it, this, _| {
        // `Function.prototype.toString` must reveal whether a value is an ES
        // class: feature detection in Polymer and other libraries runs
        // `/\bclass\b/.test(class{}.toString())` to decide between
        // `Reflect.construct` and the legacy `.apply` path.
        if let Some(obj) = this.as_object() {
            if let ObjKind::Function(Func::User(u)) = &obj.borrow().kind {
                let name = u.node.name.clone().unwrap_or_default();
                if u.is_class_ctor {
                    return Ok(Value::string(if name.is_empty() {
                        "class {}".to_string()
                    } else {
                        format!("class {name} {{}}")
                    }));
                }
                return Ok(Value::string(if name.is_empty() {
                    "function () { [native code] }".to_string()
                } else {
                    format!("function {name}() {{ [native code] }}")
                }));
            }
            if let ObjKind::Function(Func::Native(_)) = &obj.borrow().kind {
                return Ok(Value::string("function () { [native code] }"));
            }
        }
        Ok(Value::string(it.to_string(&this)?))
    });

    let function_ctor = it.native("Function", 1, |it, _, args| {
        let params: Vec<String> = args
            .iter()
            .take(args.len().saturating_sub(1))
            .map(|v| it.to_string(v).unwrap_or_default())
            .collect();
        let body = args.last().map(|v| it.to_string(v).unwrap_or_default()).unwrap_or_default();
        let src = format!("function anonymous({}) {{\n{body}\n}}", params.join(","));
        it.eval_expression(&src)
    });
    define(&function_proto, "constructor", function_ctor.clone());
    if let Some(o) = function_ctor.as_object() {
        define(o, "prototype", Value::Object(function_proto.clone()));
    }

    // ------------------------------------------------------------------
    // Array
    // ------------------------------------------------------------------
    method!(array_proto, "push", 1, |_, this, args| {
        // Array.prototype.push is generic: any object is a valid receiver,
        // with a missing/non-index `length` treated as zero (V8 behaviour).
        let mut values = array_like_values(&this)
            .or_else(|| this.is_object().then(Vec::new))
            .ok_or_else(|| JsError::type_error("Array.prototype.push called on non-array"))?;
        values.extend(args);
        store_array_values(&this, &values);
        Ok(Value::Number(values.len() as f64))
    });
    method!(array_proto, "pop", 0, |_, this, _| {
        let mut values = array_like_values(&this).unwrap_or_default();
        let v = values.pop().unwrap_or(Value::Undefined);
        store_array_values(&this, &values);
        Ok(v)
    });
    method!(array_proto, "shift", 0, |_, this, _| {
        let mut values = array_like_values(&this).unwrap_or_default();
        let v = if values.is_empty() { Value::Undefined } else { values.remove(0) };
        store_array_values(&this, &values);
        Ok(v)
    });
    method!(array_proto, "unshift", 1, |_, this, args| {
        let mut values = array_like_values(&this).unwrap_or_default();
        let mut new_values: Vec<Value> = args;
        new_values.extend(values);
        values = new_values;
        store_array_values(&this, &values);
        Ok(Value::Number(values.len() as f64))
    });
    method!(array_proto, "join", 1, |it, this, args| {
        let values = array_like_values(&this).unwrap_or_default();
        let sep = args.first().map(|v| it.to_string(v).unwrap_or_default()).unwrap_or_else(|| ",".into());
        let sep = if args.is_empty() { ",".to_string() } else { sep };
        let mut parts = Vec::new();
        for v in values {
            if v.is_nullish() {
                parts.push(String::new());
            } else {
                parts.push(it.to_string(&v)?);
            }
        }
        Ok(Value::string(parts.join(&sep)))
    });
    method!(array_proto, "toString", 0, |it, this, _| {
        let values = array_like_values(&this).unwrap_or_default();
        let mut parts = Vec::new();
        for v in values {
            if v.is_nullish() {
                parts.push(String::new());
            } else {
                parts.push(it.to_string(&v)?);
            }
        }
        Ok(Value::string(parts.join(",")))
    });
    method!(array_proto, "slice", 2, |it, this, args| {
        let values = array_like_values(&this).unwrap_or_default();
        let len = values.len() as i64;
        let start = normalize_index(arg(&args, 0).to_number(), len);
        let end = if args.len() > 1 {
            normalize_index(arg(&args, 1).to_number(), len)
        } else {
            len
        };
        let out: Vec<Value> = if start < end {
            values[start as usize..end as usize].to_vec()
        } else {
            Vec::new()
        };
        Ok(it.array_from(out))
    });
    method!(array_proto, "splice", 2, |it, this, args| {
        let mut values = array_like_values(&this).unwrap_or_default();
        let len = values.len() as i64;
        let start = normalize_index(arg(&args, 0).to_number(), len);
        let delete = if args.len() < 2 {
            (len - start).max(0)
        } else {
            (arg(&args, 1).to_number().max(0.0) as i64).min(len - start)
        };
        let removed: Vec<Value> = values[start as usize..(start + delete) as usize].to_vec();
        let items: Vec<Value> = args.iter().skip(2).cloned().collect();
        values.splice(start as usize..(start + delete) as usize, items);
        store_array_values(&this, &values);
        Ok(it.array_from(removed))
    });
    method!(array_proto, "concat", 1, |it, this, args| {
        let mut out = array_like_values(&this).unwrap_or_default();
        let mut holes = array_holes(&this).unwrap_or_else(|| vec![false; out.len()]);
        for a in args {
            if is_array(&a) {
                let values = array_values(&a).unwrap_or_default();
                let mut more = array_holes(&a).unwrap_or_else(|| vec![false; values.len()]);
                more.resize(values.len(), false);
                out.extend(values);
                holes.extend(more);
            } else {
                out.push(a);
                holes.push(false);
            }
        }
        Ok(it.array_with_holes(out, holes))
    });
    method!(array_proto, "indexOf", 1, |_, this, args| {
        let values = array_like_values(&this).unwrap_or_default();
        let needle = arg(&args, 0);
        let start = arg(&args, 1).to_number().max(0.0) as usize;
        for (i, v) in values.iter().enumerate().skip(start) {
            if strict_eq(v, &needle) {
                return Ok(Value::Number(i as f64));
            }
        }
        Ok(Value::Number(-1.0))
    });
    method!(array_proto, "lastIndexOf", 1, |_, this, args| {
        let values = array_like_values(&this).unwrap_or_default();
        let needle = arg(&args, 0);
        for (i, v) in values.iter().enumerate().rev() {
            if strict_eq(v, &needle) {
                return Ok(Value::Number(i as f64));
            }
        }
        Ok(Value::Number(-1.0))
    });
    method!(array_proto, "includes", 1, |_, this, args| {
        let values = array_like_values(&this).unwrap_or_default();
        let needle = arg(&args, 0);
        Ok(Value::Bool(values.iter().any(|v| v.same_value_zero(&needle))))
    });
    method!(array_proto, "forEach", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let this_arg = arg(&args, 1);
        let values = array_like_values(&this).unwrap_or_default();
        let holes = array_holes(&this).unwrap_or_else(|| vec![false; values.len()]);
        let arr = this.clone();
        for (i, v) in values.into_iter().enumerate() {
            if holes.get(i).copied().unwrap_or(false) {
                continue;
            }
            it.call_value(&cb, this_arg.clone(), vec![v, Value::Number(i as f64), arr.clone()])?;
        }
        Ok(Value::Undefined)
    });
    method!(array_proto, "map", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let this_arg = arg(&args, 1);
        let values = array_like_values(&this).unwrap_or_default();
        let holes = array_holes(&this).unwrap_or_else(|| vec![false; values.len()]);
        let arr = this.clone();
        let mut out = Vec::new();
        for (i, v) in values.into_iter().enumerate() {
            if holes.get(i).copied().unwrap_or(false) {
                continue;
            }
            out.push(it.call_value(&cb, this_arg.clone(), vec![v, Value::Number(i as f64), arr.clone()])?);
        }
        Ok(it.array_from(out))
    });
    method!(array_proto, "set", 2, |_, this, args| {
        let src = arg(&args, 0);
        let offset = arg(&args, 1).to_index();
        let src_vals = array_like_values(&src).unwrap_or_default();
        let mut vals = array_like_values(&this).unwrap_or_default();
        for (i, v) in src_vals.into_iter().enumerate() {
            let idx = offset + i;
            if idx >= vals.len() {
                vals.resize(idx + 1, Value::Undefined);
            }
            vals[idx] = v;
        }
        store_array_values(&this, &vals);
        Ok(Value::Undefined)
    });
    method!(array_proto, "subarray", 2, |_, this, args| {
        let vals = array_like_values(&this).unwrap_or_default();
        let start = arg(&args, 0).to_index().min(vals.len());
        let end = if args.len() > 1 && !arg(&args, 1).is_undefined() {
            arg(&args, 1).to_index().min(vals.len())
        } else {
            vals.len()
        };
        let slice = if start <= end { vals[start..end].to_vec() } else { Vec::new() };
        Ok(Value::Object(new_array(slice, this.as_object().and_then(|o| o.borrow().proto.clone()).unwrap_or_else(|| new_object(None)))))
    });
    method!(array_proto, "filter", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let this_arg = arg(&args, 1);
        let values = array_like_values(&this).unwrap_or_default();
        let holes = array_holes(&this).unwrap_or_else(|| vec![false; values.len()]);
        let arr = this.clone();
        let mut out = Vec::new();
        for (i, v) in values.into_iter().enumerate() {
            if holes.get(i).copied().unwrap_or(false) {
                continue;
            }
            if it
                .call_value(&cb, this_arg.clone(), vec![v.clone(), Value::Number(i as f64), arr.clone()])?
                .truthy()
            {
                out.push(v);
            }
        }
        Ok(it.array_from(out))
    });
    method!(array_proto, "reduce", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let values = array_like_values(&this).unwrap_or_default();
        let arr = this.clone();
        let mut iter = values.into_iter().enumerate();
        let mut acc = if args.len() > 1 {
            arg(&args, 1)
        } else {
            match iter.next() {
                Some((_, v)) => v,
                None => return Err(JsError::type_error("reduce of empty array with no initial value")),
            }
        };
        for (i, v) in iter {
            acc = it.call_value(&cb, Value::Undefined, vec![acc, v, Value::Number(i as f64), arr.clone()])?;
        }
        Ok(acc)
    });
    method!(array_proto, "reduceRight", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let values = array_like_values(&this).unwrap_or_default();
        let arr = this.clone();
        let len = values.len();
        let mut idx = len;
        let mut acc = if args.len() > 1 {
            arg(&args, 1)
        } else {
            if len == 0 {
                return Err(JsError::type_error("reduce of empty array with no initial value"));
            }
            idx = len - 1;
            values[idx].clone()
        };
        while idx > 0 {
            idx -= 1;
            acc = it.call_value(&cb, Value::Undefined, vec![acc, values[idx].clone(), Value::Number(idx as f64), arr.clone()])?;
        }
        Ok(acc)
    });
    method!(array_proto, "some", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let values = array_like_values(&this).unwrap_or_default();
        let arr = this.clone();
        for (i, v) in values.into_iter().enumerate() {
            if it.call_value(&cb, Value::Undefined, vec![v, Value::Number(i as f64), arr.clone()])?.truthy() {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(false))
    });
    method!(array_proto, "every", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let values = array_like_values(&this).unwrap_or_default();
        let arr = this.clone();
        for (i, v) in values.into_iter().enumerate() {
            if !it.call_value(&cb, Value::Undefined, vec![v, Value::Number(i as f64), arr.clone()])?.truthy() {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(true))
    });
    method!(array_proto, "find", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let values = array_like_values(&this).unwrap_or_default();
        let arr = this.clone();
        for (i, v) in values.into_iter().enumerate() {
            if it.call_value(&cb, Value::Undefined, vec![v.clone(), Value::Number(i as f64), arr.clone()])?.truthy() {
                return Ok(v);
            }
        }
        Ok(Value::Undefined)
    });
    method!(array_proto, "findIndex", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let values = array_like_values(&this).unwrap_or_default();
        let arr = this.clone();
        for (i, v) in values.into_iter().enumerate() {
            if it.call_value(&cb, Value::Undefined, vec![v, Value::Number(i as f64), arr.clone()])?.truthy() {
                return Ok(Value::Number(i as f64));
            }
        }
        Ok(Value::Number(-1.0))
    });
    method!(array_proto, "findLast", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let values = array_like_values(&this).unwrap_or_default();
        let arr = this.clone();
        for (i, v) in values.into_iter().enumerate().rev() {
            if it.call_value(&cb, Value::Undefined, vec![v.clone(), Value::Number(i as f64), arr.clone()])?.truthy() {
                return Ok(v);
            }
        }
        Ok(Value::Undefined)
    });
    method!(array_proto, "findLastIndex", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let values = array_like_values(&this).unwrap_or_default();
        let arr = this.clone();
        for (i, v) in values.into_iter().enumerate().rev() {
            if it.call_value(&cb, Value::Undefined, vec![v, Value::Number(i as f64), arr.clone()])?.truthy() {
                return Ok(Value::Number(i as f64));
            }
        }
        Ok(Value::Number(-1.0))
    });
    method!(array_proto, "toReversed", 0, |it, this, _| {
        let mut values = array_like_values(&this).unwrap_or_default();
        values.reverse();
        Ok(it.array_from(values))
    });
    method!(array_proto, "toSorted", 1, |it, this, args| {
        let mut values = array_like_values(&this).unwrap_or_default();
        let cmp = args.first().cloned().filter(|v| v.is_callable());
        if let Some(cmp) = cmp {
            let mut result = Ok(());
            values.sort_by(|a, b| {
                if result.is_err() {
                    return std::cmp::Ordering::Equal;
                }
                match it.call_value(&cmp, Value::Undefined, vec![a.clone(), b.clone()]) {
                    Ok(v) => {
                        let n = v.to_number();
                        if n < 0.0 { std::cmp::Ordering::Less } else if n > 0.0 { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal }
                    }
                    Err(e) => { result = Err(e); std::cmp::Ordering::Equal }
                }
            });
            result?;
        } else {
            values.sort_by(|a, b| it.to_string(a).unwrap_or_default().cmp(&it.to_string(b).unwrap_or_default()));
        }
        Ok(it.array_from(values))
    });
    method!(array_proto, "reverse", 0, |_, this, _| {
        let mut values = array_like_values(&this).unwrap_or_default();
        values.reverse();
        store_array_values(&this, &values);
        Ok(this)
    });
    method!(array_proto, "sort", 1, |it, this, args| {
        let mut values = array_like_values(&this).unwrap_or_default();
        let cmp = args.first().cloned().filter(|v| v.is_callable());
        if let Some(cmp) = cmp {
            let mut result = Ok(());
            values.sort_by(|a, b| {
                if result.is_err() {
                    return std::cmp::Ordering::Equal;
                }
                match it.call_value(&cmp, Value::Undefined, vec![a.clone(), b.clone()]) {
                    Ok(v) => {
                        let n = v.to_number();
                        if n < 0.0 {
                            std::cmp::Ordering::Less
                        } else if n > 0.0 {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    }
                    Err(e) => {
                        result = Err(e);
                        std::cmp::Ordering::Equal
                    }
                }
            });
            result?;
        } else {
            values.sort_by(|a, b| it.to_string(a).unwrap_or_default().cmp(&it.to_string(b).unwrap_or_default()));
        }
        store_array_values(&this, &values);
        Ok(this)
    });
    method!(array_proto, "fill", 1, |_, this, args| {
        let mut values = array_like_values(&this).unwrap_or_default();
        let len = values.len() as i64;
        let start = normalize_index(arg(&args, 1).to_number(), len);
        let end = if args.len() > 2 { normalize_index(arg(&args, 2).to_number(), len) } else { len };
        for i in start..end {
            if i < len {
                values[i as usize] = arg(&args, 0);
            }
        }
        store_array_values(&this, &values);
        Ok(this)
    });
    method!(array_proto, "flat", 0, |it, this, args| {
        let depth = if args.is_empty() { 1 } else { arg(&args, 0).to_number() as i32 };
        let values = array_like_values(&this).unwrap_or_default();
        let mut out = Vec::new();
        flatten(it, values, depth, &mut out);
        Ok(it.array_from(out))
    });
    method!(array_proto, "flatMap", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let values = array_like_values(&this).unwrap_or_default();
        let arr = this.clone();
        let mut out = Vec::new();
        for (i, v) in values.into_iter().enumerate() {
            let r = it.call_value(&cb, Value::Undefined, vec![v, Value::Number(i as f64), arr.clone()])?;
            if is_array(&r) {
                out.extend(array_values(&r).unwrap_or_default());
            } else {
                out.push(r);
            }
        }
        Ok(it.array_from(out))
    });
    method!(array_proto, "at", 1, |_, this, args| {
        let values = array_like_values(&this).unwrap_or_default();
        let len = values.len() as i64;
        let i = normalize_index(arg(&args, 0).to_number(), len);
        Ok(values.get(i as usize).cloned().unwrap_or(Value::Undefined))
    });
    method!(array_proto, "keys", 0, |it, this, _| {
        let len = array_like_values(&this).map(|v| v.len()).unwrap_or(0);
        Ok(it.array_from((0..len).map(|i| Value::Number(i as f64)).collect()))
    });
    method!(array_proto, "values", 0, |it, this, _| {
        Ok(it.array_from(array_like_values(&this).unwrap_or_default()))
    });
    method!(array_proto, "entries", 0, |it, this, _| {
        let values = array_like_values(&this).unwrap_or_default();
        Ok(it.array_from(
            values
                .into_iter()
                .enumerate()
                .map(|(i, v)| it.array_from(vec![Value::Number(i as f64), v]))
                .collect(),
        ))
    });

    let array_ctor = it.native("Array", 1, |it, _, args| {
        if args.len() == 1 {
            if let Value::Number(n) = args[0] {
                if n >= 0.0 && n.fract() == 0.0 && n < MAX_ARRAY_LIKE as f64 {
                    let len = n as usize;
                    return Ok(it.array_with_holes(vec![Value::Undefined; len], vec![true; len]));
                }
                return Err(JsError::range_error("invalid array length"));
            }
        }
        Ok(it.array_from(args))
    });
    define(&array_proto, "constructor", array_ctor.clone());
    if let Some(o) = array_ctor.as_object() {
        define(o, "prototype", Value::Object(array_proto.clone()));
    }
    let array_ctor_ref = array_ctor.clone();
    macro_rules! array_static {
        ($name:expr, $len:expr, $f:expr) => {{
            let f = it.native($name, $len, $f);
            if let Some(o) = array_ctor_ref.as_object() {
                define(o, $name, f);
            }
        }};
    }
    array_static!("isArray", 1, |_, _, args| Ok(Value::Bool(is_array(&arg(&args, 0)))));
    array_static!("of", 0, |it, _, args| Ok(it.array_from(args)));
    array_static!("from", 2, |it, _, args| {
        let v = arg(&args, 0);
        let values = it.iterate(&v).or_else(|_| {
            array_like_values(&v).ok_or_else(|| JsError::type_error("value is not iterable"))
        })?;
        if let Some(map) = args.get(1).filter(|m| m.is_callable()) {
            let map = map.clone();
            let mut out = Vec::new();
            for (i, value) in values.into_iter().enumerate() {
                out.push(it.call_value(&map, Value::Undefined, vec![value, Value::Number(i as f64)])?);
            }
            Ok(it.array_from(out))
        } else {
            Ok(it.array_from(values))
        }
    });

    // ------------------------------------------------------------------
    // String
    // ------------------------------------------------------------------
    method!(string_proto, "toString", 0, |_, this, _| Ok(Value::string(require_string(&this)?)));
    method!(string_proto, "valueOf", 0, |_, this, _| Ok(Value::string(require_string(&this)?)));
    method!(string_proto, "charAt", 1, |_, this, args| {
        let s = require_string(&this)?;
        let i = arg(&args, 0).to_number().max(0.0) as usize;
        Ok(Value::string(s.chars().nth(i).map(|c| c.to_string()).unwrap_or_default()))
    });
    method!(string_proto, "charCodeAt", 1, |_, this, args| {
        let s = require_string(&this)?;
        let i = arg(&args, 0).to_number().max(0.0) as usize;
        Ok(Value::Number(s.chars().nth(i).map(|c| c as u32 as f64).unwrap_or(f64::NAN)))
    });
    method!(string_proto, "codePointAt", 1, |_, this, args| {
        let s = require_string(&this)?;
        let i = arg(&args, 0).to_number().max(0.0) as usize;
        Ok(Value::Number(s.chars().nth(i).map(|c| c as u32 as f64).unwrap_or(f64::NAN)))
    });
    method!(string_proto, "at", 1, |_, this, args| {
        let s = require_string(&this)?;
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len() as i64;
        let i = normalize_index(arg(&args, 0).to_number(), len);
        Ok(Value::string(chars.get(i as usize).map(|c| c.to_string()).unwrap_or_default()))
    });
    method!(string_proto, "includes", 1, |_, this, args| {
        let s = require_string(&this)?;
        let needle = require_string(&arg(&args, 0))?;
        Ok(Value::Bool(s.contains(&needle)))
    });
    method!(string_proto, "indexOf", 1, |_, this, args| {
        let s = require_string(&this)?;
        let needle = require_string(&arg(&args, 0))?;
        let start = arg(&args, 1).to_number().max(0.0) as usize;
        let start = char_to_byte(&s, start);
        Ok(Value::Number(match s[start..].find(&needle) {
            Some(i) => byte_to_char(&s, start + i) as f64,
            None => -1.0,
        }))
    });
    method!(string_proto, "lastIndexOf", 1, |_, this, args| {
        let s = require_string(&this)?;
        let needle = require_string(&arg(&args, 0))?;
        let chars: Vec<char> = s.chars().collect();
        let pat: Vec<char> = needle.chars().collect();
        let len = chars.len() as i64;
        // `fromIndex` defaults to +Infinity and NaN counts as +Infinity.
        // Negative values are relative to the end of the string.
        let from = match arg(&args, 1) {
            Value::Undefined => len,
            v => {
                let n = v.to_number();
                if n.is_nan() {
                    len
                } else if n < 0.0 {
                    (len as f64 + n.trunc()).max(0.0) as i64
                } else {
                    n.trunc().min(len as f64) as i64
                }
            }
        };
        if pat.is_empty() {
            return Ok(Value::Number(from as f64));
        }
        if pat.len() > chars.len() {
            return Ok(Value::Number(-1.0));
        }
        let hi = (from as usize).min(chars.len() - pat.len());
        let found = (0..=hi).rev().find(|&k| chars[k..k + pat.len()] == pat[..]);
        Ok(Value::Number(match found {
            Some(i) => i as f64,
            None => -1.0,
        }))
    });
    method!(string_proto, "startsWith", 1, |_, this, args| {
        let s = require_string(&this)?;
        let needle = require_string(&arg(&args, 0))?;
        Ok(Value::Bool(s.starts_with(&needle)))
    });
    method!(string_proto, "endsWith", 1, |_, this, args| {
        let s = require_string(&this)?;
        let needle = require_string(&arg(&args, 0))?;
        Ok(Value::Bool(s.ends_with(&needle)))
    });
    method!(string_proto, "slice", 2, |_, this, args| {
        let s = require_string(&this)?;
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len() as i64;
        let start = normalize_index(arg(&args, 0).to_number(), len);
        let end = if args.len() > 1 { normalize_index(arg(&args, 1).to_number(), len) } else { len };
        if start >= end {
            return Ok(Value::string(""));
        }
        Ok(Value::string(chars[start as usize..end as usize].iter().collect::<String>()))
    });
    method!(string_proto, "substring", 2, |_, this, args| {
        let s = require_string(&this)?;
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len() as i64;
        let mut start = arg(&args, 0).to_number().max(0.0).min(len as f64) as i64;
        let mut end = if args.len() > 1 { arg(&args, 1).to_number().max(0.0).min(len as f64) as i64 } else { len };
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        Ok(Value::string(chars[start as usize..end as usize].iter().collect::<String>()))
    });
    method!(string_proto, "substr", 2, |_, this, args| {
        let s = require_string(&this)?;
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len() as i64;
        let start = normalize_index(arg(&args, 0).to_number(), len);
        let count = if args.len() > 1 { arg(&args, 1).to_number().max(0.0) as i64 } else { len - start };
        let end = (start + count).min(len);
        Ok(Value::string(chars[start as usize..end as usize].iter().collect::<String>()))
    });
    method!(string_proto, "split", 2, |it, this, args| {
        let s = require_string(&this)?;
        let limit = if args.len() > 1 && !arg(&args, 1).is_undefined() {
            Some(arg(&args, 1).to_number().max(0.0) as usize)
        } else {
            None
        };
        let parts: Vec<String> = if args.is_empty() || arg(&args, 0).is_undefined() {
            vec![s.clone()]
        } else if let Some((src, flags)) = it.regexp_parts(&arg(&args, 0)) {
            let re = it.regex_for(&src, &flags)?;
            regex_split(&re, &s)
        } else {
            let sep = it.to_string(&arg(&args, 0))?;
            if sep.is_empty() {
                s.chars().map(|c| c.to_string()).collect()
            } else {
                s.split(&sep).map(|p| p.to_string()).collect()
            }
        };
        let parts: Vec<Value> = parts
            .into_iter()
            .take(limit.unwrap_or(usize::MAX))
            .map(Value::string)
            .collect();
        Ok(it.array_from(parts))
    });
    method!(string_proto, "trim", 0, |_, this, _| Ok(Value::string(require_string(&this)?.trim().to_string())));
    method!(string_proto, "trimStart", 0, |_, this, _| Ok(Value::string(require_string(&this)?.trim_start().to_string())));
    method!(string_proto, "trimEnd", 0, |_, this, _| Ok(Value::string(require_string(&this)?.trim_end().to_string())));
    method!(string_proto, "toUpperCase", 0, |_, this, _| Ok(Value::string(require_string(&this)?.to_uppercase())));
    method!(string_proto, "toLowerCase", 0, |_, this, _| Ok(Value::string(require_string(&this)?.to_lowercase())));
    method!(string_proto, "repeat", 1, |_, this, args| {
        let s = require_string(&this)?;
        let n = arg(&args, 0).to_number().max(0.0) as usize;
        if n > 1_000_000 {
            return Err(JsError::range_error("invalid count value"));
        }
        Ok(Value::string(s.repeat(n)))
    });
    method!(string_proto, "padStart", 2, |it, this, args| {
        let s = require_string(&this)?;
        let target = arg(&args, 0).to_number().max(0.0) as usize;
        if target > MAX_ARRAY_LIKE {
            return Err(JsError::range_error("invalid string length"));
        }
        let pad = if args.len() > 1 { it.to_string(&arg(&args, 1))? } else { " ".into() };
        Ok(Value::string(pad_string(&s, target, &pad, true)))
    });
    method!(string_proto, "padEnd", 2, |it, this, args| {
        let s = require_string(&this)?;
        let target = arg(&args, 0).to_number().max(0.0) as usize;
        if target > MAX_ARRAY_LIKE {
            return Err(JsError::range_error("invalid string length"));
        }
        let pad = if args.len() > 1 { it.to_string(&arg(&args, 1))? } else { " ".into() };
        Ok(Value::string(pad_string(&s, target, &pad, false)))
    });
    method!(string_proto, "concat", 1, |it, this, args| {
        let mut s = require_string(&this)?;
        for a in args {
            s.push_str(&it.to_string(&a)?);
        }
        Ok(Value::string(s))
    });
    method!(string_proto, "localeCompare", 1, |it, this, args| {
        let a = require_string(&this)?;
        let b = it.to_string(&arg(&args, 0))?;
        Ok(Value::Number(match a.cmp(&b) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        }))
    });
    method!(string_proto, "replace", 2, |it, this, args| {
        let s = require_string(&this)?;
        let replacement = arg(&args, 1);
        let search = arg(&args, 0);
        if let Some((src, flags)) = it.regexp_parts(&search) {
            let re = it.regex_for(&src, &flags)?;
            let all = flags.contains('g');
            return replace_regex(it, &s, &re, &replacement, all);
        }
        let needle = it.to_string(&search)?;
        Ok(Value::string(replace_one(it, &s, &needle, &replacement)?))
    });
    method!(string_proto, "replaceAll", 2, |it, this, args| {
        let s = require_string(&this)?;
        let replacement = arg(&args, 1);
        let search = arg(&args, 0);
        if let Some((src, flags)) = it.regexp_parts(&search) {
            let re = it.regex_for(&src, &flags)?;
            return replace_regex(it, &s, &re, &replacement, true);
        }
        let needle = it.to_string(&search)?;
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(i) = rest.find(&needle) {
            out.push_str(&rest[..i]);
            out.push_str(&it.to_string(&replacement)?);
            rest = &rest[i + needle.len()..];
        }
        out.push_str(rest);
        Ok(Value::string(out))
    });
    method!(string_proto, "match", 1, |it, this, args| {
        let s = require_string(&this)?;
        let (src, flags) = if let Some(parts) = it.regexp_parts(&arg(&args, 0)) {
            parts
        } else {
            (it.to_string(&arg(&args, 0))?, String::new())
        };
        let re = it.regex_for(&src, &flags)?;
        if flags.contains('g') {
            let vals: Vec<Value> = re.find_iter(&s).map(|m| Value::string(m.as_str(&s).to_string())).collect();
            return Ok(if vals.is_empty() { Value::Null } else { it.array_from(vals) });
        }
        Ok(match_regex_result(it, &re, &s))
    });
    method!(string_proto, "search", 1, |it, this, args| {
        let s = require_string(&this)?;
        let (src, flags) = if let Some(parts) = it.regexp_parts(&arg(&args, 0)) {
            parts
        } else {
            (it.to_string(&arg(&args, 0))?, String::new())
        };
        let re = it.regex_for(&src, &flags)?;
        Ok(Value::Number(match re.find(&s) {
            Some(m) => byte_to_char(&s, m.start()) as f64,
            None => -1.0,
        }))
    });
    method!(string_proto, "matchAll", 1, |it, this, args| {
        let s = require_string(&this)?;
        let (mut src, mut flags) = if let Some(parts) = it.regexp_parts(&arg(&args, 0)) {
            parts
        } else {
            (it.to_string(&arg(&args, 0))?, String::new())
        };
        if !flags.contains('g') {
            flags.push('g');
        }
        if !flags.contains('u') {
            // Regress handles Unicode code points; flag is a hint only.
        }
        let re = it.regex_for(&src, &flags)?;
        let mut out = Vec::new();
        let input = s.clone();
        for m in re.find_iter(&s) {
            let arr = it.array_from(match_values(&s, &m));
            if let Some(o) = arr.as_object() {
                define(o, "index", Value::Number(byte_to_char(&s, m.start()) as f64));
                define(o, "input", Value::string(input.clone()));
            }
            out.push(arr);
        }
        src.clear();
        Ok(it.array_from(out))
    });
    method!(string_proto, "normalize", 0, |_, this, _| Ok(Value::string(require_string(&this)?)));


    // Minimal typed-array and text codec support used by modern tooling
    // bundles. Values are stored as ordinary JS arrays.
    for name in ["Uint8Array", "Uint8ClampedArray", "Int8Array", "Uint16Array", "Int16Array", "Uint32Array", "Int32Array", "Float32Array", "Float64Array"] {
        let ctor = it.native(name, 1, |it, _, args| {
            if let Some(Value::Number(n)) = args.first() {
                return Ok(it.array_from(vec![Value::Number(0.0); (*n as usize).min(MAX_ARRAY_LIKE)]));
            }
            if let Some(v) = args.first() {
                let vals = it.iterate(v).unwrap_or_default();
                return Ok(it.array_from(vals));
            }
            Ok(it.array_from(Vec::new()))
        });
        if let Some(o) = ctor.as_object() {
            define(o, "from", it.native("from", 1, |it, _, args| {
                let vals = args.first().map(|v| it.iterate(v)).transpose()?.unwrap_or_default();
                Ok(it.array_from(vals))
            }));
            define(o, "BYTES_PER_ELEMENT", Value::Number(1.0));
        }
        it.set_global(name, ctor);
    }

    // Binary buffers. Typed arrays in Veyra are ordinary arrays, so these
    // carry the byte length and the methods libraries probe for.
    let array_buffer = it.native("ArrayBuffer", 1, |it, this, args| {
        let len = args.first().map(|v| v.to_index()).unwrap_or(0).min(10_000_000);
        let obj = match this.as_object() {
            Some(o) if it.constructing => Value::Object(o.clone()),
            _ => it.object(),
        };
        if let Some(o) = obj.as_object() {
            define(o, "byteLength", Value::Number(len as f64));
            define(o, "__bytes", it.array_from(vec![Value::Number(0.0); len]));
            define(o, "slice", it.native("slice", 2, |it, _, _| Ok(it.object())));
        }
        Ok(obj)
    });
    if let Some(o) = array_buffer.as_object() {
        define(o, "isView", it.native("isView", 1, |_, _, _| Ok(Value::Bool(false))));
    }
    it.set_global("ArrayBuffer", array_buffer.clone());
    it.set_global("SharedArrayBuffer", array_buffer);

    it.set_global("DataView", it.native("DataView", 1, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => Value::Object(o.clone()),
            _ => it.object(),
        };
        if let Some(o) = obj.as_object() {
            define(o, "buffer", arg(&args, 0));
            define(o, "byteOffset", Value::Number(0.0));
            define(o, "byteLength", Value::Number(arg(&args, 0).as_object().map(|_| 0.0).unwrap_or(0.0)));
            for (name, len, value) in [
                ("getUint8", 1, 0.0), ("getInt8", 1, 0.0), ("getUint16", 1, 0.0),
                ("getInt16", 1, 0.0), ("getUint32", 1, 0.0), ("getInt32", 1, 0.0),
                ("getFloat32", 1, 0.0), ("getFloat64", 1, 0.0), ("getBigInt64", 1, 0.0),
            ] {
                define(o, name, it.native(name, len, move |_, _, _| Ok(Value::Number(value))));
            }
            for name in ["setUint8", "setInt8", "setUint16", "setInt16", "setUint32", "setInt32", "setFloat32", "setFloat64"] {
                define(o, name, it.native(name, 2, |_, _, _| Ok(Value::Undefined)));
            }
        }
        Ok(obj)
    }));

    let atomics = it.object();
    if let Some(a) = atomics.as_object() {
        for name in ["add", "sub", "and", "or", "xor", "exchange", "compareExchange"] {
            define(a, name, it.native(name, 3, |_, _, _| Ok(Value::Number(0.0))));
        }
        define(a, "load", it.native("load", 2, |_, _, _| Ok(Value::Number(0.0))));
        define(a, "store", it.native("store", 3, |_, _, _| Ok(Value::Number(0.0))));
        define(a, "notify", it.native("notify", 3, |_, _, _| Ok(Value::Number(0.0))));
        define(a, "wait", it.native("wait", 4, |_, _, _| Ok(Value::string("not-equal"))));
    }
    it.set_global("Atomics", atomics);

    let node_ctor = it.native("Node", 0, |it, _, _| Ok(it.object()));
    if let Some(o) = node_ctor.as_object() {
        for (name, value) in [
            ("ELEMENT_NODE", 1.0),
            ("TEXT_NODE", 3.0),
            ("COMMENT_NODE", 8.0),
            ("DOCUMENT_NODE", 9.0),
            ("DOCUMENT_FRAGMENT_NODE", 11.0),
        ] {
            define(o, name, Value::Number(value));
        }
    }
    if let Some(o) = node_ctor.as_object() {
        // DOM feature detection reads Node.prototype before deciding which
        // accessor implementation to install.
        define(o, "prototype", it.object());
    }
    it.set_global("Node", node_ctor);

    let text_encoder = it.native("TextEncoder", 0, |it, _, _| {
        let obj = it.object();
        if let Some(o) = obj.as_object() {
            define(o, "encoding", Value::string("utf-8"));
            define(o, "encode", it.native("encode", 1, |it, _, args| {
                let text = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
                Ok(it.array_from(text.bytes().map(|b| Value::Number(b as f64)).collect()))
            }));
        }
        Ok(obj)
    });
    it.set_global("TextEncoder", text_encoder);

    let text_decoder = it.native("TextDecoder", 0, |it, _, _| {
        let obj = it.object();
        if let Some(o) = obj.as_object() {
            define(o, "encoding", Value::string("utf-8"));
            define(o, "decode", it.native("decode", 1, |it, _, args| {
                let values = args.first().map(|v| it.iterate(v)).transpose()?.unwrap_or_default();
                let bytes: Vec<u8> = values
                    .iter()
                    .map(|v| (v.to_number().round().max(0.0) as u32).min(255) as u8)
                    .collect();
                Ok(Value::string(String::from_utf8_lossy(&bytes).to_string()))
            }));
        }
        Ok(obj)
    });
    it.set_global("TextDecoder", text_decoder);

    let symbol_ctor = it.native("Symbol", 0, |it, _, args| {
        let desc = args
            .first()
            .map(|v| it.to_string(v))
            .transpose()?
            .unwrap_or_default();
        let id = it.symbol_seq;
        it.symbol_seq += 1;
        Ok(Value::string(format!("@@symbol:{desc}:{id}")))
    });
    if let Some(o) = symbol_ctor.as_object() {
        define(o, "for", it.native("for", 1, |it, _, args| {
            let key = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            if let Some(sym) = it.symbol_registry.get(&key) {
                return Ok(Value::string(sym.clone()));
            }
            let sym = format!("@@symbol:for:{key}");
            it.symbol_registry.insert(key, sym.clone());
            Ok(Value::string(sym))
        }));
        define(o, "keyFor", it.native("keyFor", 1, |it, _, args| {
            let sym = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            for (k, v) in &it.symbol_registry {
                if *v == sym {
                    return Ok(Value::string(k.clone()));
                }
            }
            Ok(Value::Undefined)
        }));
        define(o, "iterator", Value::string("@@symbol:Symbol.iterator"));
        define(o, "asyncIterator", Value::string("@@symbol:Symbol.asyncIterator"));
        define(o, "toStringTag", Value::string("@@symbol:Symbol.toStringTag"));
        define(o, "hasInstance", Value::string("@@symbol:Symbol.hasInstance"));
        define(o, "toPrimitive", Value::string("@@symbol:Symbol.toPrimitive"));
    }
    it.set_global("Symbol", symbol_ctor);

    let proxy_ctor = it.native("Proxy", 2, |it, _, args| {
        let target = args
            .first()
            .and_then(|v| v.as_object())
            .cloned()
            .ok_or_else(|| JsError::type_error("Proxy target must be an object"))?;
        let handler = args
            .get(1)
            .and_then(|v| v.as_object())
            .cloned()
            .ok_or_else(|| JsError::type_error("Proxy handler must be an object"))?;
        let obj = new_object(target.borrow().proto.clone());
        {
            let mut o = obj.borrow_mut();
            o.props = target.borrow().props.clone();
            o.proxy = Some((target, handler));
        }
        Ok(Value::Object(obj))
    });
    it.set_global("Function", it.native("Function", 1, |it, _, args| {
        // Dynamic functions are compiled in the global scope, like the web
        // API. Feature detectors (MediaWiki's startup is one) probe this.
        let split = args.len().saturating_sub(1);
        let mut params = Vec::new();
        for v in &args[..split] {
            params.push(it.to_string(v)?);
        }
        let body = match args.last() {
            Some(v) => it.to_string(v)?,
            None => String::new(),
        };
        let source = format!(
            "(function anonymous({}\n) {{\n{}\n}})",
            params.join(","),
            body
        );
        it.eval_script(&source)
    }));
    it.set_global("Proxy", proxy_ctor);

    let string_ctor = it.native("String", 1, |it, this, args| {
        let s = if args.is_empty() {
            String::new()
        } else {
            it.to_string(&args[0])?
        };
        if it.constructing {
            if let Some(o) = this.as_object() {
                o.borrow_mut().kind = ObjKind::StringObj(s);
                return Ok(Value::Undefined);
            }
        }
        Ok(Value::string(s))
    });
    define(&string_proto, "constructor", string_ctor.clone());
    if let Some(o) = string_ctor.as_object() {
        define(o, "prototype", Value::Object(string_proto.clone()));
        define(o, "raw", it.native("raw", 1, |it, _, args| {
            let strings = args.first().cloned().unwrap_or(Value::Undefined);
            let values = match &strings {
                Value::Object(o) => match &o.borrow().kind {
                    ObjKind::Array(v) => v.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            let mut out = String::new();
            for (i, part) in values.iter().enumerate() {
                out.push_str(&it.to_string(part)?);
                if let Some(v) = args.get(i + 1) {
                    out.push_str(&it.to_string(v)?);
                }
            }
            Ok(Value::string(out))
        }));
        define(o, "fromCharCode", it.native("fromCharCode", 1, |_, _, args| {
            let s: String = args
                .iter()
                .filter_map(|v| char::from_u32(v.to_u32()))
                .collect();
            Ok(Value::string(s))
        }));
        define(o, "fromCodePoint", it.native("fromCodePoint", 1, |_, _, args| {
            let s: String = args
                .iter()
                .filter_map(|v| char::from_u32(v.to_u32()))
                .collect();
            Ok(Value::string(s))
        }));
    }

    // ------------------------------------------------------------------
    // Number / Boolean
    // ------------------------------------------------------------------
    method!(number_proto, "valueOf", 0, |_, this, _| Ok(Value::Number(require_number(&this)?)));
    method!(number_proto, "toString", 1, |_, this, args| {
        let n = require_number(&this)?;
        let radix = if args.is_empty() { 10 } else { arg(&args, 0).to_number() as u32 };
        Ok(Value::string(number_radix(n, radix)))
    });
    method!(number_proto, "toFixed", 1, |_, this, args| {
        let n = require_number(&this)?;
        let digits = arg(&args, 0).to_number().clamp(0.0, 100.0) as usize;
        Ok(Value::string(format!("{n:.digits$}")))
    });
    method!(number_proto, "toPrecision", 1, |_, this, args| {
        let n = require_number(&this)?;
        if args.is_empty() || arg(&args, 0).is_undefined() {
            return Ok(Value::string(number_to_string(n)));
        }
        let p = arg(&args, 0).to_number().clamp(0.0, 100.0) as usize;
        Ok(Value::string(format!("{n:.p$}")))
    });
    method!(number_proto, "toExponential", 1, |_, this, args| {
        let n = require_number(&this)?;
        let digits = if args.is_empty() { 6 } else { arg(&args, 0).to_number().clamp(0.0, 100.0) as usize };
        Ok(Value::string(format!("{n:.digits$e}")))
    });

    let number_ctor = it.native("Number", 1, |it, this, args| {
        let n = if args.is_empty() {
            0.0
        } else {
            it.to_number_value(&args[0])?
        };
        if it.constructing {
            if let Some(o) = this.as_object() {
                o.borrow_mut().kind = ObjKind::NumberObj(n);
                return Ok(Value::Undefined);
            }
        }
        Ok(Value::Number(n))
    });
    define(&number_proto, "constructor", number_ctor.clone());
    if let Some(o) = number_ctor.as_object() {
        define(o, "prototype", Value::Object(number_proto.clone()));
        for (name, value) in [
            ("MAX_VALUE", f64::MAX),
            ("MIN_VALUE", f64::MIN_POSITIVE),
            ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
            ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
            ("POSITIVE_INFINITY", f64::INFINITY),
            ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
            ("NaN", f64::NAN),
            ("EPSILON", f64::EPSILON),
        ] {
            define(o, name, Value::Number(value));
        }
        define(o, "isNaN", it.native("isNaN", 1, |_, _, args| Ok(Value::Bool(arg(&args, 0).is_number_nan()))));
        define(o, "isFinite", it.native("isFinite", 1, |_, _, args| {
            Ok(Value::Bool(matches!(arg(&args, 0), Value::Number(n) if n.is_finite())))
        }));
        define(o, "isInteger", it.native("isInteger", 1, |_, _, args| {
            Ok(Value::Bool(matches!(arg(&args, 0), Value::Number(n) if n.is_finite() && n.fract() == 0.0)))
        }));
        define(o, "isSafeInteger", it.native("isSafeInteger", 1, |_, _, args| {
            Ok(Value::Bool(matches!(arg(&args, 0), Value::Number(n) if n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0)))
        }));
        define(o, "parseInt", it.native("parseInt", 2, |it, _, args| Ok(Value::Number(parse_int_js(&it.to_string(&arg(&args, 0))?, arg(&args, 1).to_number() as u32)))));
        define(o, "parseFloat", it.native("parseFloat", 1, |it, _, args| {
            let s = it.to_string(&arg(&args, 0))?;
            Ok(Value::Number(parse_float_js(&s)))
        }));
    }

    let boolean_ctor = it.native("Boolean", 1, |it, this, args| {
        let b = arg(&args, 0).truthy();
        if it.constructing {
            if let Some(o) = this.as_object() {
                o.borrow_mut().kind = ObjKind::BooleanObj(b);
                return Ok(Value::Undefined);
            }
        }
        Ok(Value::Bool(b))
    });
    method!(boolean_proto, "valueOf", 0, |_, this, _| Ok(Value::Bool(require_boolean(&this)?)));
    method!(boolean_proto, "toString", 0, |_, this, _| Ok(Value::string(require_boolean(&this)?.to_string())));
    define(&boolean_proto, "constructor", boolean_ctor.clone());
    if let Some(o) = boolean_ctor.as_object() {
        define(o, "prototype", Value::Object(boolean_proto.clone()));
    }

    // ------------------------------------------------------------------
    // Math, JSON, console
    // ------------------------------------------------------------------
    let math = it.object();
    if let Some(m) = math.as_object() {
        for (name, value) in [
            ("PI", std::f64::consts::PI),
            ("E", std::f64::consts::E),
            ("LN2", std::f64::consts::LN_2),
            ("LN10", std::f64::consts::LN_10),
            ("LOG2E", std::f64::consts::LOG2_E),
            ("LOG10E", std::f64::consts::LOG10_E),
            ("SQRT2", std::f64::consts::SQRT_2),
            ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
        ] {
            define(m, name, Value::Number(value));
        }
        macro_rules! math1 {
            ($name:expr, $f:expr) => {{
                let f = it.native($name, 1, move |it, _, args| {
                    let x = it.to_number_value(&arg(&args, 0))?;
                    Ok(Value::Number($f(x)))
                });
                define(m, $name, f);
            }};
        }
        math1!("abs", f64::abs);
        math1!("ceil", f64::ceil);
        math1!("floor", f64::floor);
        math1!("round", |x: f64| (x + 0.5).floor());
        math1!("trunc", f64::trunc);
        math1!("sqrt", f64::sqrt);
        math1!("cbrt", f64::cbrt);
        math1!("exp", f64::exp);
        math1!("ln", f64::ln);
        math1!("log2", f64::log2);
        math1!("log10", f64::log10);
        math1!("sin", f64::sin);
        math1!("cos", f64::cos);
        math1!("tan", f64::tan);
        math1!("asin", f64::asin);
        math1!("acos", f64::acos);
        math1!("atan", f64::atan);
        math1!("sign", f64::signum);
        math1!("fround", |x: f64| x as f32 as f64);
        define(m, "pow", it.native("pow", 2, |it, _, args| {
            Ok(Value::Number(it.to_number_value(&arg(&args, 0))?.powf(it.to_number_value(&arg(&args, 1))?)))
        }));
        define(m, "atan2", it.native("atan2", 2, |it, _, args| {
            Ok(Value::Number(it.to_number_value(&arg(&args, 0))?.atan2(it.to_number_value(&arg(&args, 1))?)))
        }));
        define(m, "hypot", it.native("hypot", 2, |it, _, args| {
            let mut sum = 0.0;
            for a in &args {
                let n = it.to_number_value(a)?;
                sum += n * n;
            }
            Ok(Value::Number(sum.sqrt()))
        }));
        define(m, "min", it.native("min", 2, |it, _, args| {
            let mut best = f64::INFINITY;
            for a in &args {
                let n = it.to_number_value(a)?;
                if n.is_nan() {
                    return Ok(Value::Number(f64::NAN));
                }
                best = best.min(n);
            }
            Ok(Value::Number(best))
        }));
        define(m, "max", it.native("max", 2, |it, _, args| {
            let mut best = f64::NEG_INFINITY;
            for a in &args {
                let n = it.to_number_value(a)?;
                if n.is_nan() {
                    return Ok(Value::Number(f64::NAN));
                }
                best = best.max(n);
            }
            Ok(Value::Number(best))
        }));
        define(m, "random", it.native("random", 0, |it, _, _| Ok(Value::Number(it.random()))));
        define(m, "imul", it.native("imul", 2, |_, _, args| {
            Ok(Value::Number((arg(&args, 0).to_i32().wrapping_mul(arg(&args, 1).to_i32())) as f64))
        }));
        define(m, "clz32", it.native("clz32", 1, |_, _, args| {
            Ok(Value::Number(arg(&args, 0).to_u32().leading_zeros() as f64))
        }));
    }
    it.set_global("Math", math);

    let json_obj = it.object();
    if let Some(j) = json_obj.as_object() {
        define(j, "stringify", it.native("stringify", 1, |it, _, args| {
            let v = arg(&args, 0);
            if v.is_callable() || v.is_undefined() {
                return Ok(Value::Undefined);
            }
            Ok(Value::string(it.json_stringify(&v)?))
        }));
        define(j, "parse", it.native("parse", 1, |it, _, args| {
            let s = it.to_string(&arg(&args, 0))?;
            it.json_parse(&s)
        }));
    }
    it.set_global("JSON", json_obj);

    let console = it.object();
    if let Some(c) = console.as_object() {
        macro_rules! logger {
            ($name:expr, $level:expr) => {{
                let f = it.native($name, 0, move |it, _, args| {
                    let mut parts = Vec::new();
                    for a in &args {
                        parts.push(it.to_string(a)?);
                    }
                    let msg = parts.join(" ");
                    it.output.push(msg.clone());
                    it.host.log($level, &msg);
                    Ok(Value::Undefined)
                });
                define(c, $name, f);
            }};
        }
        logger!("log", "info");
        logger!("info", "info");
        logger!("warn", "warn");
        logger!("error", "error");
        logger!("debug", "debug");
    }
    it.set_global("console", console);

    // ------------------------------------------------------------------
    // Errors
    // ------------------------------------------------------------------
    for (name, proto) in [
        ("Error", error_proto.clone()),
        ("TypeError", type_error_proto.clone()),
        ("RangeError", range_error_proto.clone()),
        ("SyntaxError", syntax_error_proto.clone()),
        ("ReferenceError", reference_error_proto.clone()),
    ] {
        let kind = name.to_string();
        let ctor = it.native(name, 1, move |it, _, args| {
            let msg = match args.first() {
                Some(v) if !v.is_undefined() => it.to_string(v)?,
                _ => String::new(),
            };
            Ok(it.make_error(&kind, &msg))
        });
        define(&proto, "constructor", ctor.clone());
        define(&proto, "name", Value::string(name));
        define(&proto, "message", Value::string(""));
        method!(proto, "toString", 0, |it, this, _| Ok(Value::string(it.to_string(&this)?)));
        if let Some(o) = ctor.as_object() {
            define(o, "prototype", Value::Object(proto.clone()));
        }
        it.set_global(name, ctor);
    }

    // ------------------------------------------------------------------
    // Date
    // ------------------------------------------------------------------
    let date_ctor = it.native("Date", 7, |it, this, args| {
        let now = system_now_ms();
        if !it.constructing {
            return Ok(Value::string(format_date(now)));
        }
        let ms = if args.is_empty() {
            now
        } else if args.len() == 1 {
            match &args[0] {
                Value::String(s) => parse_date(s),
                v => v.to_number(),
            }
        } else {
            let y = args[0].to_number() as i64;
            let m = args.get(1).map(|v| v.to_number() as i64).unwrap_or(0);
            let d = args.get(2).map(|v| v.to_number() as i64).unwrap_or(1);
            let hh = args.get(3).map(|v| v.to_number() as i64).unwrap_or(0);
            let mm = args.get(4).map(|v| v.to_number() as i64).unwrap_or(0);
            let ss = args.get(5).map(|v| v.to_number() as i64).unwrap_or(0);
            let milli = args.get(6).map(|v| v.to_number() as i64).unwrap_or(0);
            (days_from_civil(y, (m + 1) as u32, d.max(1) as u32) * 86_400_000
                + hh * 3_600_000
                + mm * 60_000
                + ss * 1000
                + milli) as f64
        };
        if let Some(o) = this.as_object() {
            o.borrow_mut().kind = ObjKind::Date(ms);
            return Ok(Value::Undefined);
        }
        Ok(Value::string(format_date(ms)))
    });
    method!(date_proto, "valueOf", 0, |_, this, _| Ok(Value::Number(require_date(&this)?)));
    method!(date_proto, "getTime", 0, |_, this, _| Ok(Value::Number(require_date(&this)?)));
    method!(date_proto, "toISOString", 0, |_, this, _| Ok(Value::string(format_date(require_date(&this)?))));
    method!(date_proto, "toJSON", 0, |_, this, _| Ok(Value::string(format_date(require_date(&this)?))));
    method!(date_proto, "toString", 0, |_, this, _| Ok(Value::string(format_date(require_date(&this)?))));
    method!(date_proto, "toLocaleDateString", 0, |_, this, _| {
        let (y, m, d, _, _, _, _) = date_parts(require_date(&this)?);
        Ok(Value::string(format!("{}/{}/{}", m + 1, d, y)))
    });
    method!(date_proto, "toLocaleTimeString", 0, |_, this, _| {
        let (_, _, _, hh, mm, ss, _) = date_parts(require_date(&this)?);
        Ok(Value::string(format!("{hh:02}:{mm:02}:{ss:02}")))
    });
    method!(date_proto, "toUTCString", 0, |_, this, _| {
        Ok(Value::string(utc_date_string(require_date(&this)?)))
    });
    method!(date_proto, "toGMTString", 0, |_, this, _| {
        Ok(Value::string(utc_date_string(require_date(&this)?)))
    });
    method!(date_proto, "toDateString", 0, |_, this, _| {
        let ms = require_date(&this)?;
        if !ms.is_finite() {
            return Ok(Value::string("Invalid Date"));
        }
        let (y, m, d, ..) = date_parts(ms);
        Ok(Value::string(format!(
            "{} {} {} {}",
            WEEKDAYS[weekday(ms) as usize], MONTHS[m as usize], d, y
        )))
    });
    method!(date_proto, "toLocaleString", 0, |_, this, _| {
        let (y, m, d, hh, mm, ss, _) = date_parts(require_date(&this)?);
        Ok(Value::string(format!("{}/{}/{}, {:02}:{:02}:{:02}", m + 1, d, y, hh, mm, ss)))
    });
    for (name, index) in [
        ("getFullYear", 0),
        ("getUTCFullYear", 0),
        ("getMonth", 1),
        ("getUTCMonth", 1),
        ("getDate", 2),
        ("getUTCDate", 2),
        ("getDay", 6),
        ("getUTCDay", 6),
        ("getHours", 3),
        ("getUTCHours", 3),
        ("getMinutes", 4),
        ("getUTCMinutes", 4),
        ("getSeconds", 5),
        ("getUTCSeconds", 5),
        ("getMilliseconds", 7),
        ("getUTCMilliseconds", 7),
    ] {
        let f = it.native(name, 0, move |_, this, _| {
            let ms = require_date(&this)?;
            let parts = date_parts(ms);
            let v = match index {
                0 => parts.0 as f64,
                1 => parts.1 as f64,
                2 => parts.2 as f64,
                3 => parts.3 as f64,
                4 => parts.4 as f64,
                5 => parts.5 as f64,
                6 => weekday(ms),
                _ => parts.6 as f64,
            };
            Ok(Value::Number(v))
        });
        define(&date_proto, name, f);
    }
    define(&date_proto, "getTimezoneOffset", it.native("getTimezoneOffset", 0, |_, _, _| Ok(Value::Number(0.0))));
    define(&date_proto, "constructor", date_ctor.clone());
    if let Some(o) = date_ctor.as_object() {
        define(o, "prototype", Value::Object(date_proto.clone()));
        define(o, "now", it.native("now", 0, |_, _, _| Ok(Value::Number(system_now_ms()))));
        define(o, "parse", it.native("parse", 1, |it, _, args| {
            Ok(Value::Number(parse_date(&it.to_string(&arg(&args, 0))?)))
        }));
        define(o, "UTC", it.native("UTC", 7, |it, _, args| {
            let y = it.to_number_value(&arg(&args, 0))? as i64;
            let m = it.to_number_value(&arg(&args, 1))? as i64;
            let d = it.to_number_value(&arg(&args, 2))? as i64;
            let hh = it.to_number_value(&arg(&args, 3))? as i64;
            let mm = it.to_number_value(&arg(&args, 4))? as i64;
            let ss = it.to_number_value(&arg(&args, 5))? as i64;
            let milli = it.to_number_value(&arg(&args, 6))? as i64;
            Ok(Value::Number(
                (days_from_civil(y, (m + 1) as u32, d.max(1) as u32) * 86_400_000
                    + hh * 3_600_000
                    + mm * 60_000
                    + ss * 1000
                    + milli) as f64,
            ))
        }));
    }
    it.set_global("Date", date_ctor);

    // ------------------------------------------------------------------
    // RegExp
    // ------------------------------------------------------------------
    let regexp_ctor = it.native("RegExp", 2, |it, this, args| {
        let (source, flags) = match args.first() {
            Some(Value::Object(o)) => {
                if let ObjKind::RegExp { source, flags, .. } = &o.borrow().kind {
                    (source.clone(), flags.clone())
                } else {
                    (it.to_string(&args[0])?, String::new())
                }
            }
            Some(v) if !v.is_undefined() => (it.to_string(v)?, String::new()),
            _ => (String::new(), String::new()),
        };
        let flags = if args.len() > 1 { it.to_string(&args[1])? } else { flags };
        let obj = match this.as_object() {
            Some(o) if it.constructing => o.clone(),
            _ => Rc::new(RefCell::new(Object {
                proto: Some(it.intrinsics.regexp_proto.clone()),
                props: Default::default(),
                kind: ObjKind::Ordinary,
                array_holes: None,
                extensible: true,
                proxy: None,
            })),
        };
        obj.borrow_mut().kind = ObjKind::RegExp { source, flags, last_index: 0 };
        Ok(Value::Object(obj))
    });
    method!(regexp_proto, "test", 1, |it, this, args| {
        let (src, flags) = it.regexp_parts(&this).ok_or_else(|| JsError::type_error("not a regexp"))?;
        let re = it.regex_for(&src, &flags)?;
        let s = it.to_string(&arg(&args, 0))?;
        let global = flags.contains('g');
        let start = if global { it.get_member(this.clone(), "lastIndex")?.to_index() } else { 0 };
        let found = re.find_from(&s, start.min(s.len())).next();
        if global {
            let next = found.as_ref().map(|m| if m.end() == m.start() { m.end() + 1 } else { m.end() }).unwrap_or(0);
            it.set_member(this.clone(), "lastIndex", Value::Number(next as f64))?;
        }
        Ok(Value::Bool(found.is_some()))
    });
    method!(regexp_proto, "exec", 1, |it, this, args| {
        let (src, flags) = it.regexp_parts(&this).ok_or_else(|| JsError::type_error("not a regexp"))?;
        let re = it.regex_for(&src, &flags)?;
        let s = it.to_string(&arg(&args, 0))?;
        let global = flags.contains('g');
        let start = if global { it.get_member(this.clone(), "lastIndex")?.to_index() } else { 0 };
        let caps = re.find_from(&s, start.min(s.len())).next();
        if global {
            let next = caps
                .as_ref()
                .map(|m| if m.end() == m.start() { m.end() + 1 } else { m.end() })
                .unwrap_or(0);
            it.set_member(this.clone(), "lastIndex", Value::Number(next as f64))?;
        }
        match caps {
            Some(m) => {
                let arr = it.array_from(match_values(&s, &m));
                if let Some(o) = arr.as_object() {
                    define(o, "index", Value::Number(byte_to_char(&s, m.start()) as f64));
                    define(o, "input", Value::string(s.clone()));
                }
                Ok(arr)
            }
            None => Ok(Value::Null),
        }
    });
    method!(regexp_proto, "toString", 0, |it, this, _| Ok(Value::string(it.to_string(&this)?)));
    getter!(regexp_proto, "source", |it, this, _| {
        let (src, _) = it.regexp_parts(&this).ok_or_else(|| JsError::type_error("not a regexp"))?;
        Ok(Value::string(src))
    });
    getter!(regexp_proto, "flags", |it, this, _| {
        let (_, flags) = it.regexp_parts(&this).ok_or_else(|| JsError::type_error("not a regexp"))?;
        Ok(Value::string(flags))
    });
    getter!(regexp_proto, "global", |it, this, _| Ok(Value::Bool(it.regexp_parts(&this).map(|(_, f)| f.contains('g')).unwrap_or(false))));
    getter!(regexp_proto, "ignoreCase", |it, this, _| Ok(Value::Bool(it.regexp_parts(&this).map(|(_, f)| f.contains('i')).unwrap_or(false))));
    getter!(regexp_proto, "multiline", |it, this, _| Ok(Value::Bool(it.regexp_parts(&this).map(|(_, f)| f.contains('m')).unwrap_or(false))));
    getter!(regexp_proto, "dotAll", |it, this, _| Ok(Value::Bool(it.regexp_parts(&this).map(|(_, f)| f.contains('s')).unwrap_or(false))));
    getter!(regexp_proto, "sticky", |it, this, _| Ok(Value::Bool(it.regexp_parts(&this).map(|(_, f)| f.contains('y')).unwrap_or(false))));
    getter!(regexp_proto, "unicode", |it, this, _| Ok(Value::Bool(it.regexp_parts(&this).map(|(_, f)| f.contains('u')).unwrap_or(false))));
    getter!(regexp_proto, "hasIndices", |it, this, _| Ok(Value::Bool(it.regexp_parts(&this).map(|(_, f)| f.contains('d')).unwrap_or(false))));
    let last_index_get = it.native("lastIndex", 0, |_, this, _| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a regexp"))?;
        let idx = match &obj.borrow().kind {
            ObjKind::RegExp { last_index, .. } => *last_index,
            _ => 0,
        };
        Ok(Value::Number(idx as f64))
    });
    let last_index_set = it.native("lastIndex", 1, |_, this, args| {
        if let Some(obj) = this.as_object() {
            if let ObjKind::RegExp { last_index, .. } = &mut obj.borrow_mut().kind {
                *last_index = arg(&args, 0).to_index();
            }
        }
        Ok(Value::Undefined)
    });
    define_accessor(&regexp_proto, "lastIndex", Some(last_index_get), Some(last_index_set), false);
    define(&regexp_proto, "constructor", regexp_ctor.clone());
    if let Some(o) = regexp_ctor.as_object() {
        define(o, "prototype", Value::Object(regexp_proto.clone()));
    }
    it.set_global("RegExp", regexp_ctor);

    // Array iteration protocol. Polyfills feature-detect this before using
    // native Map/Set constructors with their own iterators.
    let array_values_iter = it.native("values", 0, |it, this, _| {
        let values = array_values(&this).unwrap_or_default();
        Ok(make_iterator(it, values))
    });
    define(&array_proto, "values", array_values_iter.clone());
    define(&array_proto, "@@symbol:Symbol.iterator", array_values_iter);

    // ------------------------------------------------------------------
    // Map / Set
    // ------------------------------------------------------------------
    let map_ctor = it.native("Map", 0, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => o.clone(),
            _ => Rc::new(RefCell::new(Object {
                proto: Some(it.intrinsics.map_proto.clone()),
                props: Default::default(),
                kind: ObjKind::Ordinary,
                array_holes: None,
                extensible: true,
                proxy: None,
            })),
        };
        obj.borrow_mut().kind = ObjKind::Map(Vec::new());
        if let Some(Value::Object(init)) = args.first() {
            let entries = it.iterate(&Value::Object(init.clone()))?;
            for e in entries {
                let pair = it.iterate(&e)?;
                let k = pair.first().cloned().unwrap_or(Value::Undefined);
                let v = pair.get(1).cloned().unwrap_or(Value::Undefined);
                if let ObjKind::Map(m) = &mut obj.borrow_mut().kind {
                    if !m.iter().any(|(ek, _)| ek.same_value_zero(&k)) {
                        m.push((k, v));
                    }
                }
            }
        }
        Ok(Value::Object(obj))
    });
    method!(map_proto, "set", 2, |_, this, args| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Map"))?;
        let key = arg(&args, 0);
        let value = arg(&args, 1);
        if let ObjKind::Map(m) = &mut obj.borrow_mut().kind {
            if let Some(entry) = m.iter_mut().find(|(k, _)| k.same_value_zero(&key)) {
                entry.1 = value;
            } else {
                m.push((key, value));
            }
        }
        Ok(this)
    });
    method!(map_proto, "get", 1, |_, this, args| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Map"))?;
        let key = arg(&args, 0);
        if let ObjKind::Map(m) = &obj.borrow().kind {
            Ok(m.iter().find(|(k, _)| k.same_value_zero(&key)).map(|(_, v)| v.clone()).unwrap_or(Value::Undefined))
        } else {
            Ok(Value::Undefined)
        }
    });
    method!(map_proto, "has", 1, |_, this, args| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Map"))?;
        let key = arg(&args, 0);
        if let ObjKind::Map(m) = &obj.borrow().kind {
            Ok(Value::Bool(m.iter().any(|(k, _)| k.same_value_zero(&key))))
        } else {
            Ok(Value::Bool(false))
        }
    });
    method!(map_proto, "delete", 1, |_, this, args| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Map"))?;
        let key = arg(&args, 0);
        let mut removed = false;
        if let ObjKind::Map(m) = &mut obj.borrow_mut().kind {
            let before = m.len();
            m.retain(|(k, _)| !k.same_value_zero(&key));
            removed = m.len() != before;
        }
        Ok(Value::Bool(removed))
    });
    method!(map_proto, "clear", 0, |_, this, _| {
        if let Some(obj) = this.as_object() {
            if let ObjKind::Map(m) = &mut obj.borrow_mut().kind {
                m.clear();
            }
        }
        Ok(Value::Undefined)
    });
    method!(map_proto, "forEach", 1, |it, this, args| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Map"))?;
        let cb = arg(&args, 0);
        let entries = if let ObjKind::Map(m) = &obj.borrow().kind { m.clone() } else { Vec::new() };
        for (k, v) in entries {
            it.call_value(&cb, Value::Undefined, vec![v, k, this.clone()])?;
        }
        Ok(Value::Undefined)
    });
    method!(map_proto, "keys", 0, |it, this, _| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Map"))?;
        let keys = if let ObjKind::Map(m) = &obj.borrow().kind { m.iter().map(|(k, _)| k.clone()).collect() } else { Vec::new() };
        Ok(make_iterator(it, keys))
    });
    method!(map_proto, "values", 0, |it, this, _| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Map"))?;
        let vals = if let ObjKind::Map(m) = &obj.borrow().kind { m.iter().map(|(_, v)| v.clone()).collect() } else { Vec::new() };
        Ok(make_iterator(it, vals))
    });
    method!(map_proto, "entries", 0, |it, this, _| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Map"))?;
        let entries = if let ObjKind::Map(m) = &obj.borrow().kind { m.clone() } else { Vec::new() };
        let pairs: Vec<Value> = entries.into_iter().map(|(k, v)| it.array_from(vec![k, v])).collect();
        Ok(make_iterator(it, pairs))
    });
    getter!(map_proto, "size", |_, this, _| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Map"))?;
        let len = if let ObjKind::Map(m) = &obj.borrow().kind { m.len() } else { 0 };
        Ok(Value::Number(len as f64))
    });
    let entries_iter = { map_proto.borrow().props.get("entries").map(|p| p.value.clone()) };
    if let Some(entries) = entries_iter {
        define(&map_proto, "@@symbol:Symbol.iterator", entries);
    }
    define(&map_proto, "constructor", map_ctor.clone());
    if let Some(o) = map_ctor.as_object() {
        define(o, "prototype", Value::Object(map_proto.clone()));
    }
    it.set_global("Map", map_ctor.clone());
    it.set_global("WeakMap", map_ctor);

    let set_ctor = it.native("Set", 0, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => o.clone(),
            _ => Rc::new(RefCell::new(Object {
                proto: Some(it.intrinsics.set_proto.clone()),
                props: Default::default(),
                kind: ObjKind::Ordinary,
                array_holes: None,
                extensible: true,
                proxy: None,
            })),
        };
        obj.borrow_mut().kind = ObjKind::Set(Vec::new());
        if let Some(v) = args.first() {
            for value in it.iterate(v)? {
                if let ObjKind::Set(s) = &mut obj.borrow_mut().kind {
                    if !s.iter().any(|x| x.same_value_zero(&value)) {
                        s.push(value);
                    }
                }
            }
        }
        Ok(Value::Object(obj))
    });
    method!(set_proto, "add", 1, |_, this, args| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Set"))?;
        let value = arg(&args, 0);
        if let ObjKind::Set(s) = &mut obj.borrow_mut().kind {
            if !s.iter().any(|x| x.same_value_zero(&value)) {
                s.push(value);
            }
        }
        Ok(this)
    });
    method!(set_proto, "has", 1, |_, this, args| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Set"))?;
        let value = arg(&args, 0);
        if let ObjKind::Set(s) = &obj.borrow().kind {
            Ok(Value::Bool(s.iter().any(|x| x.same_value_zero(&value))))
        } else {
            Ok(Value::Bool(false))
        }
    });
    method!(set_proto, "delete", 1, |_, this, args| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Set"))?;
        let value = arg(&args, 0);
        let mut removed = false;
        if let ObjKind::Set(s) = &mut obj.borrow_mut().kind {
            let before = s.len();
            s.retain(|x| !x.same_value_zero(&value));
            removed = s.len() != before;
        }
        Ok(Value::Bool(removed))
    });
    method!(set_proto, "clear", 0, |_, this, _| {
        if let Some(obj) = this.as_object() {
            if let ObjKind::Set(s) = &mut obj.borrow_mut().kind {
                s.clear();
            }
        }
        Ok(Value::Undefined)
    });
    method!(set_proto, "forEach", 1, |it, this, args| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Set"))?;
        let cb = arg(&args, 0);
        let values = if let ObjKind::Set(s) = &obj.borrow().kind { s.clone() } else { Vec::new() };
        for v in values {
            it.call_value(&cb, Value::Undefined, vec![v.clone(), v, this.clone()])?;
        }
        Ok(Value::Undefined)
    });
    method!(set_proto, "values", 0, |it, this, _| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Set"))?;
        let values = if let ObjKind::Set(s) = &obj.borrow().kind { s.clone() } else { Vec::new() };
        Ok(make_iterator(it, values))
    });
    method!(set_proto, "keys", 0, |it, this, _| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Set"))?;
        let values = if let ObjKind::Set(s) = &obj.borrow().kind { s.clone() } else { Vec::new() };
        Ok(make_iterator(it, values))
    });
    method!(set_proto, "entries", 0, |it, this, _| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Set"))?;
        let values = if let ObjKind::Set(s) = &obj.borrow().kind { s.clone() } else { Vec::new() };
        let pairs: Vec<Value> = values.into_iter().map(|v| it.array_from(vec![v.clone(), v])).collect();
        Ok(make_iterator(it, pairs))
    });
    getter!(set_proto, "size", |_, this, _| {
        let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Set"))?;
        let len = if let ObjKind::Set(s) = &obj.borrow().kind { s.len() } else { 0 };
        Ok(Value::Number(len as f64))
    });
    let values_iter = { set_proto.borrow().props.get("values").map(|p| p.value.clone()) };
    if let Some(values) = values_iter {
        define(&set_proto, "@@symbol:Symbol.iterator", values);
    }
    define(&set_proto, "constructor", set_ctor.clone());
    if let Some(o) = set_ctor.as_object() {
        define(o, "prototype", Value::Object(set_proto.clone()));
    }
    it.set_global("Set", set_ctor.clone());
    it.set_global("WeakSet", set_ctor);

    // ------------------------------------------------------------------
    // WeakRef / FinalizationRegistry
    // ------------------------------------------------------------------
    // JavaScript cannot observe collection timing, so holding the target
    // strongly is a conforming implementation of WeakRef. Engine code and
    // pages allocate far too little for this to retain significant memory.
    let weak_ref_ctor = it.native("WeakRef", 1, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => o.clone(),
            _ => Rc::new(RefCell::new(Object {
                proto: Some(it.intrinsics.object_proto.clone()),
                props: Default::default(),
                kind: ObjKind::Ordinary,
                array_holes: None,
                extensible: true,
                proxy: None,
            })),
        };
        define(&obj, "__weakTarget", arg(&args, 0));
        define(
            &obj,
            "deref",
            it.native("deref", 0, |it, this, _| {
                let target = it.get_member(this, "__weakTarget")?;
                Ok(target)
            }),
        );
        Ok(Value::Object(obj))
    });
    if let Some(o) = weak_ref_ctor.as_object() {
        define(o, "prototype", it.object());
    }
    it.set_global("WeakRef", weak_ref_ctor);

    let finalization_ctor = it.native("FinalizationRegistry", 1, |it, this, _args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => o.clone(),
            _ => Rc::new(RefCell::new(Object {
                proto: Some(it.intrinsics.object_proto.clone()),
                props: Default::default(),
                kind: ObjKind::Ordinary,
                array_holes: None,
                extensible: true,
                proxy: None,
            })),
        };
        define(&obj, "register", it.native("register", 2, |_, _, _| Ok(Value::Undefined)));
        define(&obj, "unregister", it.native("unregister", 1, |_, _, _| Ok(Value::Bool(false))));
        define(&obj, "cleanupSome", it.native("cleanupSome", 0, |_, _, _| Ok(Value::Undefined)));
        Ok(Value::Object(obj))
    });
    if let Some(o) = finalization_ctor.as_object() {
        define(o, "prototype", it.object());
    }
    it.set_global("FinalizationRegistry", finalization_ctor);

    // ------------------------------------------------------------------
    // Promise
    // ------------------------------------------------------------------
    let promise_ctor = it.native("Promise", 1, |it, _, args| {
        let (promise, data) = it.new_promise();
        if let Some(executor) = args.first().cloned() {
            let resolve = it.native("resolve", 1, {
                let data = data.clone();
                move |it, _, args| {
                    it.resolve_promise(&data, args.first().cloned().unwrap_or(Value::Undefined));
                    Ok(Value::Undefined)
                }
            });
            let reject = it.native("reject", 1, {
                let data = data.clone();
                move |it, _, args| {
                    it.reject_promise(&data, args.first().cloned().unwrap_or(Value::Undefined));
                    Ok(Value::Undefined)
                }
            });
            if let Err(e) = it.call_value(&executor, Value::Undefined, vec![resolve, reject]) {
                let v = it.error_value(&e);
                it.reject_promise(&data, v);
            }
        }
        Ok(promise)
    });
    method!(promise_proto, "then", 2, |it, this, args| {
        let data = promise_data(&this)?;
        Ok(it.promise_then(&data, arg(&args, 0), arg(&args, 1)))
    });
    method!(promise_proto, "catch", 1, |it, this, args| {
        let data = promise_data(&this)?;
        Ok(it.promise_then(&data, Value::Undefined, arg(&args, 0)))
    });
    method!(promise_proto, "finally", 1, |it, this, args| {
        let data = promise_data(&this)?;
        let cb = arg(&args, 0);
        let on_f = it.native("", 1, {
            let cb = cb.clone();
            move |it, _, args| {
                if cb.is_callable() {
                    it.call_value(&cb, Value::Undefined, vec![])?;
                }
                Ok(args.first().cloned().unwrap_or(Value::Undefined))
            }
        });
        let on_r = it.native("", 1, {
            let cb = cb.clone();
            move |it, _, args| {
                if cb.is_callable() {
                    it.call_value(&cb, Value::Undefined, vec![])?;
                }
                Err(JsError::throw(args.first().cloned().unwrap_or(Value::Undefined)))
            }
        });
        Ok(it.promise_then(&data, on_f, on_r))
    });
    define(&promise_proto, "constructor", promise_ctor.clone());
    if let Some(o) = promise_ctor.as_object() {
        define(o, "prototype", Value::Object(promise_proto.clone()));
        define(o, "resolve", it.native("resolve", 1, |it, _, args| {
            let v = arg(&args, 0);
            if matches!(v.as_object().map(|o| matches!(o.borrow().kind, ObjKind::Promise(_))), Some(true)) {
                return Ok(v);
            }
            let (p, data) = it.new_promise();
            it.resolve_promise(&data, v);
            Ok(p)
        }));
        define(o, "reject", it.native("reject", 1, |it, _, args| {
            let (p, data) = it.new_promise();
            it.reject_promise(&data, arg(&args, 0));
            Ok(p)
        }));
        define(o, "all", it.native("all", 1, |it, _, args| {
            let list = it.iterate(&arg(&args, 0))?;
            let (result, result_data) = it.new_promise();
            let n = list.len();
            if n == 0 {
                it.resolve_promise(&result_data, it.array_from(Vec::new()));
                return Ok(result);
            }
            let state = Rc::new(RefCell::new((vec![Value::Undefined; n], 0usize)));
            for (i, v) in list.into_iter().enumerate() {
                let entry = it.native("", 1, {
                    let state = state.clone();
                    let result_data = result_data.clone();
                    move |it, _, args| {
                        let value = args.first().cloned().unwrap_or(Value::Undefined);
                        let done = {
                            let mut s = state.borrow_mut();
                            s.0[i] = value;
                            s.1 += 1;
                            s.1 == s.0.len()
                        };
                        if done {
                            let values = state.borrow().0.clone();
                            it.resolve_promise(&result_data, it.array_from(values));
                        }
                        Ok(Value::Undefined)
                    }
                });
                let rejected = it.native("", 1, {
                    let result_data = result_data.clone();
                    move |it, _, args| {
                        it.reject_promise(&result_data, args.first().cloned().unwrap_or(Value::Undefined));
                        Ok(Value::Undefined)
                    }
                });
                let p = if matches!(v.as_object().map(|o| matches!(o.borrow().kind, ObjKind::Promise(_))), Some(true)) {
                    v
                } else {
                    let (p, d) = it.new_promise();
                    it.resolve_promise(&d, v);
                    p
                };
                if let Ok(data) = promise_data(&p) {
                    it.promise_then(&data, entry, rejected);
                }
            }
            Ok(result)
        }));
        define(o, "allSettled", it.native("allSettled", 1, |it, _, args| {
            let list = it.iterate(&arg(&args, 0))?;
            let (result, result_data) = it.new_promise();
            let n = list.len();
            if n == 0 {
                it.resolve_promise(&result_data, it.array_from(Vec::new()));
                return Ok(result);
            }
            let state = Rc::new(RefCell::new((vec![Value::Undefined; n], 0usize)));
            for (i, v) in list.into_iter().enumerate() {
                let make_handler = |status: &'static str| -> Value {
                    let state = state.clone();
                    let result_data = result_data.clone();
                    it.native("", 1, move |it, _, args| {
                        let value = args.first().cloned().unwrap_or(Value::Undefined);
                        let entry = it.object();
                        if let Some(eo) = entry.as_object() {
                            define(eo, "status", Value::string(status));
                            if status == "fulfilled" {
                                define(eo, "value", value);
                            } else {
                                define(eo, "reason", value);
                            }
                        }
                        let done = {
                            let mut s = state.borrow_mut();
                            s.0[i] = entry;
                            s.1 += 1;
                            s.1 == s.0.len()
                        };
                        if done {
                            let values = state.borrow().0.clone();
                            it.resolve_promise(&result_data, it.array_from(values));
                        }
                        Ok(Value::Undefined)
                    })
                };
                let fulfilled = make_handler("fulfilled");
                let rejected = make_handler("rejected");
                let p = if matches!(v.as_object().map(|o| matches!(o.borrow().kind, ObjKind::Promise(_))), Some(true)) {
                    v
                } else {
                    let (p, d) = it.new_promise();
                    it.resolve_promise(&d, v);
                    p
                };
                if let Ok(data) = promise_data(&p) {
                    it.promise_then(&data, fulfilled, rejected);
                }
            }
            Ok(result)
        }));
        define(o, "any", it.native("any", 1, |it, _, args| {
            let list = it.iterate(&arg(&args, 0))?;
            let (result, result_data) = it.new_promise();
            let n = list.len();
            if n == 0 {
                let err = it.make_error("AggregateError", "All promises were rejected");
                it.reject_promise(&result_data, err);
                return Ok(result);
            }
            let errors: Rc<RefCell<(Vec<Value>, usize)>> = Rc::new(RefCell::new((Vec::new(), n)));
            for v in list {
                let errors = errors.clone();
                let ok_data = result_data.clone();
                let ok = it.native("", 1, move |it, _, args| {
                    it.resolve_promise(&ok_data, args.first().cloned().unwrap_or(Value::Undefined));
                    Ok(Value::Undefined)
                });
                let fail = it.native("", 1, {
                    let errors = errors.clone();
                    let result_data = result_data.clone();
                    move |it, _, args| {
                        let all_failed = {
                            let mut e = errors.borrow_mut();
                            e.0.push(args.first().cloned().unwrap_or(Value::Undefined));
                            e.1 = e.1.saturating_sub(1);
                            e.1 == 0
                        };
                        if all_failed {
                            let err = it.make_error("AggregateError", "All promises were rejected");
                            it.reject_promise(&result_data, err);
                        }
                        Ok(Value::Undefined)
                    }
                });
                let p = if matches!(v.as_object().map(|o| matches!(o.borrow().kind, ObjKind::Promise(_))), Some(true)) {
                    v
                } else {
                    let (p, d) = it.new_promise();
                    it.resolve_promise(&d, v);
                    p
                };
                if let Ok(data) = promise_data(&p) {
                    it.promise_then(&data, ok, fail);
                }
            }
            Ok(result)
        }));
        define(o, "race", it.native("race", 1, |it, _, args| {
            let list = it.iterate(&arg(&args, 0))?;
            let (result, result_data) = it.new_promise();
            for v in list {
                let resolve = it.native("", 1, {
                    let d = result_data.clone();
                    move |it, _, args| {
                        it.resolve_promise(&d, args.first().cloned().unwrap_or(Value::Undefined));
                        Ok(Value::Undefined)
                    }
                });
                let reject = it.native("", 1, {
                    let d = result_data.clone();
                    move |it, _, args| {
                        it.reject_promise(&d, args.first().cloned().unwrap_or(Value::Undefined));
                        Ok(Value::Undefined)
                    }
                });
                let p = if matches!(v.as_object().map(|o| matches!(o.borrow().kind, ObjKind::Promise(_))), Some(true)) {
                    v
                } else {
                    let (p, d) = it.new_promise();
                    it.resolve_promise(&d, v);
                    p
                };
                if let Ok(data) = promise_data(&p) {
                    it.promise_then(&data, resolve, reject);
                }
            }
            Ok(result)
        }));
    }
    it.set_global("Promise", promise_ctor);

    // ------------------------------------------------------------------
    // Globals, timers, fetch, encoding
    // ------------------------------------------------------------------
    it.set_global("Object", object_ctor);
    // Reflect is part of the standard object-model surface. Libraries use it
    // heavily for proxy traps and receiver-preserving property access.
    let reflect = it.object();
    if let Some(r) = reflect.as_object().cloned() {
        define(&r, "get", it.native("get", 2, |it, _, args| {
            let target = arg(&args, 0);
            let key = it.to_property_key(&arg(&args, 1))?;
            let receiver = args.get(2).cloned().unwrap_or_else(|| target.clone());
            match target.as_object() {
                Some(obj) => it.get_from_object(obj, &key, &receiver),
                None => it.get_member(target, &key),
            }
        }));
        define(&r, "set", it.native("set", 3, |it, _, args| {
            let target = arg(&args, 0);
            let key = it.to_property_key(&arg(&args, 1))?;
            let value = arg(&args, 2);
            if target.as_object().is_some() {
                it.set_member(target, &key, value)?;
                return Ok(Value::Bool(true));
            }
            Ok(Value::Bool(false))
        }));
        define(&r, "has", it.native("has", 2, |it, _, args| {
            let target = arg(&args, 0);
            let key = it.to_property_key(&arg(&args, 1))?;
            Ok(Value::Bool(target.as_object().map(|o| has_property(&o, &key)).unwrap_or(false)))
        }));
        define(&r, "deleteProperty", it.native("deleteProperty", 2, |it, _, args| {
            let target = arg(&args, 0);
            let key = it.to_property_key(&arg(&args, 1))?;
            Ok(Value::Bool(match target.as_object() {
                Some(o) => delete_prop(&o, &key),
                None => false,
            }))
        }));
        define(&r, "ownKeys", it.native("ownKeys", 1, |it, _, args| {
            let keys = arg(&args, 0)
                .as_object()
                .map(|o| own_keys(&o).into_iter().map(Value::string).collect())
                .unwrap_or_default();
            Ok(it.array_from(keys))
        }));
        define(&r, "getPrototypeOf", it.native("getPrototypeOf", 1, |_, _, args| {
            Ok(arg(&args, 0)
                .as_object()
                .and_then(|o| o.borrow().proto.clone())
                .map(Value::Object)
                .unwrap_or(Value::Null))
        }));
        define(&r, "setPrototypeOf", it.native("setPrototypeOf", 2, |_, _, args| {
            let target = arg(&args, 0);
            let proto = arg(&args, 1).as_object().cloned();
            if let Some(o) = target.as_object() {
                o.borrow_mut().proto = proto;
                Ok(Value::Bool(true))
            } else {
                Ok(Value::Bool(false))
            }
        }));
        define(&r, "apply", it.native("apply", 3, |it, _, args| {
            let f = arg(&args, 0);
            let this = arg(&args, 1);
            let list = args.get(2).map(|v| it.iterate(v)).transpose()?.unwrap_or_default();
            it.call_value(&f, this, list)
        }));
        define(&r, "construct", it.native("construct", 2, |it, _, args| {
            let list = args.get(1).map(|v| it.iterate(v)).transpose()?.unwrap_or_default();
            let new_target = args.get(2).filter(|v| v.is_callable()).cloned();
            it.construct_with_new_target(&arg(&args, 0), list, new_target)
        }));
        define(&r, "defineProperty", it.native("defineProperty", 3, |it, _, args| {
            let target = arg(&args, 0);
            let key = it.to_property_key(&arg(&args, 1))?;
            match target.as_object() {
                Some(o) => {
                    apply_descriptor(it, &o, &key, &arg(&args, 2))?;
                    Ok(Value::Bool(true))
                }
                None => Ok(Value::Bool(false)),
            }
        }));
        define(&r, "getOwnPropertyDescriptor", it.native("getOwnPropertyDescriptor", 2, |it, _, args| {
            let key = it.to_property_key(&arg(&args, 1))?;
            let prop = match arg(&args, 0).as_object().and_then(|o| own_prop(&o, &key)) {
                Some(p) => p,
                None => return Ok(Value::Undefined),
            };
            let d = it.object();
            if let Some(o) = d.as_object() {
                if let Some(get) = prop.get.clone() { define(o, "get", get); }
                if let Some(set) = prop.set.clone() { define(o, "set", set); }
                if prop.get.is_none() {
                    define(o, "value", prop.value.clone());
                    define(o, "writable", Value::Bool(prop.writable));
                }
                define(o, "enumerable", Value::Bool(prop.enumerable));
                define(o, "configurable", Value::Bool(prop.configurable));
            }
            Ok(d)
        }));
        define(&r, "isExtensible", it.native("isExtensible", 1, |_, _, _| Ok(Value::Bool(true))));
        define(&r, "preventExtensions", it.native("preventExtensions", 1, |_, _, _| Ok(Value::Bool(true))));
    }
    it.set_global("Reflect", reflect);
    it.set_global("Function", function_ctor);
    it.set_global("Array", array_ctor);
    it.set_global("String", string_ctor);
    it.set_global("Number", number_ctor);
    it.set_global("BigInt", it.native("BigInt", 1, |_, _, args| {
        let v = arg(&args, 0);
        match &v {
            Value::BigInt(n) => Ok(Value::BigInt(*n)),
            Value::Bool(b) => Ok(Value::BigInt(if *b { 1 } else { 0 })),
            Value::Number(n) => {
                if !n.is_finite() || n.fract() != 0.0 {
                    return Err(JsError::range_error(
                        "The number cannot be converted to a BigInt because it is not an integer",
                    ));
                }
                if *n < i128::MIN as f64 || *n > i128::MAX as f64 {
                    return Err(JsError::range_error("BigInt out of range"));
                }
                Ok(Value::BigInt(*n as i128))
            }
            Value::String(text) => {
                let t = text.trim();
                let (negative, body) = match t.strip_prefix('-') {
                    Some(rest) => (true, rest),
                    None => (false, t.strip_prefix('+').unwrap_or(t)),
                };
                let (radix, digits) = if let Some(d) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
                    (16, d)
                } else if let Some(d) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
                    (8, d)
                } else if let Some(d) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
                    (2, d)
                } else {
                    (10, body)
                };
                let parsed = if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
                    None
                } else {
                    i128::from_str_radix(digits, radix).ok()
                };
                match parsed {
                    Some(n) => Ok(Value::BigInt(if negative { -n } else { n })),
                    None => Err(JsError::syntax_error(
                        "Cannot convert the string to a BigInt",
                    )),
                }
            }
            _ => Err(JsError::type_error("Cannot convert value to a BigInt")),
        }
    }));
    it.set_global("Boolean", boolean_ctor);
    it.set_global("globalThis", Value::Object(it.global.clone()));
    it.set_global("undefined", Value::Undefined);
    it.set_global("NaN", Value::Number(f64::NAN));
    it.set_global("Infinity", Value::Number(f64::INFINITY));
    it.set_global("parseInt", it.native("parseInt", 2, |it, _, args| {
        Ok(Value::Number(parse_int_js(&it.to_string(&arg(&args, 0))?, arg(&args, 1).to_number() as u32)))
    }));
    it.set_global("parseFloat", it.native("parseFloat", 1, |it, _, args| {
        Ok(Value::Number(parse_float_js(&it.to_string(&arg(&args, 0))?)))
    }));
    it.set_global("isNaN", it.native("isNaN", 1, |it, _, args| {
        Ok(Value::Bool(it.to_number_value(&arg(&args, 0))?.is_nan()))
    }));
    it.set_global("isFinite", it.native("isFinite", 1, |it, _, args| {
        Ok(Value::Bool(it.to_number_value(&arg(&args, 0))?.is_finite()))
    }));
    it.set_global("setTimeout", it.native("setTimeout", 2, |it, _, args| {
        let cb = arg(&args, 0);
        let delay = arg(&args, 1).to_number();
        Ok(Value::Number(it.add_timer(cb, delay, false, args.iter().skip(2).cloned().collect())))
    }));
    it.set_global("setInterval", it.native("setInterval", 2, |it, _, args| {
        let cb = arg(&args, 0);
        let delay = arg(&args, 1).to_number();
        Ok(Value::Number(it.add_timer(cb, delay, true, args.iter().skip(2).cloned().collect())))
    }));
    it.set_global("clearTimeout", it.native("clearTimeout", 1, |it, _, args| {
        it.clear_timer(arg(&args, 0).to_number());
        Ok(Value::Undefined)
    }));
    it.set_global("clearInterval", it.native("clearInterval", 1, |it, _, args| {
        it.clear_timer(arg(&args, 0).to_number());
        Ok(Value::Undefined)
    }));
    it.set_global("structuredClone", it.native("structuredClone", 1, |it, _, args| {
        clone_structured(it, &arg(&args, 0), 0)
    }));
    it.set_global("queueMicrotask", it.native("queueMicrotask", 1, |it, _, args| {
        let cb = arg(&args, 0);
        it.microtasks.push_back(Box::new(move |it: &mut Interp| {
            it.call_value(&cb, Value::Undefined, vec![])?;
            Ok(())
        }));
        Ok(Value::Undefined)
    }));
    it.set_global("fetch", it.native("fetch", 1, |it, _, args| {
        let url = it.to_string(&arg(&args, 0))?;
        let mut method = "GET".to_string();
        let mut headers = Vec::new();
        let mut body = None;
        if let Some(opts) = args.get(1).and_then(|v| v.as_object()) {
            let opts_value = Value::Object(opts.clone());
            if let Ok(m) = it.get_member(opts_value.clone(), "method") {
                if !m.is_undefined() {
                    method = it.to_string(&m)?.to_uppercase();
                }
            }
            if let Ok(b) = it.get_member(opts_value.clone(), "body") {
                if !b.is_undefined() {
                    body = Some(it.to_string(&b)?);
                }
            }
            if let Ok(h) = it.get_member(opts_value, "headers") {
                if let Some(ho) = h.as_object() {
                    for (k, v) in own_key_values(ho) {
                        headers.push((k, it.to_string(&v)?));
                    }
                }
            }
        }
        match it.host.fetch(&method, &url, &headers, body.as_deref()) {
            Ok(resp) => {
                let response = make_response(it, resp);
                let (p, d) = it.new_promise();
                it.resolve_promise(&d, response);
                Ok(p)
            }
            Err(msg) => {
                let (p, d) = it.new_promise();
                let err = it.make_error("TypeError", &format!("fetch failed: {msg}"));
                it.reject_promise(&d, err);
                Ok(p)
            }
        }
    }));
    it.set_global("encodeURIComponent", it.native("encodeURIComponent", 1, |it, _, args| {
        Ok(Value::string(percent_encode(&it.to_string(&arg(&args, 0))?, false)))
    }));
    it.set_global("decodeURIComponent", it.native("decodeURIComponent", 1, |it, _, args| {
        Ok(Value::string(percent_decode(&it.to_string(&arg(&args, 0))?)))
    }));
    it.set_global("encodeURI", it.native("encodeURI", 1, |it, _, args| {
        Ok(Value::string(percent_encode(&it.to_string(&arg(&args, 0))?, true)))
    }));
    it.set_global("decodeURI", it.native("decodeURI", 1, |it, _, args| {
        Ok(Value::string(percent_decode(&it.to_string(&arg(&args, 0))?)))
    }));
    it.set_global("btoa", it.native("btoa", 1, |it, _, args| {
        Ok(Value::string(base64_encode(it.to_string(&arg(&args, 0))?.as_bytes())))
    }));
    it.set_global("atob", it.native("atob", 1, |it, _, args| {
        let s = it.to_string(&arg(&args, 0))?;
        match base64_decode(&s) {
            Some(bytes) => Ok(Value::string(String::from_utf8_lossy(&bytes).to_string())),
            None => Err(JsError::type_error("invalid base64")),
        }
    }));

    // ------------------------------------------------------------------
    // URL / URLSearchParams
    // ------------------------------------------------------------------
    let url_ctor = it.native("URL", 1, |it, this, args| {
        let input = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
        let parsed = if let Some(base) = args.get(1).filter(|v| !v.is_undefined() && !v.is_null()) {
            let base = it.to_string(base)?;
            veyra_common::Url::parse(&base).map(|b| b.resolve(&input))
        } else {
            veyra_common::Url::parse(&input)
        };
        let url = parsed.ok_or_else(|| JsError::type_error("Invalid URL"))?;
        let obj = match this.as_object() {
            Some(o) if it.constructing => Value::Object(o.clone()),
            _ => it.object(),
        };
        populate_url(it, &obj, &url);
        Ok(obj)
    });
    if let Some(o) = url_ctor.as_object() {
        define(o, "canParse", it.native("canParse", 1, |it, _, args| {
            let input = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
            let parsed = if let Some(base) = args.get(1).filter(|v| !v.is_undefined()) {
                let base = it.to_string(base)?;
                veyra_common::Url::parse(&base).map(|b| b.resolve(&input))
            } else {
                veyra_common::Url::parse(&input)
            };
            Ok(Value::Bool(parsed.is_some()))
        }));
    }
    it.set_global("URL", url_ctor);

    let params_ctor = it.native("URLSearchParams", 0, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => Value::Object(o.clone()),
            _ => it.object(),
        };
        let entries: Vec<(String, String)> = match args.first() {
            Some(Value::String(s)) => parse_query(s),
            Some(Value::Object(o)) => {
                // A URLSearchParams instance keeps its own entries.
                if let Ok(existing) = it.get_member(Value::Object(o.clone()), "__entries") {
                    if !existing.is_undefined() {
                        params_entries(it, &existing)
                    } else {
                        own_key_values(o)
                            .into_iter()
                            .map(|(k, v)| Ok((k, it.to_string(&v)?)))
                            .collect::<Result<Vec<_>, JsError>>()?
                    }
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };
        set_params_entries(it, &obj, &entries);
        Ok(obj)
    });
    if let Some(o) = params_ctor.as_object().cloned() {
        let proto = it.object();
        if let Some(p) = proto.as_object().cloned() {
            define_params_api(it, &p);
        }
        define(&o, "prototype", proto);
    }
    it.set_global("URLSearchParams", params_ctor);

    // ------------------------------------------------------------------
    // Intl
    // ------------------------------------------------------------------
    let intl = it.object();
    if let Some(i) = intl.as_object().cloned() {
        let nf = it.native("NumberFormat", 0, |it, this, args| {
            let obj = match this.as_object() {
                Some(o) if it.constructing => Value::Object(o.clone()),
                _ => it.object(),
            };
            let mut min_frac = 0.0;
            let mut max_frac = 3.0;
            let mut grouping = true;
            let mut style = "decimal".to_string();
            let mut currency = "USD".to_string();
            if let Some(opts) = args.get(1).and_then(|v| v.as_object()) {
                let ov = Value::Object(opts.clone());
                if let Ok(v) = it.get_member(ov.clone(), "minimumFractionDigits") {
                    if !v.is_undefined() { min_frac = v.to_number(); }
                }
                if let Ok(v) = it.get_member(ov.clone(), "maximumFractionDigits") {
                    if !v.is_undefined() { max_frac = v.to_number(); }
                }
                if let Ok(v) = it.get_member(ov.clone(), "useGrouping") {
                    grouping = v.truthy();
                }
                if let Ok(v) = it.get_member(ov.clone(), "style") {
                    if let Some(s) = v.as_str() { style = s.to_string(); }
                }
                if let Ok(v) = it.get_member(ov, "currency") {
                    if let Some(s) = v.as_str() { currency = s.to_string(); }
                }
            }
            if let Some(o) = obj.as_object() {
                define(o, "__minFrac", Value::Number(min_frac));
                define(o, "__maxFrac", Value::Number(max_frac));
                define(o, "__group", Value::Bool(grouping));
                define(o, "__style", Value::string(style));
                define(o, "__currency", Value::string(currency));
            }
            Ok(obj)
        });
        if let Some(nfo) = nf.as_object().cloned() {
            let nf_proto = it.object();
            if let Some(np) = nf_proto.as_object().cloned() {
            define(&np, "format", it.native("format", 1, |it, this, args| {
                let n = it.to_number_value(&arg(&args, 0))?;
                let min = it.get_member(this.clone(), "__minFrac").ok().map(|v| v.to_number()).unwrap_or(0.0) as usize;
                let max = it.get_member(this.clone(), "__maxFrac").ok().map(|v| v.to_number()).unwrap_or(3.0) as usize;
                let group = it.get_member(this.clone(), "__group").ok().map(|v| v.truthy()).unwrap_or(true);
                let style = it.get_member(this.clone(), "__style").ok().and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_else(|| "decimal".into());
                let currency = it.get_member(this.clone(), "__currency").ok().and_then(|v| v.as_str().map(|s| s.to_string())).unwrap_or_else(|| "USD".into());
                let mut text = format_grouped(n, min, max, group);
                if style == "percent" {
                    text = format_grouped(n * 100.0, min, max, group);
                    text.push('%');
                } else if style == "currency" {
                    let symbol = if currency.eq_ignore_ascii_case("USD") { "$" } else if currency.eq_ignore_ascii_case("EUR") { "€" } else if currency.eq_ignore_ascii_case("GBP") { "£" } else { "" };
                    text = format!("{symbol}{text}");
                }
                Ok(Value::string(text))
            }));
            define(&np, "resolvedOptions", it.native("resolvedOptions", 0, |it, this, _| {
                let out = it.object();
                if let Some(o) = out.as_object() {
                    define(o, "minimumFractionDigits", it.get_member(this.clone(), "__minFrac").unwrap_or(Value::Number(0.0)));
                    define(o, "maximumFractionDigits", it.get_member(this.clone(), "__maxFrac").unwrap_or(Value::Number(3.0)));
                }
                Ok(out)
            }));
            }
            define(&nfo, "supportedLocalesOf", it.native("supportedLocalesOf", 1, |it, _, _| Ok(it.array_from(vec![]))));
            define(&nfo, "prototype", nf_proto);
        }
        define(&i, "NumberFormat", nf);

        let dtf = it.native("DateTimeFormat", 0, |it, this, _| {
            let obj = match this.as_object() {
                Some(o) if it.constructing => Value::Object(o.clone()),
                _ => it.object(),
            };
            Ok(obj)
        });
        if let Some(df) = dtf.as_object().cloned() {
            let proto = it.object();
            if let Some(p) = proto.as_object().cloned() {
                define(&p, "format", it.native("format", 1, |_, _, args| {
                    let ms = arg(&args, 0).to_number();
                    let (y, m, d, ..) = date_parts(ms);
                    Ok(Value::string(format!("{}/{}/{}", m + 1, d, y)))
                }));
                define(&p, "resolvedOptions", it.native("resolvedOptions", 0, |it, _, _| Ok(it.object())));
            }
            define(&df, "prototype", proto);
        }
        define(&i, "DateTimeFormat", dtf);
        let collator = it.native("Collator", 0, |it, this, _| {
            Ok(match this.as_object() {
                Some(o) if it.constructing => Value::Object(o.clone()),
                _ => it.object(),
            })
        });
        if let Some(co) = collator.as_object().cloned() {
            let proto = it.object();
            if let Some(p) = proto.as_object().cloned() {
                define(&p, "compare", it.native("compare", 2, |it, _, args| {
                    let a = it.to_string(&arg(&args, 0))?;
                    let b = it.to_string(&arg(&args, 1))?;
                    Ok(Value::Number(match a.cmp(&b) { std::cmp::Ordering::Less => -1.0, std::cmp::Ordering::Equal => 0.0, std::cmp::Ordering::Greater => 1.0 }))
                }));
            }
            define(&co, "prototype", proto);
        }
        define(&i, "Collator", collator);
        define(&i, "getCanonicalLocales", it.native("getCanonicalLocales", 1, |it, _, args| {
            let v = arg(&args, 0);
            let list = it.iterate(&v).unwrap_or_default();
            Ok(it.array_from(list))
        }));
    }
    it.set_global("Intl", intl);

    // ------------------------------------------------------------------
    // crypto (random source lives in the host)
    // ------------------------------------------------------------------
    let crypto = it.object();
    if let Some(c) = crypto.as_object().cloned() {
        define(&c, "getRandomValues", it.native("getRandomValues", 1, |it, _, args| {
            let target = arg(&args, 0);
            let len = match &target {
                Value::Object(o) => match &o.borrow().kind {
                    ObjKind::Array(v) => v.len(),
                    _ => 0,
                },
                _ => 0,
            };
            for i in 0..len {
                let n = (it.random() * 256.0).floor();
                it.set_member(target.clone(), &i.to_string(), Value::Number(n))?;
            }
            Ok(target)
        }));
        define(&c, "randomUUID", it.native("randomUUID", 0, |it, _, _| {
            let mut bytes = [0u8; 16];
            for b in bytes.iter_mut() {
                *b = (it.random() * 256.0).floor() as u8;
            }
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            Ok(Value::string(format!(
                "{}-{}-{}-{}-{}",
                &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32]
            )))
        }));
        define(&c, "subtle", it.object());
    }
    it.set_global("crypto", crypto);

    // ------------------------------------------------------------------
    // Fetch API constructors
    // ------------------------------------------------------------------
    let headers_ctor = it.native("Headers", 1, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => Value::Object(o.clone()),
            _ => it.object(),
        };
        let entries = header_init_pairs(it, args.first().cloned())?;
        set_params_entries(it, &obj, &entries);
        Ok(obj)
    });
    if let Some(o) = headers_ctor.as_object().cloned() {
        define_headers_api(it, &o);
    }
    it.set_global("Headers", headers_ctor.clone());

    let response_ctor = it.native("Response", 1, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => Value::Object(o.clone()),
            _ => it.object(),
        };
        let body = match args.first() {
            Some(v) if !v.is_nullish() => it.to_string(v)?,
            _ => String::new(),
        };
        let mut status = 200.0;
        let mut status_text = String::new();
        let mut headers = make_headers(it, None);
        if let Some(init) = args.get(1).and_then(|v| v.as_object()) {
            let iv = Value::Object(init.clone());
            if let Ok(v) = it.get_member(iv.clone(), "status") {
                if !v.is_undefined() { status = v.to_number(); }
            }
            if let Ok(v) = it.get_member(iv.clone(), "statusText") {
                status_text = it.to_string(&v)?;
            }
            if let Ok(v) = it.get_member(iv.clone(), "headers") {
                headers = make_headers(it, Some(v));
            }
        }
        if let Some(o) = obj.as_object() {
            define(o, "status", Value::Number(status));
            define(o, "statusText", Value::string(status_text));
            define(o, "ok", Value::Bool(status >= 200.0 && status < 300.0));
            define(o, "url", Value::string(""));
            define(o, "redirected", Value::Bool(false));
            define(o, "type", Value::string("default"));
            define(o, "bodyUsed", Value::Bool(false));
            define(o, "headers", headers);
            define(o, "__body", Value::string(body.clone()));
            define(o, "text", it.native("text", 0, {
                let body = body.clone();
                move |it, _, _| {
                    let (p, d) = it.new_promise();
                    it.resolve_promise(&d, Value::string(body.clone()));
                    Ok(p)
                }
            }));
            define(o, "json", it.native("json", 0, {
                let body = body.clone();
                move |it, _, _| {
                    let (p, d) = it.new_promise();
                    match it.json_parse(&body) {
                        Ok(v) => it.resolve_promise(&d, v),
                        Err(e) => { let ev = it.error_value(&e); it.reject_promise(&d, ev); }
                    }
                    Ok(p)
                }
            }));
            define(o, "clone", it.native("clone", 0, |_, this, _| Ok(this)));
        }
        Ok(obj)
    });
    it.set_global("Response", response_ctor);

    let request_ctor = it.native("Request", 1, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => Value::Object(o.clone()),
            _ => it.object(),
        };
        let url = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
        let mut method = "GET".to_string();
        let mut headers = make_headers(it, None);
        let mut body = String::new();
        if let Some(init) = args.get(1).and_then(|v| v.as_object()) {
            let iv = Value::Object(init.clone());
            if let Ok(v) = it.get_member(iv.clone(), "method") {
                if !v.is_undefined() { method = it.to_string(&v)?.to_uppercase(); }
            }
            if let Ok(v) = it.get_member(iv.clone(), "headers") {
                headers = make_headers(it, Some(v));
            }
            if let Ok(v) = it.get_member(iv.clone(), "body") {
                if !v.is_nullish() { body = it.to_string(&v)?; }
            }
        }
        if let Some(o) = obj.as_object() {
            define(o, "url", Value::string(url));
            define(o, "method", Value::string(method));
            define(o, "headers", headers);
            define(o, "__body", Value::string(body.clone()));
            define(o, "text", it.native("text", 0, {
                let body = body.clone();
                move |it, _, _| {
                    let (p, d) = it.new_promise();
                    it.resolve_promise(&d, Value::string(body.clone()));
                    Ok(p)
                }
            }));
        }
        Ok(obj)
    });
    it.set_global("Request", request_ctor);

    // ------------------------------------------------------------------
    // Small platform stubs: feature detection must not crash, and the
    // object shapes below match what libraries touch during startup.
    // ------------------------------------------------------------------
    let wasm = it.object();
    if let Some(w) = wasm.as_object().cloned() {
        let reject = |it: &mut Interp, _: Value, _: Vec<Value>| {
            let (p, d) = it.new_promise();
            let e = it.make_error("Error", "WebAssembly is not supported by Veyra");
            it.reject_promise(&d, e);
            Ok(p)
        };
        define(&w, "instantiate", it.native("instantiate", 1, reject));
        define(&w, "compile", it.native("compile", 1, reject));
        define(&w, "validate", it.native("validate", 1, |_, _, _| Ok(Value::Bool(false))));
        define(&w, "Module", it.native("Module", 1, |it, _, _| Ok(it.object())));
        define(&w, "Instance", it.native("Instance", 1, |it, _, _| Ok(it.object())));
        define(&w, "Memory", it.native("Memory", 1, |it, _, _| Ok(it.object())));
    }
    it.set_global("WebAssembly", wasm);

    let audio_ctor = it.native("Audio", 1, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => Value::Object(o.clone()),
            _ => it.object(),
        };
        let src = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
        if let Some(o) = obj.as_object() {
            define(o, "src", Value::string(src));
            define(o, "currentTime", Value::Number(0.0));
            define(o, "duration", Value::Number(0.0));
            define(o, "volume", Value::Number(1.0));
            define(o, "paused", Value::Bool(true));
            define(o, "ended", Value::Bool(false));
            define(o, "loop", Value::Bool(false));
            define(o, "muted", Value::Bool(false));
            define(o, "play", it.native("play", 0, |it, _, _| {
                let (p, d) = it.new_promise();
                it.resolve_promise(&d, Value::Undefined);
                Ok(p)
            }));
            define(o, "pause", it.native("pause", 0, |_, _, _| Ok(Value::Undefined)));
            define(o, "load", it.native("load", 0, |_, _, _| Ok(Value::Undefined)));
            define(o, "canPlayType", it.native("canPlayType", 1, |_, _, _| Ok(Value::string("maybe"))));
            define(o, "addEventListener", it.native("addEventListener", 2, |_, _, _| Ok(Value::Undefined)));
            define(o, "removeEventListener", it.native("removeEventListener", 2, |_, _, _| Ok(Value::Undefined)));
        }
        Ok(obj)
    });
    it.set_global("Audio", audio_ctor);

    let worker_ctor = it.native("Worker", 1, |it, this, args| {
        let obj = match this.as_object() {
            Some(o) if it.constructing => Value::Object(o.clone()),
            _ => it.object(),
        };
        let url = args.first().map(|v| it.to_string(v)).transpose()?.unwrap_or_default();
        if let Some(o) = obj.as_object() {
            define(o, "__workerUrl", Value::string(url));
            define(o, "onmessage", Value::Null);
            define(o, "onerror", Value::Null);
            define(o, "postMessage", it.native("postMessage", 1, |_, _, _| Ok(Value::Undefined)));
            define(o, "terminate", it.native("terminate", 0, |_, _, _| Ok(Value::Undefined)));
            define(o, "addEventListener", it.native("addEventListener", 2, |_, _, _| Ok(Value::Undefined)));
            define(o, "removeEventListener", it.native("removeEventListener", 2, |_, _, _| Ok(Value::Undefined)));
        }
        Ok(obj)
    });
    it.set_global("Worker", worker_ctor);

    let message_port_ctor = it.native("MessagePort", 0, |it, _, _| Ok(it.object()));
    it.set_global("MessagePort", message_port_ctor);
    it.set_global("MessageChannel", it.native("MessageChannel", 0, |it, _, _| Ok(make_message_channel(it))));

    it.set_global("requestIdleCallback", it.native("requestIdleCallback", 1, |it, _, args| {
        let cb = arg(&args, 0);
        let id = it.add_timer(cb, 1.0, false, vec![Value::Undefined]);
        Ok(Value::Number(id))
    }));
    it.set_global("cancelIdleCallback", it.native("cancelIdleCallback", 1, |it, _, args| {
        it.clear_timer(arg(&args, 0).to_number());
        Ok(Value::Undefined)
    }));
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

fn to_object(it: &mut Interp, value: &Value) -> Result<ObjRef, JsError> {
    match value {
        Value::Object(o) => Ok(o.clone()),
        Value::Undefined | Value::Null => {
            Err(JsError::type_error("cannot convert undefined or null to object"))
        }
        Value::String(s) => {
            let obj = new_object(Some(it.intrinsics.string_proto.clone()));
            obj.borrow_mut().kind = ObjKind::StringObj(s.to_string());
            Ok(obj)
        }
        Value::Number(n) => {
            let obj = new_object(Some(it.intrinsics.number_proto.clone()));
            obj.borrow_mut().kind = ObjKind::NumberObj(*n);
            Ok(obj)
        }
        Value::Bool(b) => {
            let obj = new_object(Some(it.intrinsics.boolean_proto.clone()));
            obj.borrow_mut().kind = ObjKind::BooleanObj(*b);
            Ok(obj)
        }
        Value::BigInt(_) => Ok(new_object(Some(it.intrinsics.object_proto.clone()))),
    }
}

fn apply_descriptor(it: &mut Interp, obj: &ObjRef, key: &str, desc: &Value) -> Result<(), JsError> {
    let dobj = desc
        .as_object()
        .ok_or_else(|| JsError::type_error("property descriptor must be an object"))?;
    let d = Value::Object(dobj.clone());
    let has = |name: &str| own_prop(dobj, name).is_some();
    let existing = own_prop(obj, key);

    if has("get") || has("set") {
        // Read descriptor slots directly. Walking the descriptor's prototype can
        // invoke an inherited accessor that is unrelated to the definition.
        let get = own_prop(dobj, "get").map(|p| p.value).unwrap_or(Value::Undefined);
        let set = own_prop(dobj, "set").map(|p| p.value).unwrap_or(Value::Undefined);
        let old_get = existing.as_ref().and_then(|p| p.get.clone());
        let old_set = existing.as_ref().and_then(|p| p.set.clone());
        define_accessor(
            obj,
            key,
            if get.is_callable() { Some(get) } else { old_get },
            if set.is_callable() { Some(set) } else { old_set },
            if has("enumerable") {
                it.get_member(d.clone(), "enumerable").map(|v| v.truthy()).unwrap_or(false)
            } else {
                existing.as_ref().map(|p| p.enumerable).unwrap_or(false)
            },
        );
        return Ok(());
    }

    let value = if has("value") {
        it.get_member(d.clone(), "value").unwrap_or(Value::Undefined)
    } else {
        existing.as_ref().map(|p| p.value.clone()).unwrap_or(Value::Undefined)
    };
    // When redefining an existing property, omitted attributes keep their
    // current values; new properties default to false.
    let writable = if has("writable") {
        it.get_member(d.clone(), "writable").map(|v| v.truthy()).unwrap_or(false)
    } else {
        existing.as_ref().map(|p| p.writable).unwrap_or(false)
    };
    let enumerable = if has("enumerable") {
        it.get_member(d.clone(), "enumerable").map(|v| v.truthy()).unwrap_or(false)
    } else {
        existing.as_ref().map(|p| p.enumerable).unwrap_or(false)
    };
    let configurable = if has("configurable") {
        it.get_member(d, "configurable").map(|v| v.truthy()).unwrap_or(false)
    } else {
        existing.as_ref().map(|p| p.configurable).unwrap_or(false)
    };
    define_with(obj, key, value, writable, enumerable, configurable);
    Ok(())
}

fn tag(v: &Value) -> &'static str {
    match v {
        Value::Undefined => "Undefined",
        Value::Null => "Null",
        Value::Bool(_) => "Boolean",
        Value::Number(_) => "Number",
        Value::BigInt(_) => "BigInt",
        Value::String(_) => "String",
        Value::Object(o) => match &o.borrow().kind {
            ObjKind::Array(_) => "Array",
            ObjKind::Function(_) => "Function",
            ObjKind::Error { .. } => "Error",
            ObjKind::Date(_) => "Date",
            ObjKind::RegExp { .. } => "RegExp",
            ObjKind::Map(_) => "Map",
            ObjKind::Set(_) => "Set",
            ObjKind::Promise(_) => "Promise",
            ObjKind::NumberObj(_) => "Number",
            ObjKind::StringObj(_) => "String",
            ObjKind::BooleanObj(_) => "Boolean",
            _ => "Object",
        },
    }
}

fn require_string(this: &Value) -> Result<String, JsError> {
    match this {
        Value::String(s) => Ok(s.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Undefined => Ok("undefined".into()),
        Value::Null => Ok("null".into()),
        Value::BigInt(n) => Ok(n.to_string()),
        Value::Object(o) => match &o.borrow().kind {
            ObjKind::StringObj(s) => Ok(s.clone()),
            _ => Ok(this.to_string_primitive()),
        },
    }
}

fn require_number(this: &Value) -> Result<f64, JsError> {
    match this {
        Value::Number(n) => Ok(*n),
        Value::Object(o) => match &o.borrow().kind {
            ObjKind::NumberObj(n) => Ok(*n),
            _ => Err(JsError::type_error("Number.prototype method called on incompatible receiver")),
        },
        _ => Err(JsError::type_error("Number.prototype method called on incompatible receiver")),
    }
}

fn require_boolean(this: &Value) -> Result<bool, JsError> {
    match this {
        Value::Bool(b) => Ok(*b),
        Value::Object(o) => match &o.borrow().kind {
            ObjKind::BooleanObj(b) => Ok(*b),
            _ => Err(JsError::type_error("Boolean.prototype method called on incompatible receiver")),
        },
        _ => Err(JsError::type_error("Boolean.prototype method called on incompatible receiver")),
    }
}

fn require_date(this: &Value) -> Result<f64, JsError> {
    let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Date"))?;
    match &obj.borrow().kind {
        ObjKind::Date(ms) => Ok(*ms),
        _ => Err(JsError::type_error("not a Date")),
    }
}

fn promise_data(this: &Value) -> Result<Rc<RefCell<PromiseData>>, JsError> {
    let obj = this.as_object().ok_or_else(|| JsError::type_error("not a Promise"))?;
    match &obj.borrow().kind {
        ObjKind::Promise(d) => Ok(d.clone()),
        _ => Err(JsError::type_error("not a Promise")),
    }
}

const MAX_ARRAY_LIKE: usize = 10_000_000;

fn array_like_values(this: &Value) -> Option<Vec<Value>> {
    match this {
        Value::String(s) => Some(s.chars().map(|c| Value::string(c.to_string())).collect()),
        Value::Object(o) => {
            let o = o.borrow();
            if let ObjKind::Array(v) = &o.kind {
                return Some(v.clone());
            }
            let len = o.props.get("length").map(|p| p.value.to_index());
            // Never materialize an array-like whose declared length is beyond
            // what we can safely allocate. Pages can expose bogus or enormous
            // `length` values; allocating them aborts the whole browser.
            let len = len.filter(|len| *len <= MAX_ARRAY_LIKE);
            len.map(|len| {
                (0..len)
                    .map(|i| o.props.get(&i.to_string()).map(|p| p.value.clone()).unwrap_or(Value::Undefined))
                    .collect()
            })
        }
        _ => None,
    }
}

fn array_holes(this: &Value) -> Option<Vec<bool>> {
    if let Value::Object(o) = this {
        return o.borrow().array_holes.clone();
    }
    None
}

fn store_array_values(this: &Value, values: &[Value]) {
    if let Some(obj) = this.as_object() {
        let mut o = obj.borrow_mut();
        if let ObjKind::Array(v) = &mut o.kind {
            v.clear();
            v.extend_from_slice(values);
            o.array_holes = None;
            return;
        }
        let old = o.props.get("length").map(|p| p.value.to_index()).unwrap_or(0);
        for i in old..values.len() {
            o.props.insert(i.to_string(), Prop::data(Value::Undefined, true));
        }
        for (i, v) in values.iter().enumerate() {
            o.props.insert(i.to_string(), Prop::data(v.clone(), true));
        }
        o.props.insert("length".into(), Prop::data(Value::Number(values.len() as f64), false));
    }
}

fn normalize_index(n: f64, len: i64) -> i64 {
    if n.is_nan() {
        return 0;
    }
    let n = n.trunc() as i64;
    if n < 0 {
        (len + n).max(0)
    } else {
        n.min(len)
    }
}

fn flatten(it: &Interp, values: Vec<Value>, depth: i32, out: &mut Vec<Value>) {
    for v in values {
        if depth > 0 && is_array(&v) {
            flatten(it, array_values(&v).unwrap_or_default(), depth - 1, out);
        } else {
            out.push(v);
        }
    }
}

fn match_values(s: &str, m: &regress::Match) -> Vec<Value> {
    let mut values = vec![Value::string(s[m.range.clone()].to_string())];
    for g in &m.captures {
        values.push(
            g.clone()
                .map(|r| Value::string(s[r].to_string()))
                .unwrap_or(Value::Undefined),
        );
    }
    values
}

fn match_regex_result(it: &mut Interp, re: &regress::Regex, s: &str) -> Value {
    match re.find(s) {
        Some(m) => {
            let arr = it.array_from(match_values(s, &m));
            if let Some(o) = arr.as_object() {
                define(o, "index", Value::Number(byte_to_char(s, m.start()) as f64));
                define(o, "input", Value::string(s.to_string()));
            }
            arr
        }
        None => Value::Null,
    }
}

fn regex_split(re: &regress::Regex, s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut last = 0usize;
    for m in re.find_iter(s) {
        let (start, end) = (m.start(), m.end());
        out.push(s[last..start].to_string());
        for g in &m.captures {
            out.push(g.clone().map(|r| s[r].to_string()).unwrap_or_default());
        }
        last = end;
        if start == end {
            if last >= s.len() {
                break;
            }
            let next = s[last..].chars().next().map(|c| last + c.len_utf8()).unwrap_or(last + 1);
            if next <= last {
                break;
            }
            last = next;
        }
    }
    out.push(s[last..].to_string());
    out
}

fn replace_one(it: &mut Interp, s: &str, needle: &str, replacement: &Value) -> Result<String, JsError> {
    match s.find(needle) {
        Some(i) => {
            let end = i + needle.len();
            let rep = if replacement.is_callable() {
                let r = it.call_value(replacement, Value::Undefined, vec![Value::string(needle.to_string())])?;
                it.to_string(&r)?
            } else {
                it.to_string(replacement)?
            };
            Ok(format!("{}{}{}", &s[..i], rep, &s[end..]))
        }
        None => Ok(s.to_string()),
    }
}

fn replace_regex(
    it: &mut Interp,
    s: &str,
    re: &regress::Regex,
    replacement: &Value,
    all: bool,
) -> Result<Value, JsError> {
    let mut out = String::new();
    let mut last = 0usize;
    for m in re.find_iter(s) {
        let (start, end) = (m.start(), m.end());
        out.push_str(&s[last..start]);
        let rep = if replacement.is_callable() {
            let mut args = match_values(s, &m);
            args.push(Value::Number(byte_to_char(s, start) as f64));
            args.push(Value::string(s.to_string()));
            let called = it.call_value(replacement, Value::Undefined, args)?;
            it.to_string(&called)?
        } else {
            let mut rep = it.to_string(replacement)?;
            for (i, g) in m.captures.iter().enumerate() {
                let text = g.clone().map(|r| s[r].to_string()).unwrap_or_default();
                rep = rep.replace(&format!("${}", i + 1), &text);
            }
            rep.replace("$&", &s[start..end])
        };
        out.push_str(&rep);
        last = end;
        if !all {
            break;
        }
        if start == end && last < s.len() {
            let step = s[last..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            out.push_str(&s[last..last + step]);
            last += step;
        }
    }
    out.push_str(&s[last..]);
    Ok(Value::string(out))
}

fn pad_string(s: &str, target: usize, pad: &str, start: bool) -> String {
    let len = s.chars().count();
    if len >= target || pad.is_empty() {
        return s.to_string();
    }
    let mut fill = String::new();
    while len + fill.chars().count() < target {
        fill.push_str(pad);
    }
    let fill: String = fill.chars().take(target - len).collect();
    if start {
        format!("{fill}{s}")
    } else {
        format!("{s}{fill}")
    }
}

fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(s.len())
}

fn byte_to_char(s: &str, byte_idx: usize) -> usize {
    s[..byte_idx.min(s.len())].chars().count()
}

fn number_radix(n: f64, radix: u32) -> String {
    if radix == 10 || !n.is_finite() || n.fract() != 0.0 || radix < 2 || radix > 36 {
        return number_to_string(n);
    }
    let negative = n < 0.0;
    let mut v = n.abs() as u64;
    if v == 0 {
        return "0".into();
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while v > 0 {
        out.push(digits[(v % radix as u64) as usize]);
        v /= radix as u64;
    }
    if negative {
        out.push(b'-');
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn parse_int_js(s: &str, radix: u32) -> f64 {
    let s = s.trim();
    let (neg, s) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (false, rest)
    } else {
        (false, s)
    };
    let mut radix = radix;
    let mut body = s;
    if radix == 0 || radix == 16 {
        if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            radix = 16;
            body = rest;
        }
    }
    if radix == 0 {
        radix = 10;
    }
    if !(2..=36).contains(&radix) {
        return f64::NAN;
    }
    let mut value = 0.0;
    let mut any = false;
    for c in body.chars() {
        let d = match c.to_digit(radix) {
            Some(d) => d,
            None => break,
        };
        value = value * radix as f64 + d as f64;
        any = true;
    }
    if !any {
        return f64::NAN;
    }
    if neg {
        -value
    } else {
        value
    }
}

fn parse_float_js(s: &str) -> f64 {
    let t = s.trim_start();
    let mut end = 0usize;
    let bytes = t.as_bytes();
    let mut seen_dot = false;
    let mut seen_e = false;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    while end < bytes.len() {
        let c = bytes[end];
        if c.is_ascii_digit() {
            end += 1;
        } else if c == b'.' && !seen_dot && !seen_e {
            seen_dot = true;
            end += 1;
        } else if (c == b'e' || c == b'E') && !seen_e {
            seen_e = true;
            end += 1;
            if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
                end += 1;
            }
        } else {
            break;
        }
    }
    t[..end].parse().unwrap_or(f64::NAN)
}

fn percent_encode(s: &str, uri: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')')
            || (uri && matches!(b, b';' | b',' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b'#' | b'[' | b']'));
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut vals = Vec::new();
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 64,
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return None,
        };
        vals.push(v);
    }
    let mut out = Vec::new();
    for chunk in vals.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let n = ((chunk[0] as u32) << 18)
            | ((chunk[1] as u32) << 12)
            | ((chunk.get(2).copied().unwrap_or(64).min(63) as u32) << 6)
            | chunk.get(3).copied().unwrap_or(64).min(63) as u32;
        out.push((n >> 16) as u8);
        if chunk.get(2).copied().unwrap_or(64) != 64 {
            out.push((n >> 8) as u8);
        }
        if chunk.get(3).copied().unwrap_or(64) != 64 {
            out.push(n as u8);
        }
    }
    Some(out)
}

fn system_now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

fn parse_date(s: &str) -> f64 {
    let s = s.trim();
    let date = s.split('T').next().unwrap_or(s);
    let mut parts = date.split('-');
    let y: i64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1970);
    let m: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
    let d: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
    let time = s.split('T').nth(1).unwrap_or("00:00:00");
    let time = time.trim_end_matches('Z');
    let mut tp = time.split(':');
    let hh: i64 = tp.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let mm: i64 = tp.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let ss_part = tp.next().unwrap_or("0");
    let mut sp = ss_part.split('.');
    let ss: i64 = sp.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let milli: i64 = sp.next().unwrap_or("0").chars().take(3).collect::<String>().parse().unwrap_or(0);
    (days_from_civil(y, m, d) * 86_400_000 + hh * 3_600_000 + mm * 60_000 + ss * 1000 + milli) as f64
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

fn date_parts(ms: f64) -> (i64, u32, u32, i64, i64, i64, i64) {
    if !ms.is_finite() {
        return (1970, 0, 1, 0, 0, 0, 0);
    }
    let secs = (ms / 1000.0).floor() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (y, m - 1, d, rem / 3600, (rem % 3600) / 60, rem % 60, (ms as i64).rem_euclid(1000))
}

fn weekday(ms: f64) -> f64 {
    let days = (ms / 86_400_000.0).floor() as i64;
    ((days + 4).rem_euclid(7)) as f64
}

fn format_date(ms: f64) -> String {
    if !ms.is_finite() {
        return "Invalid Date".into();
    }
    let (y, m, d, hh, mm, ss, milli) = date_parts(ms);
    format!("{y:04}-{:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{milli:03}Z", m + 1)
}

/// Deep-copies arrays, plain objects and dates for `structuredClone`.
/// Functions, promises, maps, sets and cycles are rejected like the web API.
fn clone_structured(it: &mut Interp, v: &Value, depth: usize) -> Result<Value, JsError> {
    if depth > 64 {
        return Err(JsError::type_error(
            "structuredClone: object is too deeply nested or circular",
        ));
    }
    if let Some(values) = array_values(v) {
        let mut out = Vec::with_capacity(values.len());
        for item in &values {
            out.push(clone_structured(it, item, depth + 1)?);
        }
        return Ok(it.array_from(out));
    }
    if let Some(src) = v.as_object() {
        let date = match &src.borrow().kind {
            ObjKind::Date(t) => Some(*t),
            ObjKind::Map(_) | ObjKind::Set(_) | ObjKind::Promise(_) | ObjKind::Function(_) => {
                return Err(JsError::type_error(
                    "structuredClone: value cannot be cloned",
                ))
            }
            _ => None,
        };
        if let Some(t) = date {
            let obj = new_object(Some(it.intrinsics.date_proto.clone()));
            obj.borrow_mut().kind = ObjKind::Date(t);
            return Ok(Value::Object(obj));
        }
        let obj = it.object();
        let target = obj.as_object().unwrap().clone();
        for (k, val) in own_key_values(&src) {
            let cloned = clone_structured(it, &val, depth + 1)?;
            define(&target, &k, cloned);
        }
        return Ok(obj);
    }
    Ok(v.clone())
}

fn make_message_channel(it: &mut Interp) -> Value {
    let channel = it.object();
    let port1 = it.object();
    let port2 = it.object();
    for (port, peer) in [(port1.clone(), port2.clone()), (port2.clone(), port1.clone())] {
        let Some(o) = port.as_object().cloned() else { continue };
        define(&o, "onmessage", Value::Null);
        define(&o, "onmessageerror", Value::Null);
        define(&o, "start", it.native("start", 0, |_, _, _| Ok(Value::Undefined)));
        define(&o, "close", it.native("close", 0, |_, _, _| Ok(Value::Undefined)));
        define(&o, "addEventListener", it.native("addEventListener", 2, |_, _, _| Ok(Value::Undefined)));
        define(&o, "removeEventListener", it.native("removeEventListener", 2, |_, _, _| Ok(Value::Undefined)));
        define(&o, "postMessage", it.native("postMessage", 1, move |it, _, args| {
            let event = it.object();
            if let Some(eo) = event.as_object() {
                define(eo, "type", Value::string("message"));
                define(eo, "data", arg(&args, 0));
                define(eo, "target", Value::Null);
            }
            let cb = it.get_member(peer.clone(), "onmessage")?;
            if cb.is_callable() {
                let peer = peer.clone();
                let task = it.native("message", 0, move |it, _, _| {
                    it.call_value(&cb, peer.clone(), vec![event.clone()])?;
                    Ok(Value::Undefined)
                });
                it.add_timer(task, 0.0, false, vec![]);
            }
            Ok(Value::Undefined)
        }));
    }
    if let Some(o) = channel.as_object() {
        define(o, "port1", port1);
        define(o, "port2", port2);
    }
    channel
}

fn populate_url(it: &mut Interp, obj: &Value, url: &veyra_common::Url) {
    let some = obj.as_object().cloned();
    let Some(o) = some else { return };
    let host = if let Some(port) = url.port {
        format!("{}:{port}", url.host)
    } else {
        url.host.clone()
    };
    let origin = url
        .origin()
        .map(|o| {
            let default_port = (o.scheme == "http" && o.port == 80) || (o.scheme == "https" && o.port == 443);
            if default_port { format!("{}://{}", o.scheme, o.host) } else { format!("{}://{}:{}", o.scheme, o.host, o.port) }
        })
        .unwrap_or_else(|| "null".into());
    for (name, value) in [
        ("href", url.to_string()),
        ("protocol", format!("{}:", url.scheme)),
        ("host", host),
        ("hostname", url.host.clone()),
        ("pathname", url.path.clone()),
        ("search", url.query.as_ref().map(|q| format!("?{q}")).unwrap_or_default()),
        ("hash", url.fragment.as_ref().map(|f| format!("#{f}")).unwrap_or_default()),
        ("origin", origin),
        ("username", url.username.clone()),
        ("password", url.password.clone()),
    ] {
        define(&o, name, Value::string(value));
    }
    let params = it.object();
    if let Some(p) = params.as_object().cloned() {
        define_params_api(it, &p);
        let entries = url.query.as_ref().map(|q| parse_query(q)).unwrap_or_default();
        set_params_entries(it, &params, &entries);
    }
    define(&o, "searchParams", params);
    if let Some(ctor) = it.get_member(Value::Object(o.clone()), "constructor").ok() {
        let _ = ctor;
    }
}

fn params_entries(it: &mut Interp, value: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(items) = it.get_member(value.clone(), "__entries") else { return out };
    let Some(items) = array_values(&items) else { return out };
    for item in items {
        if let Some(pair) = array_values(&item) {
            let key = pair.first().map(|v| it.to_string(v).unwrap_or_default()).unwrap_or_default();
            let val = pair.get(1).map(|v| it.to_string(v).unwrap_or_default()).unwrap_or_default();
            out.push((key, val));
        }
    }
    out
}

fn set_params_entries(it: &mut Interp, obj: &Value, entries: &[(String, String)]) {
    let arr = it.array_from(
        entries
            .iter()
            .map(|(k, v)| it.array_from(vec![Value::string(k.clone()), Value::string(v.clone())]))
            .collect(),
    );
    if let Some(o) = obj.as_object() {
        define(o, "__entries", arr);
    }
}

fn find_param(entries: &[(String, String)], name: &str) -> Option<String> {
    entries.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
}

fn define_params_api(it: &mut Interp, o: &ObjRef) {
    define(o, "get", it.native("get", 1, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let entries = params_entries(it, &this);
        Ok(find_param(&entries, &name).map(Value::string).unwrap_or(Value::Null))
    }));
    define(o, "getAll", it.native("getAll", 1, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let entries = params_entries(it, &this);
        Ok(it.array_from(entries.into_iter().filter(|(k, _)| *k == name).map(|(_, v)| Value::string(v)).collect()))
    }));
    define(o, "has", it.native("has", 1, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let entries = params_entries(it, &this);
        Ok(Value::Bool(entries.iter().any(|(k, _)| *k == name)))
    }));
    define(o, "append", it.native("append", 2, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let value = it.to_string(&arg(&args, 1))?;
        let mut entries = params_entries(it, &this);
        entries.push((name, value));
        set_params_entries(it, &this, &entries);
        Ok(Value::Undefined)
    }));
    define(o, "set", it.native("set", 2, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let value = it.to_string(&arg(&args, 1))?;
        let mut entries = params_entries(it, &this);
        if let Some(pos) = entries.iter().position(|(k, _)| *k == name) {
            entries.retain(|(k, _)| *k != name);
            entries.insert(pos.min(entries.len()), (name, value));
        } else {
            entries.push((name, value));
        }
        set_params_entries(it, &this, &entries);
        Ok(Value::Undefined)
    }));
    define(o, "delete", it.native("delete", 1, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let mut entries = params_entries(it, &this);
        entries.retain(|(k, _)| *k != name);
        set_params_entries(it, &this, &entries);
        Ok(Value::Undefined)
    }));
    define(o, "toString", it.native("toString", 0, |it, this, _| {
        let entries = params_entries(it, &this);
        Ok(Value::string(entries.iter().map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v))).collect::<Vec<_>>().join("&")))
    }));
    define(o, "keys", it.native("keys", 0, |it, this, _| {
        let entries = params_entries(it, &this);
        Ok(it.array_from(entries.into_iter().map(|(k, _)| Value::string(k)).collect()))
    }));
    define(o, "values", it.native("values", 0, |it, this, _| {
        let entries = params_entries(it, &this);
        Ok(it.array_from(entries.into_iter().map(|(_, v)| Value::string(v)).collect()))
    }));
    define(o, "entries", it.native("entries", 0, |it, this, _| {
        let entries = params_entries(it, &this);
        Ok(it.array_from(entries.into_iter().map(|(k, v)| it.array_from(vec![Value::string(k), Value::string(v)])).collect()))
    }));
    define(o, "forEach", it.native("forEach", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let entries = params_entries(it, &this);
        for (k, v) in entries {
            it.call_value(&cb, Value::Undefined, vec![Value::string(v), Value::string(k), this.clone()])?;
        }
        Ok(Value::Undefined)
    }));
    define(o, "sort", it.native("sort", 0, |it, this, _| {
        let mut entries = params_entries(it, &this);
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        set_params_entries(it, &this, &entries);
        Ok(Value::Undefined)
    }));
    define(o, "size", Value::Number(0.0));
}

fn header_init_pairs(it: &mut Interp, init: Option<Value>) -> Result<Vec<(String, String)>, JsError> {
    let mut out = Vec::new();
    match init {
        Some(Value::String(s)) => {
            for line in s.split('\n') {
                if let Some((k, v)) = line.split_once(':') {
                    out.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
        }
        Some(Value::Object(o)) => {
            if let Ok(existing) = it.get_member(Value::Object(o.clone()), "__entries") {
                if !existing.is_undefined() {
                    return Ok(params_entries(it, &existing));
                }
            }
            for (k, v) in own_key_values(&o) {
                out.push((k, it.to_string(&v)?));
            }
        }
        _ => {}
    }
    Ok(out)
}

fn make_headers(it: &mut Interp, init: Option<Value>) -> Value {
    let obj = it.object();
    if let Some(o) = obj.as_object().cloned() {
        define_headers_api(it, &o);
        let entries = header_init_pairs(it, init).unwrap_or_default();
        set_params_entries(it, &obj, &entries);
    }
    obj
}

fn define_headers_api(it: &mut Interp, o: &ObjRef) {
    define(o, "get", it.native("get", 1, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?.to_lowercase();
        let entries = params_entries(it, &this);
        Ok(entries.into_iter().find(|(k, _)| k.to_lowercase() == name).map(|(_, v)| Value::string(v)).unwrap_or(Value::Null))
    }));
    define(o, "has", it.native("has", 1, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?.to_lowercase();
        let entries = params_entries(it, &this);
        Ok(Value::Bool(entries.iter().any(|(k, _)| k.to_lowercase() == name)))
    }));
    define(o, "set", it.native("set", 2, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let value = it.to_string(&arg(&args, 1))?;
        let mut entries = params_entries(it, &this);
        entries.retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
        entries.push((name, value));
        set_params_entries(it, &this, &entries);
        Ok(Value::Undefined)
    }));
    define(o, "append", it.native("append", 2, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let value = it.to_string(&arg(&args, 1))?;
        let mut entries = params_entries(it, &this);
        entries.push((name, value));
        set_params_entries(it, &this, &entries);
        Ok(Value::Undefined)
    }));
    define(o, "delete", it.native("delete", 1, |it, this, args| {
        let name = it.to_string(&arg(&args, 0))?;
        let mut entries = params_entries(it, &this);
        entries.retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
        set_params_entries(it, &this, &entries);
        Ok(Value::Undefined)
    }));
    define(o, "forEach", it.native("forEach", 1, |it, this, args| {
        let cb = arg(&args, 0);
        let entries = params_entries(it, &this);
        for (k, v) in entries {
            it.call_value(&cb, Value::Undefined, vec![Value::string(v), Value::string(k), this.clone()])?;
        }
        Ok(Value::Undefined)
    }));
    define(o, "entries", it.native("entries", 0, |it, this, _| {
        let entries = params_entries(it, &this);
        Ok(it.array_from(entries.into_iter().map(|(k, v)| it.array_from(vec![Value::string(k), Value::string(v)])).collect()))
    }));
    define(o, "keys", it.native("keys", 0, |it, this, _| {
        let entries = params_entries(it, &this);
        Ok(it.array_from(entries.into_iter().map(|(k, _)| Value::string(k)).collect()))
    }));
    define(o, "values", it.native("values", 0, |it, this, _| {
        let entries = params_entries(it, &this);
        Ok(it.array_from(entries.into_iter().map(|(_, v)| Value::string(v)).collect()))
    }));
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (form_decode(k), form_decode(v)),
            None => (form_decode(pair), String::new()),
        })
        .collect()
}

fn form_decode(s: &str) -> String {
    percent_decode(&s.replace('+', " "))
}

fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'*');
        if keep {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn format_grouped(n: f64, min_frac: usize, max_frac: usize, grouping: bool) -> String {
    if !n.is_finite() {
        return if n.is_nan() { "NaN".into() } else if n > 0.0 { "∞".into() } else { "-∞".into() };
    }
    let negative = n < 0.0;
    let text = format!("{:.max_frac$}", n.abs());
    let (int_part, frac_part) = match text.split_once('.') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (text, String::new()),
    };
    let mut frac_part = frac_part;
    while frac_part.len() > min_frac && frac_part.ends_with('0') {
        frac_part.pop();
    }
    let mut int_part = int_part;
    if grouping {
        let mut grouped = String::new();
        let digits: Vec<char> = int_part.chars().collect();
        for (i, c) in digits.iter().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(*c);
        }
        int_part = grouped;
    }
    let mut out = if frac_part.is_empty() { int_part } else { format!("{int_part}.{frac_part}") };
    if negative {
        out.insert(0, '-');
    }
    out
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn utc_date_string(ms: f64) -> String {
    if !ms.is_finite() {
        return "Invalid Date".into();
    }
    let (y, m, d, hh, mm, ss, _) = date_parts(ms);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[weekday(ms) as usize], d, MONTHS[m as usize], y, hh, mm, ss
    )
}

fn make_response(it: &mut Interp, resp: crate::host::FetchResponse) -> Value {
    let obj = it.object();
    if let Some(o) = obj.as_object() {
        define(o, "status", Value::Number(resp.status as f64));
        define(o, "ok", Value::Bool(resp.status >= 200 && resp.status < 300));
        define(o, "url", Value::string(resp.url));
        define(o, "statusText", Value::string(String::new()));
        let headers = it.object();
        if let Some(h) = headers.as_object() {
            let list = resp.headers.clone();
            define(h, "get", it.native("get", 1, move |it, _, args| {
                let name = it.to_string(&arg(&args, 0))?.to_lowercase();
                Ok(list
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == name)
                    .map(|(_, v)| Value::string(v.clone()))
                    .unwrap_or(Value::Null))
            }));
        }
        define(o, "headers", headers);
        let body = resp.body;
        define(o, "text", it.native("text", 0, {
            let body = body.clone();
            move |it, _, _| {
                let (p, d) = it.new_promise();
                it.resolve_promise(&d, Value::string(body.clone()));
                Ok(p)
            }
        }));
        define(o, "json", it.native("json", 0, {
            let body = body.clone();
            move |it, _, _| {
                let (p, d) = it.new_promise();
                match it.json_parse(&body) {
                    Ok(v) => it.resolve_promise(&d, v),
                    Err(e) => {
                        let ev = it.error_value(&e);
                        it.reject_promise(&d, ev);
                    }
                }
                Ok(p)
            }
        }));
    }
    obj
}

trait NumberExt {
    fn is_number_nan(&self) -> bool;
}

impl NumberExt for Value {
    fn is_number_nan(&self) -> bool {
        matches!(self, Value::Number(n) if n.is_nan())
    }

}

/// Global `eval`. Page-inserted instrumentation and some Google loaders call
/// `eval` as a free identifier, so it must exist on the global object and in
/// global scope lookup.
fn install_eval(it: &mut Interp) {
    let eval_fn = it.native("eval", 1, |it, _, args| {
        let Some(src) = args.first() else { return Ok(Value::Undefined) };
        let Value::String(text) = src else { return Ok(src.clone()) };
        it.eval_source(text)
    });
    it.set_global("eval", eval_fn);
}
