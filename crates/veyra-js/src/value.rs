//! JavaScript values, objects, functions, environments and errors.

use std::cell::RefCell;
use std::collections::HashMap;

use indexmap::IndexMap;
use std::fmt;
use std::rc::Rc;

use crate::ast::{ClassNode, Expr, FnNode, Pattern};
use crate::runtime::Interp;

pub type ObjRef = Rc<RefCell<Object>>;
pub type EnvRef = Rc<RefCell<Env>>;
pub type NativeImpl = dyn Fn(&mut Interp, Value, Vec<Value>) -> Result<Value, JsError>;

#[derive(Clone)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    BigInt(i128),
    String(Rc<str>),
    Object(ObjRef),
}

impl Value {
    pub fn string(s: impl Into<String>) -> Value {
        Value::String(Rc::from(s.into().as_str()))
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn is_nullish(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
    pub fn as_object(&self) -> Option<&ObjRef> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }
    pub fn is_callable(&self) -> bool {
        match self {
            Value::Object(o) => matches!(o.borrow().kind, ObjKind::Function(_)),
            _ => false,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bigint(&self) -> Option<i128> {
        match self {
            Value::BigInt(n) => Some(*n),
            _ => None,
        }
    }
    pub fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::BigInt(n) => *n != 0,
            Value::String(s) => !s.is_empty(),
            Value::Object(_) => true,
        }
    }
    pub fn type_of(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::BigInt(_) => "bigint",
            Value::String(_) => "string",
            Value::Object(o) => match o.borrow().kind {
                ObjKind::Function(_) => "function",
                _ => "object",
            },
        }
    }
    pub fn to_number(&self) -> f64 {
        match self {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Number(n) => *n,
            Value::BigInt(n) => *n as f64,
            Value::String(s) => string_to_number(s),
            Value::Object(_) => f64::NAN,
        }
    }
    pub fn to_string_primitive(&self) -> String {
        match self {
            Value::Undefined => "undefined".into(),
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => number_to_string(*n),
            Value::BigInt(n) => n.to_string(),
            Value::String(s) => s.to_string(),
            Value::Object(_) => "[object Object]".into(),
        }
    }
    pub fn to_i32(&self) -> i32 {
        let n = self.to_number();
        if !n.is_finite() || n == 0.0 {
            return 0;
        }
        let m = n.trunc() as i64 as u64;
        (m & 0xffff_ffff) as u32 as i32
    }
    pub fn to_u32(&self) -> u32 {
        self.to_i32() as u32
    }
    pub fn to_index(&self) -> usize {
        let n = self.to_number();
        if n.is_nan() || n < 0.0 {
            0
        } else if n > 9_007_199_254_740_991.0 {
            usize::MAX
        } else {
            n as usize
        }
    }
    pub fn same_value_zero(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::BigInt(a), Value::BigInt(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => {
                (a.is_nan() && b.is_nan()) || (a == b)
            }
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::Undefined, Value::Undefined) => true,
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Undefined => write!(f, "undefined"),
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Number(n) => write!(f, "{n}"),
            Value::BigInt(n) => write!(f, "{n}n"),
            Value::String(s) => write!(f, "{s:?}"),
            Value::Object(o) => {
                let o = o.borrow();
                match &o.kind {
                    ObjKind::Function(fun) => write!(f, "[Function {}]", fun.name()),
                    ObjKind::Array(_) => write!(f, "[Array]"),
                    _ => write!(f, "[Object]"),
                }
            }
        }
    }
}

pub fn strict_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::BigInt(x), Value::BigInt(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

pub fn loose_eq(a: &Value, b: &Value) -> bool {
    if strict_eq(a, b) {
        return true;
    }
    match (a, b) {
        (Value::Null, Value::Undefined) | (Value::Undefined, Value::Null) => true,
        (Value::Number(x), Value::String(s)) => *x == string_to_number(s),
        (Value::String(s), Value::Number(x)) => string_to_number(s) == *x,
        (Value::BigInt(x), Value::Number(y)) | (Value::Number(y), Value::BigInt(x)) => *x as f64 == *y,
        (Value::BigInt(x), Value::String(s)) | (Value::String(s), Value::BigInt(x)) => {
            s.trim().parse::<i128>().map(|v| v == *x).unwrap_or(false)
        }
        (Value::Bool(x), _) => Value::Number(if *x { 1.0 } else { 0.0 }).same_value_zero(a) || {
            let n = Value::Number(if *x { 1.0 } else { 0.0 });
            loose_eq(&n, b)
        },
        (_, Value::Bool(y)) => loose_eq(a, &Value::Number(if *y { 1.0 } else { 0.0 })),
        (Value::Number(x), _) if b.is_object() => *x == b.to_number(),
        (_, Value::Number(y)) if a.is_object() => a.to_number() == *y,
        (Value::String(x), _) if b.is_object() => x.as_ref() == b.to_string_primitive(),
        (_, Value::String(y)) if a.is_object() => a.to_string_primitive() == y.as_ref(),
        _ => false,
    }
}

pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n == f64::INFINITY {
        return "Infinity".into();
    }
    if n == f64::NEG_INFINITY {
        return "-Infinity".into();
    }
    if n == 0.0 {
        return "0".into();
    }
    if n.fract() == 0.0 && n.abs() < 1e21 {
        return format!("{}", n as i64);
    }
    let mut s = format!("{n}");
    if let Some(pos) = s.find('e') {
        let (mantissa, exp) = s.split_at(pos);
        let exp = &exp[1..];
        let exp = exp.strip_prefix('+').unwrap_or(exp);
        s = format!("{mantissa}e{exp}");
    }
    s
}

pub fn string_to_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    if let Some(bin) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        return i64::from_str_radix(bin, 2).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    if let Some(oct) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        return i64::from_str_radix(oct, 8).map(|v| v as f64).unwrap_or(f64::NAN);
    }
    match t {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => t.parse::<f64>().unwrap_or(f64::NAN),
    }
}

#[derive(Clone)]
pub enum Func {
    User(Rc<UserFunction>),
    Native(Rc<NativeFunction>),
}

impl Func {
    pub fn name(&self) -> String {
        match self {
            Func::User(f) => f.node.name.clone().unwrap_or_default(),
            Func::Native(f) => f.name.clone(),
        }
    }
}

pub struct UserFunction {
    pub node: Rc<FnNode>,
    pub source: Option<String>,
    pub closure: EnvRef,
    pub home: Option<ObjRef>,
    pub parent_ctor: Option<Value>,
    pub super_proto: Option<ObjRef>,
    pub fields: Vec<(String, Option<Expr>)>,
    pub is_class_ctor: bool,
    pub derived: bool,
}

pub struct NativeFunction {
    pub name: String,
    pub length: usize,
    pub f: Rc<NativeImpl>,
}

impl fmt::Debug for NativeFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "native {}", self.name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PromiseState {
    Pending,
    Fulfilled,
    Rejected,
}

pub struct PromiseData {
    pub state: PromiseState,
    pub value: Value,
    pub reactions: Vec<PromiseReaction>,
}

pub struct PromiseReaction {
    pub on_fulfilled: Option<Value>,
    pub on_rejected: Option<Value>,
    pub result: Rc<RefCell<PromiseData>>,
}

pub enum ObjKind {
    Ordinary,
    Array(Vec<Value>),
    Function(Func),
    Error { name: String },
    Date(f64),
    RegExp { source: String, flags: String, last_index: usize },
    Map(Vec<(Value, Value)>),
    Set(Vec<Value>),
    Promise(Rc<RefCell<PromiseData>>),
    Arguments,
    BooleanObj(bool),
    NumberObj(f64),
    StringObj(String),
}

pub struct Object {
    pub proto: Option<ObjRef>,
    /// Insertion-ordered so `Object.keys`, `for…in` and `JSON.stringify` follow
    /// JavaScript's own-property ordering instead of hash order.
    pub props: IndexMap<String, Prop>,
    pub kind: ObjKind,
    /// For arrays: which indices are holes (present in length but absent as
    /// own properties). Holes are skipped by map/forEach/filter and by
    /// Array.prototype iteration, just as in V8.
    pub array_holes: Option<Vec<bool>>,
    pub extensible: bool,
    /// A JS Proxy: (target, handler). Traps are applied on member access.
    pub proxy: Option<(ObjRef, ObjRef)>,
}

#[derive(Clone)]
pub struct Prop {
    pub value: Value,
    pub get: Option<Value>,
    pub set: Option<Value>,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}

impl Prop {
    pub fn data(value: Value, enumerable: bool) -> Prop {
        Prop {
            value,
            get: None,
            set: None,
            writable: true,
            enumerable,
            configurable: true,
        }
    }
    pub fn ro(value: Value) -> Prop {
        Prop {
            value,
            get: None,
            set: None,
            writable: false,
            enumerable: false,
            configurable: true,
        }
    }
    pub fn accessor(get: Option<Value>, set: Option<Value>, enumerable: bool) -> Prop {
        Prop {
            value: Value::Undefined,
            get,
            set,
            writable: false,
            enumerable,
            configurable: true,
        }
    }
}

pub fn new_object(proto: Option<ObjRef>) -> ObjRef {
    Rc::new(RefCell::new(Object {
        proto,
        props: IndexMap::new(),
        kind: ObjKind::Ordinary,
        array_holes: None,
        extensible: true,
        proxy: None,
    }))
}

pub fn new_array(values: Vec<Value>, array_proto: ObjRef) -> ObjRef {
    Rc::new(RefCell::new(Object {
        proto: Some(array_proto),
        props: IndexMap::new(),
        kind: ObjKind::Array(values),
        array_holes: None,
        extensible: true,
        proxy: None,
    }))
}

pub fn new_array_with_holes(values: Vec<Value>, holes: Vec<bool>, array_proto: ObjRef) -> ObjRef {
    let obj = new_array(values, array_proto);
    obj.borrow_mut().array_holes = Some(holes);
    obj
}

pub fn new_array_like(values: Vec<Value>, proto: ObjRef) -> ObjRef {
    let mut props = IndexMap::new();
    for (i, v) in values.iter().enumerate() {
        props.insert(i.to_string(), Prop::data(v.clone(), true));
    }
    props.insert("length".into(), Prop::data(Value::Number(values.len() as f64), false));
    Rc::new(RefCell::new(Object {
        proto: Some(proto),
        props,
        kind: ObjKind::Arguments,
        array_holes: None,
        extensible: true,
        proxy: None,
    }))
}

pub fn array_values(v: &Value) -> Option<Vec<Value>> {
    match v {
        Value::Object(o) => match &o.borrow().kind {
            ObjKind::Array(values) => Some(values.clone()),
            _ => None,
        },
        _ => None,
    }
}

pub fn is_array(v: &Value) -> bool {
    matches!(v, Value::Object(o) if matches!(o.borrow().kind, ObjKind::Array(_)))
}

pub fn is_string_object(v: &Value) -> bool {
    matches!(v, Value::Object(o) if matches!(o.borrow().kind, ObjKind::StringObj(_)))
}

pub fn own_prop(obj: &ObjRef, key: &str) -> Option<Prop> {
    let o = obj.borrow();
    if let ObjKind::Array(values) = &o.kind {
        if key == "length" {
            return Some(Prop::data(Value::Number(values.len() as f64), false));
        }
        if let Some(i) = parse_index(key) {
            return values.get(i).cloned().map(|v| Prop::data(v, true));
        }
    }
    if let ObjKind::StringObj(s) = &o.kind {
        if key == "length" {
            return Some(Prop::ro(Value::Number(s.chars().count() as f64)));
        }
        if let Some(i) = parse_index(key) {
            return s.chars().nth(i).map(|c| Prop::data(Value::string(c.to_string()), true));
        }
    }
    if let ObjKind::Function(f) = &o.kind {
        if key == "length" {
            return Some(Prop::ro(Value::Number(match f {
                Func::Native(n) => n.length as f64,
                Func::User(u) => u.node.params.len() as f64,
            })));
        }
        if key == "name" {
            return Some(Prop::ro(Value::string(f.name())));
        }
    }
    o.props.get(key).cloned()
}

pub fn find_prop(obj: &ObjRef, key: &str) -> Option<(ObjRef, Prop)> {
    let mut cur = Some(obj.clone());
    let mut depth = 0;
    while let Some(o) = cur {
        if let Some(p) = own_prop(&o, key) {
            return Some((o, p));
        }
        let next = o.borrow().proto.clone();
        cur = next;
        depth += 1;
        if depth > 200 {
            break;
        }
    }
    None
}

pub fn data_prop(obj: &ObjRef, key: &str) -> Option<Value> {
    find_prop(obj, key).and_then(|(_, p)| {
        if p.get.is_none() {
            Some(p.value)
        } else {
            None
        }
    })
}

pub fn has_property(obj: &ObjRef, key: &str) -> bool {
    find_prop(obj, key).is_some()
}

pub fn define(obj: &ObjRef, key: &str, value: Value) {
    define_with(obj, key, value, true, true, true);
}

pub fn define_with(
    obj: &ObjRef,
    key: &str,
    value: Value,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) {
    let mut o = obj.borrow_mut();
    if let ObjKind::Array(values) = &mut o.kind {
        if key == "length" {
            let n = value.to_index();
            values.truncate(n);
            return;
        }
        if let Some(i) = parse_index(key) {
            while values.len() <= i {
                values.push(Value::Undefined);
            }
            values[i] = value;
            return;
        }
    }
    o.props.insert(
        key.to_string(),
        Prop {
            value,
            get: None,
            set: None,
            writable,
            enumerable,
            configurable,
        },
    );
}

pub fn define_accessor(obj: &ObjRef, key: &str, get: Option<Value>, set: Option<Value>, enumerable: bool) {
    obj.borrow_mut().props.insert(key.to_string(), Prop::accessor(get, set, enumerable));
}

pub fn delete_prop(obj: &ObjRef, key: &str) -> bool {
    let mut o = obj.borrow_mut();
    if let ObjKind::Array(values) = &mut o.kind {
        if let Some(i) = parse_index(key) {
            if i < values.len() {
                values.remove(i);
            }
            return true;
        }
    }
    o.props.remove(key).is_some()
}

pub fn own_keys(obj: &ObjRef) -> Vec<String> {
    let o = obj.borrow();
    let mut keys = Vec::new();
    if let ObjKind::Array(values) = &o.kind {
        for i in 0..values.len() {
            keys.push(i.to_string());
        }
    }
    if let ObjKind::StringObj(s) = &o.kind {
        for i in 0..s.chars().count() {
            keys.push(i.to_string());
        }
    }
    for (k, p) in &o.props {
        if p.enumerable && !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    keys
}

pub fn own_key_values(obj: &ObjRef) -> Vec<(String, Value)> {
    own_keys(obj)
        .into_iter()
        .filter_map(|k| own_prop(obj, &k).map(|p| (k, p.value)))
        .collect()
}

pub fn parse_index(key: &str) -> Option<usize> {
    if key.is_empty() || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    if key.chars().all(|c| c.is_ascii_digit()) {
        key.parse().ok()
    } else {
        None
    }
}

pub struct Env {
    pub vars: HashMap<String, Binding>,
    pub parent: Option<EnvRef>,
    pub this: Value,
    pub is_function: bool,
    pub global: Option<ObjRef>,
    pub home: Option<ObjRef>,
}

#[derive(Clone)]
pub struct Binding {
    pub value: Value,
    pub mutable: bool,
}

impl Env {
    pub fn new(parent: Option<EnvRef>, this: Value, is_function: bool) -> EnvRef {
        Rc::new(RefCell::new(Env {
            vars: HashMap::new(),
            parent,
            this,
            is_function,
            global: None,
            home: None,
        }))
    }

    pub fn new_block(parent: EnvRef) -> EnvRef {
        let this = parent.borrow().this.clone();
        Rc::new(RefCell::new(Env {
            vars: HashMap::new(),
            parent: Some(parent),
            this,
            is_function: false,
            global: None,
            home: None,
        }))
    }
}

pub fn lookup(env: &EnvRef, name: &str) -> Option<Value> {
    let mut cur = Some(env.clone());
    let mut depth = 0;
    while let Some(e) = cur {
        if let Some(b) = e.borrow().vars.get(name) {
            return Some(b.value.clone());
        }
        let next = e.borrow().parent.clone();
        cur = next;
        depth += 1;
        if depth > 500 {
            break;
        }
    }
    None
}

pub fn env_this(env: &EnvRef) -> Value {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let borrowed = e.borrow();
        if borrowed.is_function {
            return borrowed.this.clone();
        }
        cur = borrowed.parent.clone();
    }
    Value::Undefined
}

/// Declares a binding in the nearest function/global scope (for `var`).
pub fn declare_var(env: &EnvRef, name: &str, value: Value) {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let is_fn = {
            let b = e.borrow();
            b.is_function || b.global.is_some()
        };
        if is_fn {
            let mut b = e.borrow_mut();
            b.vars.entry(name.to_string()).or_insert(Binding { value: value.clone(), mutable: true });
            if let Some(g) = &b.global {
                let mut gb = g.borrow_mut();
                gb.props.insert(name.to_string(), Prop::data(value, true));
            }
            return;
        }
        let parent = e.borrow().parent.clone();
        cur = parent;
    }
}

pub fn declare_lexical(env: &EnvRef, name: &str, value: Value, mutable: bool) {
    env.borrow_mut().vars.insert(name.to_string(), Binding { value, mutable });
}

pub fn assign_env(env: &EnvRef, name: &str, value: Value) -> bool {
    let mut cur = Some(env.clone());
    while let Some(e) = cur {
        let mut b = e.borrow_mut();
        if let Some(binding) = b.vars.get_mut(name) {
            if binding.mutable {
                binding.value = value.clone();
                if let Some(g) = &b.global {
                    if let Some(p) = g.borrow_mut().props.get_mut(name) {
                        p.value = value;
                    }
                }
            }
            return true;
        }
        // Assignment to an undeclared name becomes a global property.
        if let Some(g) = &b.global {
            g.borrow_mut().props.insert(name.to_string(), Prop::data(value.clone(), true));
            b.vars.insert(name.to_string(), Binding { value, mutable: true });
            return true;
        }
        let parent = b.parent.clone();
        cur = parent.map(|p| p);
    }
    false
}

#[derive(Debug, Clone)]
pub struct JsError {
    pub kind: &'static str,
    pub message: String,
    pub value: Option<Value>,
    pub stack: Vec<String>,
    /// Source line where the error was first raised.
    pub line: u32,
}

impl JsError {
    pub fn type_error(msg: impl Into<String>) -> JsError {
        let message = msg.into();
        JsError { kind: "TypeError", message, value: None, stack: Vec::new(), line: 0 }
    }
    pub fn range_error(msg: impl Into<String>) -> JsError {
        let message = msg.into();
        JsError { kind: "RangeError", message, value: None, stack: Vec::new(), line: 0 }
    }
    pub fn reference_error(msg: impl Into<String>) -> JsError {
        let message = msg.into();
        JsError { kind: "ReferenceError", message, value: None, stack: Vec::new(), line: 0 }
    }
    pub fn syntax_error(msg: impl Into<String>) -> JsError {
        let message = msg.into();
        JsError { kind: "SyntaxError", message, value: None, stack: Vec::new(), line: 0 }
    }
    pub fn throw(value: Value) -> JsError {
        JsError { kind: "throw", message: "uncaught exception".into(), value: Some(value), stack: Vec::new(), line: 0 }
    }
    pub fn display(&self) -> String {
        if self.kind == "throw" {
            match &self.value {
                Some(v) => format!("Uncaught {}", thrown_to_string(v)),
                None => "Uncaught exception".into(),
            }
        } else if self.message.is_empty() {
            self.kind.to_string()
        } else {
            let base = format!("{}: {}", self.kind, self.message);
            if self.line > 0 {
                format!("{base} [line {}]", self.line)
            } else {
                base
            }
        }
    }
}

/// Render a thrown value the way a console does: strings as-is, error objects
/// as `Name: message`, everything else with its debug form.
fn thrown_to_string(v: &Value) -> String {
    if let Value::String(s) = v {
        return s.to_string();
    }
    if let Some(o) = v.as_object() {
        let o = o.borrow();
        let text = |key: &str| match o.props.get(key).map(|p| &p.value) {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        };
        if let Some(message) = text("message") {
            let name = text("name").unwrap_or_else(|| "Error".into());
            return format!("{name}: {message}");
        }
    }
    format!("{v:?}")
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// A class constructor's static class record, used while evaluating classes.
pub struct ClassRecord {
    pub node: Rc<ClassNode>,
    pub closure: EnvRef,
    pub prototype: ObjRef,
    pub ctor: Option<Rc<UserFunction>>,
}

pub fn pattern_names(pat: &Pattern, out: &mut Vec<String>) {
    match pat {
        Pattern::Ident(n) => out.push(n.clone()),
        Pattern::Array { items, rest } => {
            for i in items.iter().flatten() {
                pattern_names(i, out);
            }
            if let Some(r) = rest {
                pattern_names(r, out);
            }
        }
        Pattern::Object { props, rest } => {
            for (_, p, _) in props {
                if let Some(p) = p {
                    pattern_names(p, out);
                }
            }
            if let Some(r) = rest {
                pattern_names(r, out);
            }
        }
        Pattern::Default(p, _) => pattern_names(p, out),
        Pattern::Rest(p) => pattern_names(p, out),
    }
}

pub fn is_constructor_value(v: &Value) -> bool {
    match v {
        Value::Object(o) => match &o.borrow().kind {
            ObjKind::Function(Func::User(_)) => true,
            ObjKind::Function(Func::Native(_)) => true,
            _ => false,
        },
        _ => false,
    }
}
