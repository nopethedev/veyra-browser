//! The interpreter: environments, expressions, statements, calls, classes,
//! promises, timers and the microtask queue.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use veyra_common::json;

use crate::ast::*;
use crate::builtins;
use crate::host::Host;
use crate::value::*;

pub type Job = Box<dyn FnOnce(&mut Interp) -> Result<(), JsError>>;

pub struct Timer {
    pub id: f64,
    pub due: f64,
    pub repeat: Option<f64>,
    pub callback: Value,
    pub args: Vec<Value>,
}

pub struct Intrinsics {
    pub object_proto: ObjRef,
    pub function_proto: ObjRef,
    pub array_proto: ObjRef,
    pub string_proto: ObjRef,
    pub number_proto: ObjRef,
    pub boolean_proto: ObjRef,
    pub error_proto: ObjRef,
    pub type_error_proto: ObjRef,
    pub range_error_proto: ObjRef,
    pub syntax_error_proto: ObjRef,
    pub reference_error_proto: ObjRef,
    pub date_proto: ObjRef,
    pub regexp_proto: ObjRef,
    pub map_proto: ObjRef,
    pub set_proto: ObjRef,
    pub promise_proto: ObjRef,
}

enum Ref {
    Env(EnvRef, String),
    Member(Value, String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Completion {
    Normal,
    Return,
    Break,
    Continue,
}

pub struct Interp {
    pub global: ObjRef,
    pub global_env: EnvRef,
    pub intrinsics: Intrinsics,
    pub host: Box<dyn Host>,
    pub microtasks: VecDeque<Job>,
    pub timers: Vec<Timer>,
    pub next_timer: f64,
    pub depth: usize,
    pub constructing: bool,
    /// Custom-element upgrades invoke a class constructor with an existing
    /// wrapper as `this`; ordinary calls must not be able to do that.
    pub allow_class_call: bool,
    pub now: f64,
    pub steps: u64,
    /// Source line of the statement currently executing, used in error stacks.
    pub current_line: u32,
    pub step_limit: u64,
    pub step_log: bool,
    pub started: std::time::Instant,
    turn_started: std::time::Instant,
    turn_limit_ms: u64,
    pub regex_cache: HashMap<String, regress::Regex>,
    pub stack: Vec<String>,
    user_stack: Vec<Rc<UserFunction>>,
    /// `new.target` for each active non-arrow function frame. Arrow frames
    /// inherit the value from their enclosing function.
    new_targets: Vec<Option<Value>>,
    /// The target consumed by the next user-function call started by `new` or
    /// `super`.
    pending_new_target: Option<Value>,
    pub current_source: Option<String>,
    pub output: Vec<String>,
    pub symbol_seq: u64,
    pub symbol_registry: HashMap<String, String>,
    yield_stack: Vec<Vec<Value>>,
    /// The promise an async body suspended on at its last pending `await`.
    async_suspend: Option<Value>,
    rng: u64,
    /// Dynamic `import()` requests for the host to resolve after a turn.
    pending_imports: Vec<(String, Value, Value)>,
}

impl Interp {
    pub fn new(host: Box<dyn Host>) -> Interp {
        let object_proto = new_object(None);
        let function_proto_value = make_native_value(
            "Function.prototype",
            0,
            Some(object_proto.clone()),
            Rc::new(|_, _, _| Ok(Value::Undefined)),
        );
        let function_proto = function_proto_value.as_object().unwrap().clone();

        let plain = |p: &ObjRef| new_object(Some(p.clone()));
        let array_proto = plain(&object_proto);
        let string_proto = plain(&object_proto);
        let number_proto = plain(&object_proto);
        let boolean_proto = plain(&object_proto);
        let error_proto = plain(&object_proto);
        let type_error_proto = new_object(Some(error_proto.clone()));
        let range_error_proto = new_object(Some(error_proto.clone()));
        let syntax_error_proto = new_object(Some(error_proto.clone()));
        let reference_error_proto = new_object(Some(error_proto.clone()));
        let date_proto = plain(&object_proto);
        let regexp_proto = plain(&object_proto);
        let map_proto = plain(&object_proto);
        let set_proto = plain(&object_proto);
        let promise_proto = plain(&object_proto);

        let intrinsics = Intrinsics {
            object_proto: object_proto.clone(),
            function_proto,
            array_proto,
            string_proto,
            number_proto,
            boolean_proto,
            error_proto,
            type_error_proto,
            range_error_proto,
            syntax_error_proto,
            reference_error_proto,
            date_proto,
            regexp_proto,
            map_proto,
            set_proto,
            promise_proto,
        };
        let global = new_object(Some(object_proto));
        let global_env = Rc::new(RefCell::new(Env {
            vars: HashMap::new(),
            parent: None,
            this: Value::Object(global.clone()),
            is_function: true,
            global: Some(global.clone()),
            home: None,
        }));
        let mut interp = Interp {
            global,
            global_env,
            intrinsics,
            host,
            microtasks: VecDeque::new(),
            timers: Vec::new(),
            next_timer: 1.0,
            depth: 0,
            constructing: false,
            allow_class_call: false,
            now: 0.0,
            steps: 0,
            current_line: 0,
            step_limit: std::env::var("VEYRA_MAX_STEPS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50_000_000),
            step_log: std::env::var("VEYRA_STEP_LOG").is_ok(),
            started: std::time::Instant::now(),
            turn_started: std::time::Instant::now(),
            turn_limit_ms: std::env::var("VEYRA_JS_TIME_LIMIT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5000),
            regex_cache: HashMap::new(),
            stack: Vec::new(),
            user_stack: Vec::new(),
            new_targets: Vec::new(),
            pending_new_target: None,
            current_source: None,
            output: Vec::new(),
            symbol_seq: 1,
            symbol_registry: HashMap::new(),
            yield_stack: Vec::new(),
            async_suspend: None,
            rng: 0x2545_f491_4f6c_dd1d,
            pending_imports: Vec::new(),
        };
        builtins::install(&mut interp);
        interp
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Starts a new event-loop turn. Each turn gets a wall-clock budget so a
    /// runaway script or handler cannot freeze the browser indefinitely.
    pub fn begin_turn(&mut self) {
        self.turn_started = std::time::Instant::now();
    }

    fn check_turn(&self) -> Result<(), JsError> {
        if self.turn_limit_ms > 0
            && self.turn_started.elapsed().as_millis() as u64 > self.turn_limit_ms
        {
            return Err(JsError::range_error("script exceeded its time limit"));
        }
        Ok(())
    }

    pub fn eval_script(&mut self, src: &str) -> Result<Value, JsError> {
        self.begin_turn();
        let parse_start = std::time::Instant::now();
        let stmts = crate::parser::parse_program(src).map_err(|e| {
            let mut err = JsError::syntax_error(e.message);
            err.stack.push(format!("at line {}", e.line));
            err.stack.push(format!("at offset {}", e.offset));
            err
        })?;
        if std::env::var_os("VEYRA_DEBUG_PARSE").is_some() {
            eprintln!(
                "PARSE bytes={} stmts={} ms={}",
                src.len(),
                stmts.len(),
                parse_start.elapsed().as_millis()
            );
        }
        let env = self.global_env.clone();
        self.hoist(&stmts, &env);
        let mut last = Value::Undefined;
        for stmt in &stmts {
            if let Stmt::Expr(e) = stmt {
                last = self.eval_expr(e, &env)?;
            } else {
                let c = self.exec_stmt(stmt, &env)?;
                if c == Completion::Return {
                    break;
                }
            }
        }
        Ok(last)
    }

    /// Direct-eval entry used by the global `eval` binding. Source runs in
    /// the global environment, so functions and vars it defines are visible to
    /// subsequent scripts, matching classic-script eval semantics closely
    /// enough for page loaders and instrumentation shims.
    pub fn eval_source(&mut self, src: &str) -> Result<Value, JsError> {
        let stmts = crate::parser::parse_program(src).map_err(|e| {
            let mut err = JsError::syntax_error(e.message);
            err.stack.push(format!("at line {}", e.line));
            err
        })?;
        let env = self.global_env.clone();
        self.hoist(&stmts, &env);
        let mut last = Value::Undefined;
        for stmt in &stmts {
            if let Stmt::Expr(e) = stmt {
                last = self.eval_expr(e, &env)?;
            } else {
                let c = self.exec_stmt(stmt, &env)?;
                if c == Completion::Return {
                    break;
                }
            }
        }
        Ok(last)
    }

    /// Evaluates a parsed module body in a fresh module scope. The host passes
    /// resolved import bindings and receives the module's exported values.
    pub fn eval_module(
        &mut self,
        module: &crate::ast::ModuleAst,
        imports: &HashMap<String, Value>,
        url: &str,
    ) -> Result<HashMap<String, Value>, JsError> {
        self.begin_turn();
        let env = Env::new(Some(self.global_env.clone()), Value::Object(self.global.clone()), true);
        env.borrow_mut().vars.insert(
            "__module_url__".into(),
            Binding { value: Value::string(url.to_string()), mutable: false },
        );
        for (name, value) in imports {
            env.borrow_mut().vars.insert(
                name.clone(),
                Binding { value: value.clone(), mutable: false },
            );
        }
        self.hoist(&module.body, &env);
        for stmt in &module.body {
            let completion = self.exec_stmt(stmt, &env)?;
            if completion == Completion::Return {
                break;
            }
        }
        let mut out = HashMap::new();
        for (export, local) in &module.exports {
            if let Some(value) = lookup(&env, local) {
                out.insert(export.clone(), value);
            }
        }
        Ok(out)
    }

    pub fn take_pending_imports(&mut self) -> Vec<(String, Value, Value)> {
        std::mem::take(&mut self.pending_imports)
    }

    pub fn eval_expression(&mut self, src: &str) -> Result<Value, JsError> {
        self.begin_turn();
        let expr = crate::parser::parse_expression(src)
            .map_err(|e| JsError::syntax_error(e.message))?;
        let env = self.global_env.clone();
        self.eval_expr(&expr, &env)
    }

    pub fn set_global(&mut self, name: &str, value: Value) {
        define(&self.global, name, value.clone());
        self.global_env
            .borrow_mut()
            .vars
            .insert(name.to_string(), Binding { value, mutable: true });
    }

    pub fn get_global(&mut self, name: &str) -> Option<Value> {
        lookup(&self.global_env, name).or_else(|| data_prop(&self.global, name))
    }

    pub fn global_object(&self) -> ObjRef {
        self.global.clone()
    }

    pub fn set_now(&mut self, now_ms: f64) {
        self.now = now_ms;
    }

    pub fn native<F>(&self, name: &str, length: usize, f: F) -> Value
    where
        F: Fn(&mut Interp, Value, Vec<Value>) -> Result<Value, JsError> + 'static,
    {
        let func: Rc<NativeImpl> = Rc::new(f);
        make_native_value(name, length, Some(self.intrinsics.function_proto.clone()), func)
    }

    pub fn array_from(&self, values: Vec<Value>) -> Value {
        Value::Object(new_array(values, self.intrinsics.array_proto.clone()))
    }

    pub fn array_with_holes(&self, values: Vec<Value>, holes: Vec<bool>) -> Value {
        Value::Object(new_array_with_holes(values, holes, self.intrinsics.array_proto.clone()))
    }

    pub fn object(&self) -> Value {
        Value::Object(new_object(Some(self.intrinsics.object_proto.clone())))
    }

    pub fn make_error(&self, kind: &str, message: &str) -> Value {
        let proto = match kind {
            "TypeError" => self.intrinsics.type_error_proto.clone(),
            "RangeError" => self.intrinsics.range_error_proto.clone(),
            "SyntaxError" => self.intrinsics.syntax_error_proto.clone(),
            "ReferenceError" => self.intrinsics.reference_error_proto.clone(),
            _ => self.intrinsics.error_proto.clone(),
        };
        let obj = new_object(Some(proto));
        obj.borrow_mut().kind = ObjKind::Error { name: kind.to_string() };
        define(&obj, "message", Value::string(message.to_string()));
        define(&obj, "name", Value::string(kind.to_string()));
        let mut frames: Vec<String> = self.stack.iter().rev().map(|n| format!("    at {n}")).collect();
        if self.current_line > 0 {
            frames.push(format!("    at line {}", self.current_line));
        }
        define(
            &obj,
            "stack",
            Value::string(format!("{kind}: {message}\n{}", frames.join("\n"))),
        );
        Value::Object(obj)
    }

    pub fn error_value(&mut self, err: &JsError) -> Value {
        if let Some(v) = &err.value {
            return v.clone();
        }
        let previous = self.current_line;
        if err.line > 0 {
            self.current_line = err.line;
        }
        let value = self.make_error(err.kind, &err.message);
        self.current_line = previous;
        value
    }

    pub fn run_microtasks(&mut self) -> Result<(), JsError> {
        self.begin_turn();
        let mut count = 0;
        while let Some(job) = self.microtasks.pop_front() {
            job(self)?;
            self.check_turn()?;
            count += 1;
            if count > 100_000 {
                return Err(JsError::range_error("microtask loop exceeded limit"));
            }
        }
        Ok(())
    }

    fn pump_until_settled(&mut self, data: &Rc<RefCell<PromiseData>>) -> Result<(), JsError> {
        for step in 0..8 {
            self.run_microtasks()?;
            if data.borrow().state != PromiseState::Pending {
                return Ok(());
            }
            let next = self.now + (step as f64) + 1.0;
            self.run_due_timers(next)?;
            self.run_microtasks()?;
            if data.borrow().state != PromiseState::Pending {
                return Ok(());
            }
        }
        Ok(())
    }

    pub fn pending_tasks(&self) -> usize {
        self.microtasks.len() + self.timers.len()
    }

    pub fn run_due_timers(&mut self, now_ms: f64) -> Result<usize, JsError> {
        self.begin_turn();
        self.now = now_ms;
        let mut ran = 0;
        loop {
            let mut due: Vec<(usize, f64)> = self
                .timers
                .iter()
                .enumerate()
                .filter(|(_, t)| t.due <= now_ms)
                .map(|(i, t)| (i, t.due))
                .collect();
            if due.is_empty() {
                break;
            }
            due.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let timer = self.timers.remove(due[0].0);
            if let Some(period) = timer.repeat {
                self.timers.push(Timer {
                    id: timer.id,
                    due: now_ms + period.max(1.0),
                    repeat: timer.repeat,
                    callback: timer.callback.clone(),
                    args: timer.args.clone(),
                });
            }
            match &timer.callback {
                Value::String(src) => {
                    self.eval_script(src)?;
                }
                cb => {
                    self.call_value(cb, Value::Undefined, timer.args)?;
                }
            }
            self.run_microtasks()?;
            ran += 1;
            if ran > 1000 {
                break;
            }
        }
        Ok(ran)
    }

    pub fn run_timers_until_idle(&mut self, step_ms: f64, max_ms: f64) -> Result<(), JsError> {
        let mut t = self.now;
        let end = t + max_ms;
        while !self.timers.is_empty() && t < end {
            t += step_ms;
            self.run_due_timers(t)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Functions and calls
    // ------------------------------------------------------------------

    fn function_object(&self, f: Func) -> Value {
        let obj = new_object(Some(self.intrinsics.function_proto.clone()));
        obj.borrow_mut().kind = ObjKind::Function(f);
        Value::Object(obj)
    }

    pub fn make_user_function(
        &self,
        node: Rc<FnNode>,
        closure: EnvRef,
        is_arrow: bool,
        home: Option<ObjRef>,
    ) -> Value {
        let uf = Rc::new(UserFunction {
            node,
            source: self.current_source.clone(),
            closure,
            home,
            parent_ctor: None,
            super_proto: None,
            fields: Vec::new(),
            is_class_ctor: false,
            derived: false,
        });
        let value = self.function_object(Func::User(uf.clone()));
        if !is_arrow {
            let proto = new_object(Some(self.intrinsics.object_proto.clone()));
            define(&proto, "constructor", value.clone());
            if let Some(obj) = value.as_object() {
                define(obj, "prototype", Value::Object(proto));
            }
        }
        value
    }

    pub fn call_value(&mut self, f: &Value, this: Value, args: Vec<Value>) -> Result<Value, JsError> {
        let func = match f.as_object() {
            Some(o) => match &o.borrow().kind {
                ObjKind::Function(func) => func.clone(),
                _ => return Err(JsError::type_error("value is not a function")),
            },
            None => return Err(JsError::type_error("value is not a function")),
        };
        self.steps += 1;
        if self.steps & 1023 == 0 {
            self.check_turn()?;
        }
        if self.steps > 3_000_000_000 {
            if std::env::var("VEYRA_DEBUG_STEPS").is_ok() {
                eprintln!("STEP-LIMIT stack={:?}", self.stack);
            }
            return Err(JsError::range_error("script exceeded its step limit"));
        }
        if self.depth > 500 {
            return Err(JsError::range_error("maximum call stack size exceeded"));
        }
        if let Func::User(u) = &func {
            if u.is_class_ctor && !self.constructing && !self.allow_class_call {
                let name = u.node.name.clone().unwrap_or_else(|| "class".into());
                return Err(JsError::type_error(format!(
                    "class constructor {name} cannot be invoked without 'new' (decl line {} call {:?})",
                    u.node.line,
                    self.stack.last().cloned().unwrap_or_default()
                )));
            }
        }
        match func {
            Func::Native(n) => {
                self.depth += 1;
                let result = (n.f)(self, this, args);
                self.depth -= 1;
                result
            }
            Func::User(u) => {
                let name = u.node.name.clone().unwrap_or_else(|| "anonymous".into());
                self.depth += 1;
                let where_ = match &u.source {
                    Some(src) => format!("{name}@{}[{src}]", u.node.line),
                    None => format!("{name}@{}", u.node.line),
                };
                self.stack.push(where_.clone());
                self.user_stack.push(u.clone());
                let new_target = if u.node.is_arrow {
                    self.new_targets.last().cloned().flatten()
                } else {
                    self.pending_new_target.take()
                };
                self.new_targets.push(new_target);
                let mut result = self.call_user(&u, this, args, f.clone());
                self.new_targets.pop();
                self.user_stack.pop();
                self.stack.pop();
                self.depth -= 1;
                if let Err(e) = &mut result {
                    if e.stack.len() < 32 {
                        e.stack.push(format!("at {name}:{}[{}]", u.node.line, u.source.clone().unwrap_or_default()));
                    }
                }
                result
            }
        }
    }

    /// Starts an async function. The body runs synchronously up to its first
    /// pending `await`, exactly as JavaScript requires. Suspension unwinds the
    /// Rust stack with a `__Suspend` marker; when the awaited promise settles,
    /// the body is resumed from its start with that promise now already
    /// fulfilled. Resolved awaits therefore return their value and later
    /// pending awaits suspend again, preserving bootstrap ordering without a
    /// separate coroutine stack.
    fn call_user(
        &mut self,
        f: &Rc<UserFunction>,
        this_arg: Value,
        args: Vec<Value>,
        self_value: Value,
    ) -> Result<Value, JsError> {
        if !f.node.is_async {
            return self.run_user_body(f, this_arg, args, self_value);
        }
        let (promise, data) = self.new_promise();
        self.resume_async(f.clone(), this_arg, args, self_value, data);
        Ok(promise)
    }

    fn resume_async(
        &mut self,
        f: Rc<UserFunction>,
        this_arg: Value,
        args: Vec<Value>,
        self_value: Value,
        data: Rc<RefCell<PromiseData>>,
    ) {
        self.async_suspend = None;
        match self.run_user_body(&f, this_arg.clone(), args.clone(), self_value.clone()) {
            Ok(v) => {
                self.async_suspend = None;
                self.resolve_promise(&data, v);
            }
            Err(e) => {
                let awaited = self.async_suspend.take();
                let Some(awaited) = awaited else {
                    self.reject_promise(&data, e.value.unwrap_or(Value::Undefined));
                    return;
                };
                let Some(pd) = awaited.as_object().and_then(|o| match &o.borrow().kind {
                    ObjKind::Promise(p) => Some(p.clone()),
                    _ => None,
                }) else {
                    self.reject_promise(&data, Value::Undefined);
                    return;
                };
                let cb = self.native("asyncResume", 0, {
                    let f = f.clone();
                    let this_arg = this_arg.clone();
                    let args = args.clone();
                    let self_value = self_value.clone();
                    let data = data.clone();
                    move |it: &mut Interp, _this, _args| {
                        it.resume_async(f.clone(), this_arg.clone(), args.clone(), self_value.clone(), data.clone());
                        Ok(Value::Undefined)
                    }
                });
                let reject_cb = self.native("asyncResumeRejected", 0, {
                    let f = f.clone();
                    let this_arg = this_arg.clone();
                    let args = args.clone();
                    let self_value = self_value.clone();
                    let data = data.clone();
                    move |it: &mut Interp, _this, _args| {
                        it.resume_async(f.clone(), this_arg.clone(), args.clone(), self_value.clone(), data.clone());
                        Ok(Value::Undefined)
                    }
                });
                self.promise_then(&pd, cb, reject_cb);
            }
        }
    }

    fn run_user_body(
        &mut self,
        f: &Rc<UserFunction>,
        this_arg: Value,
        args: Vec<Value>,
        self_value: Value,
    ) -> Result<Value, JsError> {
        let this = if f.node.is_arrow {
            env_this(&f.closure)
        } else if this_arg.is_nullish() {
            Value::Object(self.global.clone())
        } else {
            this_arg
        };
        let env = Env::new(Some(f.closure.clone()), this.clone(), true);
        if f.node.self_bind {
            if let Some(name) = &f.node.name {
                env.borrow_mut().vars.insert(
                    name.clone(),
                    Binding { value: self_value, mutable: false },
                );
            }
        }
        if let Some(home) = &f.home {
            env.borrow_mut().home = Some(home.clone());
            env.borrow_mut()
                .vars
                .insert("__home__".into(), Binding { value: Value::Object(home.clone()), mutable: false });
        }
        if let Some(parent) = &f.parent_ctor {
            env.borrow_mut().vars.insert(
                "__super_ctor__".into(),
                Binding { value: parent.clone(), mutable: false },
            );
        }
        // Bind parameters.
        let mut arg_i = 0usize;
        for p in &f.node.params {
            if let Pattern::Rest(inner) = p {
                let rest = self.array_from(args.iter().skip(arg_i).cloned().collect());
                self.bind_pattern(inner, rest, &env, true)?;
                arg_i = args.len();
                continue;
            }
            let val = args.get(arg_i).cloned().unwrap_or(Value::Undefined);
            arg_i += 1;
            self.bind_pattern(p, val, &env, true)?;
        }
        let arguments = new_array_like(args.clone(), self.intrinsics.object_proto.clone());
        define(&arguments, "callee", Value::Undefined);
        env.borrow_mut()
            .vars
            .insert("arguments".into(), Binding { value: Value::Object(arguments), mutable: false });

        // Class fields and implicit parent construction.
        if f.is_class_ctor {
            if f.derived && f.fields.is_empty() {
                if let Some(parent) = &f.parent_ctor {
                    let implicit = matches!(&f.node.body, FnBody::Block(b) if b.is_empty());
                    if implicit {
                        let parent_result = self.call_value(parent, this.clone(), args.clone())?;
                        if parent_result.is_object() {
                            env.borrow_mut().this = parent_result;
                        }
                    }
                }
            }
            let this_now = env_this(&env);
            if let Some(obj) = this_now.as_object() {
                for (name, init) in &f.fields {
                    let value = match init {
                        Some(e) => self.eval_expr(e, &env)?,
                        None => Value::Undefined,
                    };
                    define(obj, name, value);
                }
            }
        }

        if f.node.is_generator {
            self.yield_stack.push(Vec::new());
        }
        let mut returned: Option<Value> = None;
        if let FnBody::Block(body) = &f.node.body {
            let body = body.clone();
            self.hoist(&body, &env);
            for stmt in &body {
                match self.exec_stmt(stmt, &env)? {
                    Completion::Normal => {}
                    Completion::Return => {
                        returned = Some(lookup(&env, "__return__").unwrap_or(Value::Undefined));
                        break;
                    }
                    Completion::Break | Completion::Continue => {}
                }
            }
        } else if let FnBody::Expr(e) = &f.node.body {
            returned = Some(self.eval_expr(e, &env)?);
        }
        if f.node.is_generator {
            let values = self.yield_stack.pop().unwrap_or_default();
            let array = self.array_from(values);
            return Ok(array);
        }
        let value = match returned {
            Some(v) => v,
            None if f.is_class_ctor => env_this(&env),
            None => Value::Undefined,
        };
        Ok(value)
    }

    pub fn construct(&mut self, ctor: &Value, args: Vec<Value>) -> Result<Value, JsError> {
        self.construct_with_new_target(ctor, args, None)
    }

    /// `Reflect.construct` can construct with a different `new.target`; the
    /// resulting object inherits from that target's `prototype`, not from the
    /// constructor's own prototype. Polymer's fast-init path relies on this to
    /// borrow another element's prototype while running its constructor.
    pub fn construct_with_new_target(
        &mut self,
        ctor: &Value,
        args: Vec<Value>,
        new_target: Option<Value>,
    ) -> Result<Value, JsError> {
        let obj = ctor.as_object().ok_or_else(|| JsError::type_error("not a constructor"))?;
        let func = match &obj.borrow().kind {
            ObjKind::Function(f) => f.clone(),
            _ => return Err(JsError::type_error("not a constructor")),
        };
        let proto = new_target
            .as_ref()
            .and_then(|v| v.as_object().cloned())
            .and_then(|o| data_prop(&o, "prototype"))
            .and_then(|v| v.as_object().cloned())
            .or_else(|| data_prop(obj, "prototype").and_then(|v| v.as_object().cloned()))
            .unwrap_or_else(|| self.intrinsics.object_proto.clone());
        match func {
            Func::User(u) => {
                if u.node.is_arrow {
                    return Err(JsError::type_error("arrow functions are not constructors"));
                }
                let instance = new_object(Some(proto.clone()));
                if find_prop(&proto, "__data").is_some() && data_prop(&instance, "__data").is_none() {
                    define(&instance, "__data", Value::Object(new_object(None)));
                }
                let this = Value::Object(instance.clone());
                let saved = self.constructing;
                self.constructing = true;
                self.pending_new_target = Some(new_target.clone().unwrap_or_else(|| ctor.clone()));
                let result = self.call_value(ctor, this, args);
                self.pending_new_target = None;
                self.constructing = saved;
                let result = result?;
                if result.is_object() {
                    Ok(result)
                } else {
                    Ok(Value::Object(instance))
                }
            }
            Func::Native(_) => {
                let instance = new_object(Some(proto.clone()));
                if find_prop(&proto, "__data").is_some() && data_prop(&instance, "__data").is_none() {
                    define(&instance, "__data", Value::Object(new_object(None)));
                }
                let this = Value::Object(instance.clone());
                let saved = self.constructing;
                self.constructing = true;
                let result = self.call_value(ctor, this, args);
                self.constructing = saved;
                let result = result?;
                if result.is_object() {
                    Ok(result)
                } else {
                    Ok(Value::Object(instance))
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Property access
    // ------------------------------------------------------------------

    pub fn get_member(&mut self, base: Value, key: &str) -> Result<Value, JsError> {
        match &base {
            Value::Undefined | Value::Null => {
                if std::env::var_os("VEYRA_TRACE_UNDEF").is_some() {
                    let frame = self.user_stack.last().map(|u| {
                        format!("name={:?} line={} source={:?} body={:?}", u.node.name, u.node.line, u.source, u.node.body)
                    });
                    eprintln!("UNDEF key={key} jsstack={:?} frame={}", self.stack, frame.unwrap_or_else(|| "<none>".into()));
                }
                Err(JsError::type_error(format!(
                    "cannot read properties of {} (reading '{key}')",
                    if base.is_null() { "null" } else { "undefined" }
                )))
            }
            Value::Object(o) => self.get_from_object(o, key, &base),
            Value::String(s) => {
                if key == "length" {
                    return Ok(Value::Number(s.chars().count() as f64));
                }
                if let Some(i) = parse_index(key) {
                    if let Some(c) = s.chars().nth(i) {
                        return Ok(Value::string(c.to_string()));
                    }
                }
                self.get_from_object(&self.intrinsics.string_proto.clone(), key, &base)
            }
            Value::Number(_) => {
                self.get_from_object(&self.intrinsics.number_proto.clone(), key, &base)
            }
            Value::Bool(_) => {
                self.get_from_object(&self.intrinsics.boolean_proto.clone(), key, &base)
            }
            Value::BigInt(_) => {
                self.get_from_object(&self.intrinsics.object_proto.clone(), key, &base)
            }
        }
    }

    pub fn get_from_object(&mut self, obj: &ObjRef, key: &str, receiver: &Value) -> Result<Value, JsError> {
        let proxy = obj.borrow().proxy.clone();
        if let Some((target, handler)) = proxy {
            let trap = self.get_from_object(&handler, "get", &Value::Object(handler.clone()))?;
            if trap.is_callable() {
                return self.call_value(
                    &trap,
                    Value::Object(handler),
                    vec![Value::Object(target), Value::string(key), receiver.clone()],
                );
            }
            let receiver = Value::Object(target.clone());
            return self.get_from_object(&target, key, &receiver);
        }
        if let Some(prop) = own_prop(obj, key) {
            if let Some(getter) = prop.get {
                return self.call_value(&getter, receiver.clone(), vec![]);
            }
            return Ok(prop.value);
        }
        // Polymer's property accessors read `this.__data`. When one is invoked
        // on the class prototype during finalisation there is no instance data
        // store yet; expose an empty one rather than failing on the prototype.
        if key == "__data" {
            if let Some(Value::Object(ctor)) = own_prop(obj, "constructor").map(|p| p.value) {
                if let Some(prototype) = data_prop(&ctor, "prototype").and_then(|v| v.as_object().cloned()) {
                    if Rc::ptr_eq(&prototype, obj) {
                        return Ok(Value::Object(new_object(None)));
                    }
                }
            }
        }
        let mut cur = obj.borrow().proto.clone();
        let mut depth = 0;
        while let Some(proto) = cur {
            let proxy = proto.borrow().proxy.clone();
            if let Some((target, handler)) = proxy {
                let trap = self.get_from_object(&handler, "get", &Value::Object(handler.clone()))?;
                if trap.is_callable() {
                    return self.call_value(
                        &trap,
                        Value::Object(handler),
                        vec![Value::Object(target), Value::string(key), receiver.clone()],
                    );
                }
                cur = Some(target);
                depth += 1;
                if depth > 200 {
                    break;
                }
                continue;
            }
            if let Some(prop) = own_prop(&proto, key) {
                if let Some(getter) = prop.get {
                    return self.call_value(&getter, receiver.clone(), vec![]);
                }
                return Ok(prop.value);
            }
            cur = proto.borrow().proto.clone();
            depth += 1;
            if depth > 200 {
                break;
            }
        }
        Ok(Value::Undefined)
    }

    pub fn set_member(&mut self, base: Value, key: &str, value: Value) -> Result<(), JsError> {
        let obj = match &base {
            Value::Object(o) => o.clone(),
            Value::Undefined | Value::Null => {
                return Err(JsError::type_error(format!("cannot set properties of {} (setting '{key}')", if base.is_null() { "null" } else { "undefined" })))
            }
            _ => return Ok(()),
        };
        // Assignments to `window.foo` are global variable assignments. Keep the
        // environment binding in sync so a later unqualified `foo` observes the
        // value (ShadyDOM relies on this when it replaces ShadowRoot).
        if Rc::ptr_eq(&obj, &self.global) {
            self.global_env
                .borrow_mut()
                .vars
                .insert(key.to_string(), Binding { value: value.clone(), mutable: true });
        }
        let proxy = obj.borrow().proxy.clone();
        if let Some((target, handler)) = proxy {
            let trap = self.get_from_object(&handler, "set", &Value::Object(handler.clone()))?;
            if trap.is_callable() {
                self.call_value(
                    &trap,
                    Value::Object(handler),
                    vec![Value::Object(target), Value::string(key), value, base.clone()],
                )?;
                return Ok(());
            }
            return self.set_member(Value::Object(target), key, value);
        }
        // Accessor or non-writable inherited property?
        let mut cur = Some(obj.clone());
        let mut depth = 0;
        while let Some(o) = cur {
            // A Proxy in the prototype chain receives [[Set]] with the original
            // receiver. Polymer's fast-init constructs controllers with a proxy
            // new.target precisely so constructor field writes land on the
            // controller; without this the writes stay on the temporary object.
            let inherited_proxy = o.borrow().proxy.clone();
            if let Some((target, handler)) = inherited_proxy {
                let trap = self.get_from_object(&handler, "set", &Value::Object(handler.clone()))?;
                if trap.is_callable() {
                    self.call_value(
                        &trap,
                        Value::Object(handler),
                        vec![
                            Value::Object(target),
                            Value::string(key),
                            value,
                            base.clone(),
                        ],
                    )?;
                    return Ok(());
                }
            }
            if let Some(p) = own_prop(&o, key) {
                if let Some(setter) = p.set {
                    self.call_value(&setter, base.clone(), vec![value])?;
                    return Ok(());
                }
                if Rc::ptr_eq(&o, &obj) {
                    if p.writable || p.get.is_some() {
                        define_with(&obj, key, value, true, p.enumerable, p.configurable);
                        return Ok(());
                    }
                    return Ok(());
                }
                if !p.writable && p.get.is_none() {
                    return Ok(());
                }
                break;
            }
            let next = o.borrow().proto.clone();
            cur = next;
            depth += 1;
            if depth > 100 {
                break;
            }
        }
        if !obj.borrow().extensible {
            return Ok(());
        }
        define(&obj, key, value);
        Ok(())
    }

    pub fn delete_member(&mut self, base: &Value, key: &str) -> bool {
        match base.as_object() {
            Some(o) => delete_prop(o, key),
            None => true,
        }
    }

    // ------------------------------------------------------------------
    // Conversions
    // ------------------------------------------------------------------

    pub fn to_primitive(&mut self, value: &Value, hint_string: bool) -> Result<Value, JsError> {
        if !value.is_object() {
            return Ok(value.clone());
        }
        let order: [&str; 2] = if hint_string {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        for name in order {
            let method = self.get_member(value.clone(), name)?;
            if method.is_callable() {
                let r = self.call_value(&method, value.clone(), vec![])?;
                if !r.is_object() {
                    return Ok(r);
                }
            }
        }
        Err(JsError::type_error("cannot convert object to primitive value"))
    }

    pub fn to_string(&mut self, value: &Value) -> Result<String, JsError> {
        match value {
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::BigInt(_)
            | Value::String(_) => {
                Ok(value.to_string_primitive())
            }
            Value::Object(o) => {
                let kind_tag = {
                    let o = o.borrow();
                    match &o.kind {
                        ObjKind::Array(_) => "array",
                        ObjKind::Error { .. } => "error",
                        ObjKind::Date(_) => "date",
                        ObjKind::RegExp { .. } => "regexp",
                        ObjKind::Function(_) => "function",
                        ObjKind::Map(_) => "map",
                        ObjKind::Set(_) => "set",
                        _ => "object",
                    }
                };
                match kind_tag {
                    "array" => {
                        let values = array_values(value).unwrap_or_default();
                        let mut parts = Vec::new();
                        for v in values {
                            if v.is_nullish() {
                                parts.push(String::new());
                            } else {
                                parts.push(self.to_string(&v)?);
                            }
                        }
                        return Ok(parts.join(","));
                    }
                    "date" => {
                        let ms = match &o.borrow().kind {
                            ObjKind::Date(ms) => *ms,
                            _ => 0.0,
                        };
                        return Ok(format_date(ms));
                    }
                    "error" => {
                        let (name, msg) = {
                            let o = o.borrow();
                            let name = match &o.kind {
                                ObjKind::Error { name } => name.clone(),
                                _ => "Error".into(),
                            };
                            let msg = o
                                .props
                                .get("message")
                                .map(|p| self.to_string_primitive_of(&p.value))
                                .unwrap_or_default();
                            (name, msg)
                        };
                        return Ok(if msg.is_empty() { name } else { format!("{name}: {msg}") });
                    }
                    "regexp" => {
                        let o = o.borrow();
                        if let ObjKind::RegExp { source, flags, .. } = &o.kind {
                            return Ok(format!("/{source}/{flags}"));
                        }
                    }
                    "function" => {
                        let name = match &o.borrow().kind {
                            ObjKind::Function(f) => f.name(),
                            _ => String::new(),
                        };
                        return Ok(format!("function {name}() {{ [native code] }}"));
                    }
                    "map" => return Ok("[object Map]".into()),
                    "set" => return Ok("[object Set]".into()),
                    _ => {}
                }
                let method = self.get_member(value.clone(), "toString")?;
                if method.is_callable() {
                    let r = self.call_value(&method, value.clone(), vec![])?;
                    if !r.is_object() {
                        return self.to_string(&r);
                    }
                }
                Ok("[object Object]".into())
            }
        }
    }

    fn to_string_primitive_of(&self, v: &Value) -> String {
        v.to_string_primitive()
    }

    pub fn to_number_value(&mut self, value: &Value) -> Result<f64, JsError> {
        if value.is_object() {
            let p = self.to_primitive(value, false)?;
            Ok(p.to_number())
        } else {
            Ok(value.to_number())
        }
    }

    pub fn to_property_key(&mut self, value: &Value) -> Result<String, JsError> {
        match value {
            Value::Number(n) => Ok(number_to_string(*n)),
            Value::BigInt(n) => Ok(n.to_string()),
            Value::String(s) => Ok(s.to_string()),
            Value::Bool(b) => Ok(b.to_string()),
            Value::Null => Ok("null".into()),
            Value::Undefined => Ok("undefined".into()),
            Value::Object(_) => self.to_string(value),
        }
    }

    pub fn truthy(&mut self, value: &Value) -> bool {
        value.truthy()
    }

    // ------------------------------------------------------------------
    // Iteration
    // ------------------------------------------------------------------

    pub fn iterate(&mut self, value: &Value) -> Result<Vec<Value>, JsError> {
        match value {
            Value::String(s) => Ok(s.chars().map(|c| Value::string(c.to_string())).collect()),
            Value::Object(o) => {
                let o = o.borrow();
                match &o.kind {
                    ObjKind::Array(values) => Ok(values.clone()),
                    ObjKind::Set(values) => Ok(values.clone()),
                    ObjKind::Map(entries) => Ok(entries
                        .iter()
                        .map(|(k, v)| self.array_from(vec![k.clone(), v.clone()]))
                        .collect()),
                    ObjKind::Arguments => {
                        let len = o
                            .props
                            .get("length")
                            .map(|p| p.value.to_index())
                            .unwrap_or(0);
                        let mut out = Vec::new();
                        for i in 0..len {
                            out.push(o.props.get(&i.to_string()).map(|p| p.value.clone()).unwrap_or(Value::Undefined));
                        }
                        Ok(out)
                    }
                    _ => {
                        drop(o);
                        let sym = self.get_member(value.clone(), "@@symbol:Symbol.iterator")?;
                        if !sym.is_nullish() {
                            let iter = self.call_value(&sym, value.clone(), Vec::new())?;
                            if let Some(values) = array_values(&iter) {
                                return Ok(values);
                            }
                            if iter.is_object() {
                                let next = self.get_member(iter.clone(), "next")?;
                                if next.is_callable() {
                                    let mut out = Vec::new();
                                    for _ in 0..10_000_000 {
                                        let step = self.call_value(&next, iter.clone(), Vec::new())?;
                                        let done = self.get_member(step.clone(), "done")?.truthy();
                                        if done {
                                            return Ok(out);
                                        }
                                        out.push(self.get_member(step, "value")?);
                                    }
                                }
                            }
                        }
                        Err(JsError::type_error("value is not iterable"))
                    }
                }
            }
            _ => Err(JsError::type_error("value is not iterable")),
        }
    }

    pub fn enumerable_keys(&self, value: &Value) -> Vec<String> {
        let obj = match value.as_object() {
            Some(o) => o.clone(),
            None => return Vec::new(),
        };
        // Built-in prototype methods are represented with enumerable flags
        // that differ from ECMAScript's non-enumerable defaults, so for-in
        // over own enumerable properties only avoids leaking methods such as
        // `toLocaleString` into application data.
        own_keys(&obj)
    }

    // ------------------------------------------------------------------
    // Promises
    // ------------------------------------------------------------------

    pub fn new_promise(&self) -> (Value, Rc<RefCell<PromiseData>>) {
        let data = Rc::new(RefCell::new(PromiseData {
            state: PromiseState::Pending,
            value: Value::Undefined,
            reactions: Vec::new(),
        }));
        let obj = new_object(Some(self.intrinsics.promise_proto.clone()));
        obj.borrow_mut().kind = ObjKind::Promise(data.clone());
        (Value::Object(obj), data)
    }

    pub fn resolve_promise(&mut self, data: &Rc<RefCell<PromiseData>>, value: Value) {
        // Adopt another promise.
        if let Some(obj) = value.as_object() {
            let adopted = match &obj.borrow().kind {
                ObjKind::Promise(p) => Some(p.clone()),
                _ => None,
            };
            if let Some(p) = adopted {
                let on_f = self.native("", 1, {
                    let data = data.clone();
                    move |interp, _this, args| {
                        interp.resolve_promise(&data, args.first().cloned().unwrap_or(Value::Undefined));
                        Ok(Value::Undefined)
                    }
                });
                let on_r = self.native("", 1, {
                    let data = data.clone();
                    move |interp, _this, args| {
                        interp.reject_promise(&data, args.first().cloned().unwrap_or(Value::Undefined));
                        Ok(Value::Undefined)
                    }
                });
                let _ = self.promise_then(&p, on_f, on_r);
                return;
            }
        }
        self.settle_promise(data, PromiseState::Fulfilled, value);
    }

    pub fn reject_promise(&mut self, data: &Rc<RefCell<PromiseData>>, reason: Value) {
        self.settle_promise(data, PromiseState::Rejected, reason);
    }

    fn settle_promise(&mut self, data: &Rc<RefCell<PromiseData>>, state: PromiseState, value: Value) {
        {
            let mut d = data.borrow_mut();
            if d.state != PromiseState::Pending {
                return;
            }
            d.state = state;
            d.value = value.clone();
        }
        let reactions = {
            let mut d = data.borrow_mut();
            std::mem::take(&mut d.reactions)
        };
        for r in reactions {
            self.schedule_reaction(state, value.clone(), r);
        }
    }

    fn schedule_reaction(&mut self, state: PromiseState, value: Value, r: PromiseReaction) {
        let handler = if state == PromiseState::Fulfilled {
            r.on_fulfilled.clone()
        } else {
            r.on_rejected.clone()
        };
        let result = r.result.clone();
        self.microtasks.push_back(Box::new(move |interp: &mut Interp| {
            match handler {
                Some(h) if h.is_callable() => match interp.call_value(&h, Value::Undefined, vec![value]) {
                    Ok(v) => {
                        interp.resolve_promise(&result, v);
                    }
                    Err(e) => {
                        let err_value = interp.error_value(&e);
                        interp.reject_promise(&result, err_value);
                    }
                },
                _ => {
                    if state == PromiseState::Fulfilled {
                        interp.resolve_promise(&result, value);
                    } else {
                        interp.reject_promise(&result, value);
                    }
                }
            }
            Ok(())
        }));
    }

    pub fn promise_then(
        &mut self,
        data: &Rc<RefCell<PromiseData>>,
        on_fulfilled: Value,
        on_rejected: Value,
    ) -> Value {
        let (result_value, result_data) = self.new_promise();
        let reaction = PromiseReaction {
            on_fulfilled: if on_fulfilled.is_callable() { Some(on_fulfilled) } else { None },
            on_rejected: if on_rejected.is_callable() { Some(on_rejected) } else { None },
            result: result_data,
        };
        let (state, value) = {
            let d = data.borrow();
            (d.state, d.value.clone())
        };
        if state == PromiseState::Pending {
            data.borrow_mut().reactions.push(reaction);
        } else {
            self.schedule_reaction(state, value, reaction);
        }
        result_value
    }

    // ------------------------------------------------------------------
    // Classes
    // ------------------------------------------------------------------

    fn eval_class(&mut self, node: &Rc<ClassNode>, env: &EnvRef) -> Result<Value, JsError> {
        let parent = match &node.super_class {
            Some(e) => Some(self.eval_expr(e, env)?),
            None => None,
        };
        let parent_proto = parent
            .as_ref()
            .and_then(|p| p.as_object())
            .and_then(|o| data_prop(o, "prototype"))
            .and_then(|v| v.as_object().cloned());
        let proto = new_object(Some(
            parent_proto.clone().unwrap_or_else(|| self.intrinsics.object_proto.clone()),
        ));

        let ctor_method = node.members.iter().find(|m| {
            !m.is_static
                && matches!(m.kind, MemberKind::Method)
                && matches!(&m.key, PropKey::Static(k) if k == "constructor")
        });
        let ctor_node = match ctor_method.and_then(|m| m.value.as_ref()) {
            Some(Expr::Function(f)) => f.clone(),
            _ => Rc::new(empty_fn(node.name.clone())),
        };
        let mut fields = Vec::new();
        for m in &node.members {
            if matches!(m.kind, MemberKind::Field) && !m.is_static {
                if let PropKey::Static(name) = &m.key {
                    fields.push((name.clone(), m.value.clone()));
                }
            }
        }
        let uf = Rc::new(UserFunction {
            node: ctor_node,
            source: self.current_source.clone(),
            closure: env.clone(),
            home: Some(proto.clone()),
            parent_ctor: parent.clone(),
            super_proto: parent_proto,
            fields,
            is_class_ctor: true,
            derived: parent.is_some(),
        });
        let value = self.function_object(Func::User(uf));
        if let Some(obj) = value.as_object() {
            define(obj, "prototype", Value::Object(proto.clone()));
            if let Some(p) = &parent {
                if let Some(po) = p.as_object() {
                    obj.borrow_mut().proto = Some(po.clone());
                }
            }
            if let Some(name) = &node.name {
                define(obj, "name", Value::string(name.clone()));
            }
        }
        define_with(&proto, "constructor", value.clone(), true, false, true);
        self.install_members(node, env, &proto, &value)?;
        Ok(value)
    }

    fn install_members(
        &mut self,
        node: &Rc<ClassNode>,
        env: &EnvRef,
        proto: &ObjRef,
        ctor: &Value,
    ) -> Result<(), JsError> {
        for m in &node.members {
            if matches!(m.kind, MemberKind::StaticBlock) {
                if let Some(Expr::Function(f)) = &m.value {
                    let target = ctor.as_object().cloned().unwrap();
                    let func = self.make_user_function(f.clone(), env.clone(), false, Some(target));
                    self.call_value(&func, ctor.clone(), Vec::new())?;
                }
                continue;
            }
            if matches!(m.kind, MemberKind::Field) {
                if m.is_static {
                    let target = ctor.as_object().cloned().unwrap();
                    if let PropKey::Static(name) = &m.key {
                        let value = match &m.value {
                            Some(e) => self.eval_expr(e, env)?,
                            None => Value::Undefined,
                        };
                        define(&target, name, value);
                    }
                }
                continue;
            }
            if matches!(&m.key, PropKey::Static(k) if k == "constructor") && !m.is_static {
                continue;
            }
            let key = match &m.key {
                PropKey::Static(s) => s.clone(),
                PropKey::Computed(e) => {
                    let v = self.eval_expr(e, env)?;
                    self.to_property_key(&v)?
                }
            };
            let target = if m.is_static {
                ctor.as_object().cloned().unwrap()
            } else {
                proto.clone()
            };
            let home = if m.is_static { ctor.as_object().cloned() } else { Some(proto.clone()) };
            if let Some(Expr::Function(f)) = &m.value {
                let value = self.make_user_function(f.clone(), env.clone(), false, home);
                match m.kind {
                    MemberKind::Get => {
                        let setter = own_prop(&target, &key).and_then(|p| p.set);
                        define_accessor(&target, &key, Some(value), setter, false);
                    }
                    MemberKind::Set => {
                        let getter = own_prop(&target, &key).and_then(|p| p.get);
                        define_accessor(&target, &key, getter, Some(value), false);
                    }
                    _ => define_with(&target, &key, value, true, false, true),
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    pub fn hoist(&mut self, stmts: &[Stmt], env: &EnvRef) {
        for stmt in stmts {
            match stmt {
                Stmt::Var { kind: VarKind::Var, decls } => {
                    let mut names = Vec::new();
                    for (p, _) in decls {
                        pattern_names(p, &mut names);
                    }
                    for n in names {
                        let mut cur = Some(env.clone());
                        while let Some(e) = cur {
                            let is_fn = {
                                let b = e.borrow();
                                b.is_function || b.global.is_some()
                            };
                            if is_fn {
                                let mut frame = e.borrow_mut();
                                frame.vars.entry(n.clone()).or_insert(Binding {
                                    value: Value::Undefined,
                                    mutable: true,
                                });
                                // A global `var` is also a property of the global
                                // object (window.ytcfg etc.), not only a binding
                                // in the global environment record.
                                if let Some(global) = frame.global.clone() {
                                    let mut g = global.borrow_mut();
                                    if !g.props.contains_key(&n) {
                                        g.props.insert(n.clone(), Prop::data(Value::Undefined, true));
                                    }
                                }
                                break;
                            }
                            let p = e.borrow().parent.clone();
                            cur = p;
                        }
                    }
                }
                Stmt::Located { stmt, .. } => self.hoist(std::slice::from_ref(stmt), env),
                Stmt::FuncDecl(node) => {
                    let value = self.make_user_function(node.clone(), env.clone(), false, None);
                    let name = node.name.clone().unwrap_or_default();
                    let global = env.borrow().global.clone();
                    env.borrow_mut()
                        .vars
                        .insert(name.clone(), Binding { value: value.clone(), mutable: true });
                    if let Some(g) = global {
                        define(&g, &name, value);
                    }
                }
                Stmt::Block(body) => self.hoist(body, env),
                Stmt::If { cons, alt, .. } => {
                    self.hoist(std::slice::from_ref(cons), env);
                    if let Some(a) = alt {
                        self.hoist(std::slice::from_ref(a), env);
                    }
                }
                Stmt::For { init, body, .. } => {
                    if let Some(i) = init {
                        self.hoist(std::slice::from_ref(i), env);
                    }
                    self.hoist(std::slice::from_ref(body), env);
                }
                Stmt::ForIn { body, .. } | Stmt::While { body, .. } | Stmt::DoWhile { body, .. } => {
                    self.hoist(std::slice::from_ref(body), env)
                }
                Stmt::Try { body, catch, finally } => {
                    self.hoist(std::slice::from_ref(body), env);
                    if let Some((_, c)) = catch {
                        self.hoist(std::slice::from_ref(c), env);
                    }
                    if let Some(f) = finally {
                        self.hoist(std::slice::from_ref(f), env);
                    }
                }
                Stmt::Switch { cases, .. } => {
                    for (_, body) in cases {
                        self.hoist(body, env);
                    }
                }
                Stmt::Labeled { body, .. } => self.hoist(std::slice::from_ref(body), env),
                _ => {}
            }
        }
    }

    fn trace_steps(&self) {
        let frames: Vec<&str> = self.stack.iter().rev().take(8).map(|s| s.as_str()).collect();
        eprintln!(
            "steps={} elapsed={:.1}s stack={:?}",
            self.steps,
            self.started.elapsed().as_secs_f64(),
            frames
        );
    }

    fn exec_stmt(&mut self, stmt: &Stmt, env: &EnvRef) -> Result<Completion, JsError> {
        self.steps += 1;
        if self.steps & 1023 == 0 {
            self.check_turn()?;
        }
        if self.step_log && self.steps % 100_000_000 == 0 {
            self.trace_steps();
        }
        if self.steps > self.step_limit {
            if std::env::var("VEYRA_DEBUG_STEPS").is_ok() {
                eprintln!("STEP-LIMIT-CALL stack={:?}", self.stack);
            }
            return Err(JsError::range_error("script exceeded its step limit"));
        }
        match stmt {
            Stmt::Located { line, stmt } => {
                let previous = self.current_line;
                self.current_line = *line;
                let result = self.exec_stmt(stmt, env);
                self.current_line = previous;
                match result {
                    Err(mut err) => {
                        if err.line == 0 {
                            err.line = *line;
                        }
                        Err(err)
                    }
                    ok => ok,
                }
            }
            Stmt::Expr(e) => {
                self.eval_expr(e, env)?;
                Ok(Completion::Normal)
            }
            Stmt::Empty | Stmt::Debugger => Ok(Completion::Normal),
            Stmt::Block(body) => {
                let block = Env::new_block(env.clone());
                for s in body {
                    match self.exec_stmt(s, &block)? {
                        Completion::Normal => {}
                        other => return Ok(other),
                    }
                }
                Ok(Completion::Normal)
            }
            Stmt::Var { kind, decls } => {
                for (pat, init) in decls {
                    let value = match init {
                        Some(e) => self.eval_expr(e, env)?,
                        None => Value::Undefined,
                    };
                    if matches!(kind, VarKind::Var) && init.is_none() {
                        continue;
                    }
                    match kind {
                        VarKind::Var => self.bind_pattern(pat, value, env, false)?,
                        VarKind::Let => self.bind_declare(pat, value, env, true)?,
                        VarKind::Const => self.bind_declare(pat, value, env, false)?,
                    }
                }
                Ok(Completion::Normal)
            }
            Stmt::FuncDecl(_) => Ok(Completion::Normal),
            Stmt::ClassDecl(node) => {
                let name = node.name.clone().unwrap_or_default();
                let value = self.eval_class(node, env)?;
                if let Some(obj) = value.as_object() {
                    define(obj, "name", Value::string(name.clone()));
                }
                declare_lexical(env, &name, value, true);
                Ok(Completion::Normal)
            }
            Stmt::If { test, cons, alt } => {
                if self.eval_expr(test, env)?.truthy() {
                    self.exec_stmt(cons, env)
                } else if let Some(a) = alt {
                    self.exec_stmt(a, env)
                } else {
                    Ok(Completion::Normal)
                }
            }
            Stmt::For { init, test, update, body } => {
                let loop_env = Env::new_block(env.clone());
                let scoped = matches!(init.as_deref(), Some(Stmt::Var { kind: VarKind::Let | VarKind::Const, .. }));
                let mut use_env = if scoped { loop_env.clone() } else { env.clone() };
                if let Some(i) = init {
                    self.exec_stmt(i, &use_env)?;
                }
                loop {
                    if let Some(t) = test {
                        if !self.eval_expr(t, &use_env)?.truthy() {
                            break;
                        }
                    }
                    match self.exec_stmt(body, &use_env)? {
                        Completion::Break => break,
                        Completion::Return => return Ok(Completion::Return),
                        _ => {}
                    }
                    if let Some(u) = update {
                        if scoped {
                            // A `let`/`const` loop variable is bound afresh
                            // before the update, so closures created in the
                            // body keep that iteration's value.
                            let iter_env = Env::new_block(env.clone());
                            iter_env.borrow_mut().vars = use_env.borrow().vars.clone();
                            use_env = iter_env;
                        }
                        self.eval_expr(u, &use_env)?;
                    }
                }
                Ok(Completion::Normal)
            }
            Stmt::ForIn { left, right, body, of } => {
                let subject = self.eval_expr(right, env)?;
                let items: Vec<Value> = if *of {
                    self.iterate(&subject)?
                } else {
                    self.enumerable_keys(&subject)
                        .into_iter()
                        .map(Value::string)
                        .collect()
                };
                for item in items {
                    let iter_env = Env::new_block(env.clone());
                    match left {
                        ForLeft::Var(kind, pat) => match kind {
                            VarKind::Var => self.bind_pattern(pat, item, &iter_env, false)?,
                            VarKind::Let => self.bind_declare(pat, item, &iter_env, true)?,
                            VarKind::Const => self.bind_declare(pat, item, &iter_env, false)?,
                        },
                        ForLeft::Pat(pat) => self.bind_pattern(pat, item, &iter_env, false)?,
                    }
                    match self.exec_stmt(body, &iter_env)? {
                        Completion::Break => break,
                        Completion::Return => return Ok(Completion::Return),
                        _ => {}
                    }
                }
                Ok(Completion::Normal)
            }
            Stmt::While { test, body } => {
                let mut guard = 0u64;
                while self.eval_expr(test, env)?.truthy() {
                    match self.exec_stmt(body, env)? {
                        Completion::Break => break,
                        Completion::Return => return Ok(Completion::Return),
                        _ => {}
                    }
                    guard += 1;
                    if guard > 100_000_000 {
                        return Err(JsError::range_error("loop exceeded limit"));
                    }
                }
                Ok(Completion::Normal)
            }
            Stmt::DoWhile { body, test } => {
                loop {
                    match self.exec_stmt(body, env)? {
                        Completion::Break => break,
                        Completion::Return => return Ok(Completion::Return),
                        _ => {}
                    }
                    if !self.eval_expr(test, env)?.truthy() {
                        break;
                    }
                }
                Ok(Completion::Normal)
            }
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval_expr(e, env)?,
                    None => Value::Undefined,
                };
                let fn_env = self.function_env(env);
                fn_env
                    .borrow_mut()
                    .vars
                    .insert("__return__".into(), Binding { value: v, mutable: true });
                Ok(Completion::Return)
            }
            Stmt::Throw(e) => {
                let v = self.eval_expr(e, env)?;
                let mut err = JsError::throw(v.clone());
                err.message = match &v {
                    Value::String(text) => text.to_string(),
                    Value::Object(o) => {
                        let name = match &o.borrow().kind {
                            ObjKind::Error { name } => name.clone(),
                            _ => "Error".into(),
                        };
                        let msg = data_prop(o, "message")
                            .and_then(|m| m.as_str().map(|s| s.to_string()))
                            .unwrap_or_default();
                        if msg.is_empty() { name } else { format!("{name}: {msg}") }
                    }
                    other => format!("{other:?}"),
                };
                Err(err)
            }
            Stmt::Try { body, catch, finally } => {
                let result = self.exec_stmt(body, env);
                let mut completion = match result {
                    Ok(c) => Ok(c),
                    Err(err) => {
                        if err.kind == "__Suspend" {
                            return Err(err);
                        }
                        if let Some((pat, catch_body)) = catch {
                            let catch_env = Env::new_block(env.clone());
                            let value = self.error_value(&err);
                            if let Some(p) = pat {
                                self.bind_declare(p, value, &catch_env, true)?;
                            }
                            self.exec_stmt(catch_body, &catch_env)
                        } else {
                            Err(err)
                        }
                    }
                };
                if let Some(f) = finally {
                    if let Ok(c) = &completion {
                        if *c == Completion::Normal {
                            if let Err(e) = self.exec_stmt(f, env) {
                                completion = Err(e);
                            }
                        } else if let Err(e) = self.exec_stmt(f, env) {
                            completion = Err(e);
                        }
                    } else if let Err(e) = self.exec_stmt(f, env) {
                        completion = Err(e);
                    }
                }
                completion
            }
            Stmt::Switch { disc, cases } => {
                let d = self.eval_expr(disc, env)?;
                let mut start = None;
                for (i, (test, _)) in cases.iter().enumerate() {
                    if let Some(t) = test {
                        let v = self.eval_expr(t, env)?;
                        if strict_eq(&d, &v) {
                            start = Some(i);
                            break;
                        }
                    }
                }
                if start.is_none() {
                    start = cases.iter().position(|(t, _)| t.is_none());
                }
                let switch_env = Env::new_block(env.clone());
                if let Some(s) = start {
                    for (_, stmts) in &cases[s..] {
                        for st in stmts {
                            match self.exec_stmt(st, &switch_env)? {
                                Completion::Break => return Ok(Completion::Normal),
                                Completion::Return => return Ok(Completion::Return),
                                _ => {}
                            }
                        }
                    }
                }
                Ok(Completion::Normal)
            }
            Stmt::Break(_) => Ok(Completion::Break),
            Stmt::Continue(_) => Ok(Completion::Continue),
            Stmt::Labeled { label, body } => match self.exec_stmt(body, env)? {
                Completion::Break => Ok(Completion::Normal),
                other => {
                    let _ = label;
                    Ok(other)
                }
            },
        }
    }

    fn function_env(&self, env: &EnvRef) -> EnvRef {
        let mut cur = env.clone();
        loop {
            if cur.borrow().is_function {
                return cur;
            }
            let parent = cur.borrow().parent.clone();
            match parent {
                Some(p) => cur = p,
                None => return cur,
            }
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn eval_expr(&mut self, expr: &Expr, env: &EnvRef) -> Result<Value, JsError> {
        self.steps += 1;
        if self.step_log && self.steps % 100_000_000 == 0 {
            self.trace_steps();
        }
        if self.steps > self.step_limit {
            return Err(JsError::range_error("script exceeded its step limit"));
        }
        match expr {
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::BigInt(n) => Ok(Value::BigInt(*n)),
            Expr::String(s) => Ok(Value::string(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Null => Ok(Value::Null),
            Expr::Undefined => Ok(Value::Undefined),
            Expr::NewTarget => Ok(self
                .new_targets
                .last()
                .cloned()
                .flatten()
                .unwrap_or(Value::Undefined)),
            Expr::This => Ok(env_this(env)),
            Expr::Ident(name) => match lookup(env, name) {
                Some(v) => Ok(v),
                None => {
                    if let Some(g) = self.global_object_in(env) {
                        if let Some(p) = find_prop(&g, name) {
                            return self.get_from_object(&p.0, name, &Value::Object(g));
                        }
                    }
                    Err(JsError::reference_error(format!("{name} is not defined")))
                }
            },
            Expr::Array(items) => {
                let mut values = Vec::new();
                for item in items {
                    match item {
                        Some(Expr::Spread(e)) => {
                            let v = self.eval_expr(e, env)?;
                            values.extend(self.iterate(&v)?);
                        }
                        Some(e) => values.push(self.eval_expr(e, env)?),
                        None => values.push(Value::Undefined),
                    }
                }
                Ok(self.array_from(values))
            }
            Expr::Object(props) => {
                let obj = new_object(Some(self.intrinsics.object_proto.clone()));
                for p in props {
                    if p.kind == PropKind::Spread {
                        if let Some(e) = &p.value {
                            let v = self.eval_expr(e, env)?;
                            if let Some(src) = v.as_object() {
                                for (k, val) in own_key_values(src) {
                                    define(&obj, &k, val);
                                }
                            }
                        }
                        continue;
                    }
                    let key = match &p.key {
                        PropKey::Static(s) => s.clone(),
                        PropKey::Computed(e) => {
                            let v = self.eval_expr(e, env)?;
                            self.to_property_key(&v)?
                        }
                    };
                    match p.kind {
                        PropKind::Get | PropKind::Set => {
                            let value = match &p.value {
                                Some(Expr::Function(node)) => {
                                    self.make_user_function(node.clone(), env.clone(), false, Some(obj.clone()))
                                }
                                _ => Value::Undefined,
                            };
                            let existing = own_prop(&obj, &key);
                            let (get, set) = if p.kind == PropKind::Get {
                                (Some(value), existing.and_then(|e| e.set))
                            } else {
                                (existing.and_then(|e| e.get), Some(value))
                            };
                            define_accessor(&obj, &key, get, set, true);
                        }
                        PropKind::Method => {
                            let value = match &p.value {
                                Some(Expr::Function(node)) => {
                                    self.make_user_function(node.clone(), env.clone(), false, Some(obj.clone()))
                                }
                                Some(e) => self.eval_expr(e, env)?,
                                None => Value::Undefined,
                            };
                            define(&obj, &key, value);
                        }
                        _ => {
                            let value = match &p.value {
                                Some(e) => self.eval_expr(e, env)?,
                                None => Value::Undefined,
                            };
                            define(&obj, &key, value);
                        }
                    }
                }
                Ok(Value::Object(obj))
            }
            Expr::Function(node) => {
                if node.is_arrow {
                    Ok(self.make_user_function(node.clone(), env.clone(), true, None))
                } else {
                    Ok(self.make_user_function(node.clone(), env.clone(), false, None))
                }
            }
            Expr::Class(node) => self.eval_class(node, env),
            Expr::Template { raw, exprs } => {
                let mut out = String::new();
                for (i, part) in raw.iter().enumerate() {
                    out.push_str(part);
                    if let Some(e) = exprs.get(i) {
                        let v = self.eval_expr(e, env)?;
                        out.push_str(&self.to_string(&v)?);
                    }
                }
                Ok(Value::string(out))
            }
            Expr::TaggedTemplate { tag, raw, exprs } => {
                let tag = self.eval_expr(tag, env)?;
                let strings: Vec<Value> = raw.iter().map(|s| Value::string(s.clone())).collect();
                let strings_value = self.array_from(strings);
                let strings = strings_value.as_object().unwrap();
                let raw_values: Vec<Value> = raw.iter().map(|s| Value::string(s.clone())).collect();
                let raw_array = self.array_from(raw_values);
                strings.borrow_mut().props.insert("raw".into(), Prop::data(raw_array, false));
                let mut args = vec![strings_value];
                for e in exprs {
                    args.push(self.eval_expr(e, env)?);
                }
                self.call_value(&tag, Value::Undefined, args)
            }
            Expr::Regex { source, flags } => {
                let obj = new_object(Some(self.intrinsics.regexp_proto.clone()));
                obj.borrow_mut().kind = ObjKind::RegExp {
                    source: source.clone(),
                    flags: flags.clone(),
                    last_index: 0,
                };
                Ok(Value::Object(obj))
            }
            Expr::Unary(op, inner) => self.eval_unary(*op, inner, env),
            Expr::Update { inc, prefix, target } => {
                let r = self.eval_ref(target, env)?;
                let old = self.ref_get(&r)?;
                let new = if let Value::BigInt(n) = &old {
                    let v = if *inc { n.checked_add(1) } else { n.checked_sub(1) }
                        .ok_or_else(|| JsError::range_error("BigInt overflow"))?;
                    Value::BigInt(v)
                } else {
                    let n = self.to_number_value(&old)?;
                    Value::Number(if *inc { n + 1.0 } else { n - 1.0 })
                };
                self.ref_set(&r, new.clone())?;
                Ok(if *prefix { new } else { old })
            }
            Expr::Binary(op, a, b) => self.eval_binary(*op, a, b, env),
            Expr::Logical(op, a, b) => {
                let left = self.eval_expr(a, env)?;
                match op {
                    LogOp::And => {
                        if left.truthy() {
                            self.eval_expr(b, env)
                        } else {
                            Ok(left)
                        }
                    }
                    LogOp::Or => {
                        if left.truthy() {
                            Ok(left)
                        } else {
                            self.eval_expr(b, env)
                        }
                    }
                    LogOp::Nullish => {
                        if left.is_nullish() {
                            self.eval_expr(b, env)
                        } else {
                            Ok(left)
                        }
                    }
                }
            }
            Expr::Conditional(t, c, a) => {
                if self.eval_expr(t, env)?.truthy() {
                    self.eval_expr(c, env)
                } else {
                    self.eval_expr(a, env)
                }
            }
            Expr::Assign(op, target, value) => {
                if matches!(target.as_ref(), Expr::Array(_) | Expr::Object(_)) {
                    let v = self.eval_expr(value, env)?;
                    self.assign_pattern_expr(target, v, env)?;
                    return Ok(Value::Undefined);
                }
                let r = self.eval_ref(target, env)?;
                if *op == AssignOp::Assign {
                    let v = self.eval_expr(value, env)?;
                    self.ref_set(&r, v.clone())?;
                    return Ok(v);
                }
                let old = self.ref_get(&r)?;
                match op {
                    AssignOp::And => {
                        if !old.truthy() {
                            return Ok(old);
                        }
                    }
                    AssignOp::Or => {
                        if old.truthy() {
                            return Ok(old);
                        }
                    }
                    AssignOp::Nullish => {
                        if !old.is_nullish() {
                            return Ok(old);
                        }
                    }
                    _ => {}
                }
                let rhs = self.eval_expr(value, env)?;
                let new = if matches!(op, AssignOp::And | AssignOp::Or | AssignOp::Nullish) {
                    rhs
                } else {
                    let bin = assign_to_bin(*op).unwrap();
                    self.apply_binary(bin, old, rhs)?
                };
                self.ref_set(&r, new.clone())?;
                Ok(new)
            }
            Expr::Member { object, property, computed, optional } => {
                let base = self.eval_expr(object, env)?;
                if *optional && base.is_nullish() {
                    return Ok(Value::Undefined);
                }
                let key = if *computed {
                    let k = self.eval_expr(property, env)?;
                    self.to_property_key(&k)?
                } else {
                    match property.as_ref() {
                        Expr::String(s) => s.clone(),
                        Expr::Ident(s) => s.clone(),
                        _ => String::new(),
                    }
                };
                if matches!(object.as_ref(), Expr::Super) {
                    if let Some(obj) = base.as_object() {
                        let obj = obj.clone();
                        let receiver = env_this(env);
                        return self.get_from_object(&obj, &key, &receiver);
                    }
                }
                self.get_member(base, &key)
            }
            Expr::Chain { base, ops } => {
                // `o.m?.()` keeps `o` as the receiver even though the call
                // itself is the optional part of the chain.
                let (mut current, mut this) = match base.as_ref() {
                    Expr::Member { .. } => self.eval_callee(base, env)?,
                    _ => (self.eval_expr(base, env)?, Value::Undefined),
                };
                for op in ops {
                    match op {
                        ChainOp::Member { property, computed, optional } => {
                            if *optional && current.is_nullish() {
                                return Ok(Value::Undefined);
                            }
                            let key = if *computed {
                                let k = self.eval_expr(property, env)?;
                                self.to_property_key(&k)?
                            } else {
                                match property.as_ref() {
                                    Expr::String(s) => s.clone(),
                                    Expr::Ident(s) => s.clone(),
                                    _ => String::new(),
                                }
                            };
                            this = current.clone();
                            current = self.get_member(current, &key)?;
                        }
                        ChainOp::Call { args, optional } => {
                            if *optional && current.is_nullish() {
                                return Ok(Value::Undefined);
                            }
                            let values = self.eval_args(args, env)?;
                            let f = current.clone();
                            current = self.call_value(&f, this.clone(), values)?;
                            this = Value::Undefined;
                        }
                    }
                }
                Ok(current)
            }
            Expr::Call { callee, args, optional } => {
                let (f, this) = self.eval_callee(callee, env)?;
                if *optional && f.is_nullish() {
                    return Ok(Value::Undefined);
                }
                let values = self.eval_args(args, env)?;
                if !f.is_callable() {
                    return Err(JsError::type_error(format!(
                        "value is not a function: {}",
                        callee_label(callee)
                    )));
                }
                self.call_value(&f, this, values)
            }
            Expr::New { callee, args } => {
                let f = self.eval_expr(callee, env)?;
                let values = self.eval_args(args, env)?;
                self.construct(&f, values)
            }
            Expr::Seq(items) => {
                let mut last = Value::Undefined;
                for e in items {
                    last = self.eval_expr(e, env)?;
                }
                Ok(last)
            }
            Expr::Spread(e) => self.eval_expr(e, env),
            Expr::Yield { value, delegate } => {
                let v = match value {
                    Some(e) => self.eval_expr(e, env)?,
                    None => Value::Undefined,
                };
                if *delegate {
                    let items = self.iterate(&v)?;
                    if let Some(out) = self.yield_stack.last_mut() {
                        out.extend(items);
                    }
                    return Ok(Value::Undefined);
                }
                if let Some(out) = self.yield_stack.last_mut() {
                    out.push(v.clone());
                }
                Ok(v)
            }
            Expr::ImportMeta => {
                let url = lookup(env, "__module_url__")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                let obj = self.object();
                if let Some(o) = obj.as_object() {
                    define(o, "url", Value::string(url));
                }
                Ok(obj)
            }
            Expr::DynamicImport(spec) => {
                let specifier = self.eval_expr(spec, env)?;
                let specifier = self.to_string(&specifier)?;
                let (promise, data) = self.new_promise();
                let resolve = self.native("import.resolve", 1, {
                    let data = data.clone();
                    move |it, _, args| {
                        it.resolve_promise(&data, args.first().cloned().unwrap_or(Value::Undefined));
                        Ok(Value::Undefined)
                    }
                });
                let reject = self.native("import.reject", 1, {
                    let data = data.clone();
                    move |it, _, args| {
                        it.reject_promise(&data, args.first().cloned().unwrap_or(Value::Undefined));
                        Ok(Value::Undefined)
                    }
                });
                self.pending_imports.push((specifier, resolve, reject));
                Ok(promise)
            }
            Expr::Await(e) => {
                let v = self.eval_expr(e, env)?;
                if let Some(obj) = v.as_object() {
                    let promise = match &obj.borrow().kind {
                        ObjKind::Promise(p) => Some(p.clone()),
                        _ => None,
                    };
                    if let Some(p) = promise {
                        let (state, value) = {
                            let d = p.borrow();
                            (d.state, d.value.clone())
                        };
                        return match state {
                            PromiseState::Fulfilled => Ok(value),
                            PromiseState::Rejected => Err(JsError::throw(value)),
                            PromiseState::Pending => {
                                // The interpreter is synchronous. Drain the microtask
                                // queue (promise reactions, async continuations) so
                                // awaits on work that is already resolving can settle.
                                self.pump_until_settled(&p)?;
                                let (state, value) = {
                                    let d = p.borrow();
                                    (d.state, d.value.clone())
                                };
                                match state {
                                    PromiseState::Fulfilled => Ok(value),
                                    PromiseState::Rejected => Err(JsError::throw(value)),
                                    // Suspend this async body; the interpreter
                                    // resumes it when the promise settles.
                                    PromiseState::Pending => {
                                        self.async_suspend = Some(v.clone());
                                        Err(JsError { kind: "__Suspend", message: "await".into(), value: None, stack: Vec::new(), line: 0 })
                                    }
                                }
                            }
                        };
                    }
                }
                // `await` adopts any thenable, not just native promises.
                if let Some(obj) = v.as_object() {
                    let then = self.get_from_object(&obj, "then", &v)?;
                    if then.is_callable() {
                        let (_p, data) = self.new_promise();
                        let resolve = self.native("", 1, {
                            let data = data.clone();
                            move |it, _, args| {
                                it.resolve_promise(
                                    &data,
                                    args.first().cloned().unwrap_or(Value::Undefined),
                                );
                                Ok(Value::Undefined)
                            }
                        });
                        let reject = self.native("", 1, {
                            let data = data.clone();
                            move |it, _, args| {
                                it.reject_promise(
                                    &data,
                                    args.first().cloned().unwrap_or(Value::Undefined),
                                );
                                Ok(Value::Undefined)
                            }
                        });
                        self.call_value(&then, v.clone(), vec![resolve, reject])?;
                        self.pump_until_settled(&data)?;
                        let (state, value) = {
                            let d = data.borrow();
                            (d.state, d.value.clone())
                        };
                        return match state {
                            PromiseState::Fulfilled => Ok(value),
                            PromiseState::Rejected => Err(JsError::throw(value)),
                            PromiseState::Pending => Ok(Value::Undefined),
                        };
                    }
                }
                Ok(v)
            }
            Expr::Super => {
                let home = lookup(env, "__home__");
                if let Some(Value::Object(h)) = home {
                    if let Some(proto) = h.borrow().proto.clone() {
                        return Ok(Value::Object(proto));
                    }
                }
                Ok(Value::Undefined)
            }
            Expr::SuperCall(args) => {
                let parent = lookup(env, "__super_ctor__").ok_or_else(|| {
                    JsError::type_error("'super' keyword unexpected here")
                })?;
                let this = env_this(env);
                let values = self.eval_args(args, env)?;
                let saved_pending = self.pending_new_target.take();
                self.pending_new_target = self.new_targets.last().cloned().flatten();
                let result = self.call_value(&parent, this.clone(), values);
                self.pending_new_target = saved_pending;
                let result = result?;
                // A parent constructor may return an object; in a derived
                // constructor that object becomes `this` for the rest of the
                // body and for any implicit return.
                if result.is_object() {
                    env.borrow_mut().this = result.clone();
                    Ok(result)
                } else {
                    Ok(this)
                }
            }
        }
    }

    fn eval_args(&mut self, args: &[Expr], env: &EnvRef) -> Result<Vec<Value>, JsError> {
        let mut values = Vec::new();
        for a in args {
            if let Expr::Spread(e) = a {
                let v = self.eval_expr(e, env)?;
                values.extend(self.iterate(&v)?);
            } else {
                values.push(self.eval_expr(a, env)?);
            }
        }
        Ok(values)
    }

    fn eval_callee(&mut self, callee: &Expr, env: &EnvRef) -> Result<(Value, Value), JsError> {
        match callee {
            Expr::Member { object, property, computed, optional } => {
                let base = self.eval_expr(object, env)?;
                if *optional && base.is_nullish() {
                    return Ok((Value::Undefined, Value::Undefined));
                }
                let key = if *computed {
                    let k = self.eval_expr(property, env)?;
                    self.to_property_key(&k)?
                } else {
                    match property.as_ref() {
                        Expr::String(s) => s.clone(),
                        Expr::Ident(s) => s.clone(),
                        _ => String::new(),
                    }
                };
                let f = self.get_member(base.clone(), &key)?;
                let this = if matches!(object.as_ref(), Expr::Super) {
                    env_this(env)
                } else {
                    base
                };
                Ok((f, this))
            }
            Expr::Super => {
                let home = lookup(env, "__home__");
                let base = if let Some(Value::Object(h)) = home {
                    h.borrow().proto.clone().map(Value::Object).unwrap_or(Value::Undefined)
                } else {
                    Value::Undefined
                };
                Ok((base, env_this(env)))
            }
            _ => {
                let f = self.eval_expr(callee, env)?;
                Ok((f, Value::Undefined))
            }
        }
    }

    fn eval_unary(&mut self, op: UnOp, inner: &Expr, env: &EnvRef) -> Result<Value, JsError> {
        match op {
            UnOp::Typeof => {
                if let Expr::Ident(name) = inner {
                    if lookup(env, name).is_none() {
                        if let Some(g) = self.global_object_in(env) {
                            if !has_property(&g, name) {
                                return Ok(Value::string("undefined"));
                            }
                        }
                    }
                }
                let v = self.eval_expr(inner, env)?;
                Ok(Value::string(v.type_of()))
            }
            UnOp::Delete => match inner {
                Expr::Member { object, property, computed, .. } => {
                    let base = self.eval_expr(object, env)?;
                    let key = if *computed {
                        let k = self.eval_expr(property, env)?;
                        self.to_property_key(&k)?
                    } else {
                        match property.as_ref() {
                            Expr::String(s) => s.clone(),
                            Expr::Ident(s) => s.clone(),
                            _ => String::new(),
                        }
                    };
                    Ok(Value::Bool(self.delete_member(&base, &key)))
                }
                Expr::Ident(name) => {
                    if let Some(g) = self.global_object_in(env) {
                        delete_prop(&g, name);
                    }
                    Ok(Value::Bool(true))
                }
                _ => {
                    self.eval_expr(inner, env)?;
                    Ok(Value::Bool(true))
                }
            },
            UnOp::Void => {
                self.eval_expr(inner, env)?;
                Ok(Value::Undefined)
            }
            UnOp::Not => Ok(Value::Bool(!self.eval_expr(inner, env)?.truthy())),
            UnOp::Neg => {
                let v = self.eval_expr(inner, env)?;
                if let Value::BigInt(n) = &v {
                    return n
                        .checked_neg()
                        .map(Value::BigInt)
                        .ok_or_else(|| JsError::range_error("BigInt overflow"));
                }
                Ok(Value::Number(-self.to_number_value(&v)?))
            }
            UnOp::Pos => {
                let v = self.eval_expr(inner, env)?;
                if matches!(v, Value::BigInt(_)) {
                    return Ok(v);
                }
                Ok(Value::Number(self.to_number_value(&v)?))
            }
            UnOp::BitNot => {
                let v = self.eval_expr(inner, env)?;
                if let Value::BigInt(n) = &v {
                    return Ok(Value::BigInt(!*n));
                }
                Ok(Value::Number((!v.to_i32()) as f64))
            }
        }
    }

    fn eval_binary(&mut self, op: BinOp, a: &Expr, b: &Expr, env: &EnvRef) -> Result<Value, JsError> {
        if op == BinOp::In {
            let key = self.eval_expr(a, env)?;
            let obj = self.eval_expr(b, env)?;
            let key = self.to_property_key(&key)?;
            let obj = obj
                .as_object()
                .ok_or_else(|| JsError::type_error("right-hand side of 'in' is not an object"))?;
            return Ok(Value::Bool(has_property(obj, &key)));
        }
        if op == BinOp::Instanceof {
            let left = self.eval_expr(a, env)?;
            let right = self.eval_expr(b, env)?;
            let ctor = right
                .as_object()
                .ok_or_else(|| JsError::type_error("right-hand side of 'instanceof' is not callable"))?;
            let proto = data_prop(ctor, "prototype")
                .and_then(|v| v.as_object().cloned())
                .ok_or_else(|| JsError::type_error("function has non-object prototype"))?;
            let mut cur = left.as_object().and_then(|o| o.borrow().proto.clone());
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
            return Ok(Value::Bool(false));
        }
        let left = self.eval_expr(a, env)?;
        let right = self.eval_expr(b, env)?;
        self.apply_binary(op, left, right)
    }

    fn apply_binary(&mut self, op: BinOp, left: Value, right: Value) -> Result<Value, JsError> {
        match op {
            BinOp::Eq => Ok(Value::Bool(loose_eq(&left, &right))),
            BinOp::Ne => Ok(Value::Bool(!loose_eq(&left, &right))),
            BinOp::StrictEq => Ok(Value::Bool(strict_eq(&left, &right))),
            BinOp::StrictNe => Ok(Value::Bool(!strict_eq(&left, &right))),
            BinOp::Add => {
                let lp = self.to_primitive(&left, false)?;
                let rp = self.to_primitive(&right, false)?;
                if matches!(lp, Value::String(_)) || matches!(rp, Value::String(_)) {
                    let mut s = self.to_string(&lp)?;
                    s.push_str(&self.to_string(&rp)?);
                    Ok(Value::string(s))
                } else if let (Value::BigInt(a), Value::BigInt(b)) = (&lp, &rp) {
                    a.checked_add(*b)
                        .map(Value::BigInt)
                        .ok_or_else(|| JsError::range_error("BigInt overflow"))
                } else if matches!(lp, Value::BigInt(_)) || matches!(rp, Value::BigInt(_)) {
                    Err(JsError::type_error(
                        "cannot mix BigInt and other types, use explicit conversions",
                    ))
                } else {
                    Ok(Value::Number(lp.to_number() + rp.to_number()))
                }
            }
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Exp => {
                if let (Value::BigInt(a), Value::BigInt(b)) = (&left, &right) {
                    let v = match op {
                        BinOp::Sub => a.checked_sub(*b),
                        BinOp::Mul => a.checked_mul(*b),
                        BinOp::Div => {
                            if *b == 0 {
                                return Err(JsError::range_error("Division by zero"));
                            }
                            a.checked_div(*b)
                        }
                        BinOp::Mod => {
                            if *b == 0 {
                                return Err(JsError::range_error("Division by zero"));
                            }
                            a.checked_rem(*b)
                        }
                        _ => {
                            if *b < 0 {
                                return Err(JsError::range_error(
                                    "BigInt exponent must be non-negative",
                                ));
                            }
                            if *b > u32::MAX as i128 {
                                return Err(JsError::range_error("BigInt exponent too large"));
                            }
                            a.checked_pow(*b as u32)
                        }
                    };
                    return v
                        .map(Value::BigInt)
                        .ok_or_else(|| JsError::range_error("BigInt overflow"));
                }
                if matches!(left, Value::BigInt(_)) || matches!(right, Value::BigInt(_)) {
                    return Err(JsError::type_error(
                        "cannot mix BigInt and other types, use explicit conversions",
                    ));
                }
                let l = self.to_number_value(&left)?;
                let r = self.to_number_value(&right)?;
                Ok(Value::Number(match op {
                    BinOp::Sub => l - r,
                    BinOp::Mul => l * r,
                    BinOp::Div => l / r,
                    BinOp::Mod => l % r,
                    _ => l.powf(r),
                }))
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr | BinOp::UShr => {
                if let (Value::BigInt(a), Value::BigInt(b)) = (&left, &right) {
                    let v = match op {
                        BinOp::BitAnd => *a & *b,
                        BinOp::BitOr => *a | *b,
                        BinOp::BitXor => *a ^ *b,
                        BinOp::Shl => {
                            if *b < 0 {
                                return Err(JsError::range_error("BigInt shift count is negative"));
                            }
                            if *b >= 128 {
                                0
                            } else {
                                a << (*b as u32)
                            }
                        }
                        BinOp::Shr => {
                            if *b < 0 {
                                return Err(JsError::range_error("BigInt shift count is negative"));
                            }
                            if *b >= 128 {
                                if *a < 0 { -1 } else { 0 }
                            } else {
                                a >> (*b as u32)
                            }
                        }
                        _ => {
                            return Err(JsError::type_error(
                                "BigInts have no unsigned right shift",
                            ));
                        }
                    };
                    return Ok(Value::BigInt(v));
                }
                if matches!(left, Value::BigInt(_)) || matches!(right, Value::BigInt(_)) {
                    return Err(JsError::type_error(
                        "cannot mix BigInt and other types, use explicit conversions",
                    ));
                }
                let l = left.to_i32();
                let r = right.to_i32();
                let n = match op {
                    BinOp::BitAnd => l & r,
                    BinOp::BitOr => l | r,
                    BinOp::BitXor => l ^ r,
                    BinOp::Shl => l.wrapping_shl((r as u32) & 31),
                    BinOp::Shr => l.wrapping_shr((r as u32) & 31),
                    _ => (left.to_u32() >> ((r as u32) & 31)) as i32,
                };
                Ok(Value::Number(n as f64))
            }
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                let lp = self.to_primitive(&left, false)?;
                let rp = self.to_primitive(&right, false)?;
                let result = if let (Value::String(a), Value::String(b)) = (&lp, &rp) {
                    match op {
                        BinOp::Lt => a < b,
                        BinOp::Gt => a > b,
                        BinOp::Le => a <= b,
                        _ => a >= b,
                    }
                } else if let (Value::BigInt(a), Value::BigInt(b)) = (&lp, &rp) {
                    match op {
                        BinOp::Lt => a < b,
                        BinOp::Gt => a > b,
                        BinOp::Le => a <= b,
                        _ => a >= b,
                    }
                } else {
                    let a = lp.to_number();
                    let b = rp.to_number();
                    match op {
                        BinOp::Lt => a < b,
                        BinOp::Gt => a > b,
                        BinOp::Le => a <= b,
                        _ => a >= b,
                    }
                };
                Ok(Value::Bool(result))
            }
            _ => Ok(Value::Bool(false)),
        }
    }

    fn eval_ref(&mut self, expr: &Expr, env: &EnvRef) -> Result<Ref, JsError> {
        match expr {
            Expr::Ident(name) => Ok(Ref::Env(env.clone(), name.clone())),
            Expr::Member { object, property, computed, .. } => {
                let base = self.eval_expr(object, env)?;
                let key = if *computed {
                    let k = self.eval_expr(property, env)?;
                    self.to_property_key(&k)?
                } else {
                    match property.as_ref() {
                        Expr::String(s) => s.clone(),
                        Expr::Ident(s) => s.clone(),
                        _ => String::new(),
                    }
                };
                Ok(Ref::Member(base, key))
            }
            _ => {
                let _ = self.eval_expr(expr, env)?;
                Err(JsError::reference_error("invalid assignment target"))
            }
        }
    }

    fn ref_get(&mut self, r: &Ref) -> Result<Value, JsError> {
        match r {
            Ref::Env(env, name) => {
                if let Some(v) = lookup(env, name) {
                    return Ok(v);
                }
                if let Some(g) = self.global_object_in(env) {
                    if let Some((o, _)) = find_prop(&g, name) {
                        return self.get_from_object(&o, name, &Value::Object(g));
                    }
                }
                Err(JsError::reference_error(format!("{name} is not defined")))
            }
            Ref::Member(base, key) => self.get_member(base.clone(), key),
        }
    }

    fn ref_set(&mut self, r: &Ref, value: Value) -> Result<(), JsError> {
        match r {
            Ref::Env(env, name) => {
                if !assign_env(env, name, value.clone()) {
                    let g = self.global_object_in(env).unwrap_or_else(|| self.global.clone());
                    define(&g, name, value.clone());
                    declare_lexical(env, name, value, true);
                }
                Ok(())
            }
            Ref::Member(base, key) => self.set_member(base.clone(), key, value),
        }
    }

    fn global_object_in(&self, env: &EnvRef) -> Option<ObjRef> {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            if let Some(g) = e.borrow().global.clone() {
                return Some(g);
            }
            let p = e.borrow().parent.clone();
            cur = p;
        }
        None
    }

    // ------------------------------------------------------------------
    // Patterns
    // ------------------------------------------------------------------

    fn bind_declare(&mut self, pat: &Pattern, value: Value, env: &EnvRef, mutable: bool) -> Result<(), JsError> {
        match pat {
            Pattern::Ident(name) => {
                declare_lexical(env, name, value, mutable);
                Ok(())
            }
            Pattern::Default(inner, default) => {
                if value.is_undefined() {
                    let dv = self.eval_expr(default, env)?;
                    self.bind_declare(inner, dv, env, mutable)
                } else {
                    self.bind_declare(inner, value, env, mutable)
                }
            }
            Pattern::Rest(inner) => self.bind_declare(inner, value, env, mutable),
            Pattern::Array { items, rest } => {
                let values = self.iterate(&value).unwrap_or_default();
                for (i, item) in items.iter().enumerate() {
                    if let Some(p) = item {
                        let v = values.get(i).cloned().unwrap_or(Value::Undefined);
                        self.bind_declare(p, v, env, mutable)?;
                    }
                }
                if let Some(r) = rest {
                    let remaining: Vec<Value> = values.iter().skip(items.len()).cloned().collect();
                    let arr = self.array_from(remaining);
                    self.bind_declare(r, arr, env, mutable)?;
                }
                Ok(())
            }
            Pattern::Object { props, rest } => {
                let mut used = Vec::new();
                for (key, p, _) in props {
                    let key_s = match key {
                        PropKey::Static(s) => s.clone(),
                        PropKey::Computed(e) => {
                            let v = self.eval_expr(e, env)?;
                            self.to_property_key(&v)?
                        }
                    };
                    used.push(key_s.clone());
                    let v = self.get_member(value.clone(), &key_s)?;
                    if let Some(p) = p {
                        self.bind_declare(p, v, env, mutable)?;
                    }
                }
                if let Some(r) = rest {
                    let obj = new_object(Some(self.intrinsics.object_proto.clone()));
                    if let Some(src) = value.as_object() {
                        for (k, v) in own_key_values(src) {
                            if !used.contains(&k) {
                                define(&obj, &k, v);
                            }
                        }
                    }
                    self.bind_declare(r, Value::Object(obj), env, mutable)?;
                }
                Ok(())
            }
        }
    }

    fn bind_pattern(&mut self, pat: &Pattern, value: Value, env: &EnvRef, declare: bool) -> Result<(), JsError> {
        if declare {
            return self.bind_declare(pat, value, env, true);
        }
        match pat {
            Pattern::Ident(name) => {
                if !assign_env(env, name, value.clone()) {
                    self.ref_set(&Ref::Env(env.clone(), name.clone()), value)?;
                }
                Ok(())
            }
            Pattern::Default(inner, default) => {
                if value.is_undefined() {
                    let dv = self.eval_expr(default, env)?;
                    self.bind_pattern(inner, dv, env, false)
                } else {
                    self.bind_pattern(inner, value, env, false)
                }
            }
            Pattern::Rest(inner) => self.bind_pattern(inner, value, env, false),
            Pattern::Array { items, rest } => {
                let values = self.iterate(&value).unwrap_or_default();
                for (i, item) in items.iter().enumerate() {
                    if let Some(p) = item {
                        let v = values.get(i).cloned().unwrap_or(Value::Undefined);
                        self.bind_pattern(p, v, env, false)?;
                    }
                }
                if let Some(r) = rest {
                    let remaining: Vec<Value> = values.iter().skip(items.len()).cloned().collect();
                    let arr = self.array_from(remaining);
                    self.bind_pattern(r, arr, env, false)?;
                }
                Ok(())
            }
            Pattern::Object { props, rest } => {
                let mut used = Vec::new();
                for (key, p, _) in props {
                    let key_s = match key {
                        PropKey::Static(s) => s.clone(),
                        PropKey::Computed(e) => {
                            let v = self.eval_expr(e, env)?;
                            self.to_property_key(&v)?
                        }
                    };
                    used.push(key_s.clone());
                    let v = self.get_member(value.clone(), &key_s)?;
                    if let Some(p) = p {
                        self.bind_pattern(p, v, env, false)?;
                    }
                }
                if let Some(r) = rest {
                    let obj = new_object(Some(self.intrinsics.object_proto.clone()));
                    if let Some(src) = value.as_object() {
                        for (k, v) in own_key_values(src) {
                            if !used.contains(&k) {
                                define(&obj, &k, v);
                            }
                        }
                    }
                    self.bind_pattern(r, Value::Object(obj), env, false)?;
                }
                Ok(())
            }
        }
    }

    /// A destructuring target may carry a default (`{a: b = 1}`), which the
    /// expression parser represents as an assignment. Apply the default only
    /// when the source value is undefined, then assign the resolved value.
    fn assign_pattern_value(&mut self, target: &Expr, value: Value, env: &EnvRef) -> Result<(), JsError> {
        if let Expr::Assign(AssignOp::Assign, actual, default) = target {
            let value = if value.is_undefined() {
                self.eval_expr(default, env)?
            } else {
                value
            };
            self.assign_pattern_expr(actual, value, env)
        } else {
            self.assign_pattern_expr(target, value, env)
        }
    }

    fn assign_pattern_expr(&mut self, target: &Expr, value: Value, env: &EnvRef) -> Result<(), JsError> {
        match target {
            Expr::Array(items) => {
                let values = self.iterate(&value)?;
                let mut i = 0;
                for item in items {
                    match item {
                        Some(Expr::Spread(e)) => {
                            let rest = self.array_from(values[i..].to_vec());
                            let r = self.eval_ref(e, env)?;
                            self.ref_set(&r, rest)?;
                            return Ok(());
                        }
                        Some(e) => {
                            let v = values.get(i).cloned().unwrap_or(Value::Undefined);
                            self.assign_pattern_value(e, v, env)?;
                            i += 1;
                        }
                        None => i += 1,
                    }
                }
                Ok(())
            }
            Expr::Object(props) => {
                let mut used = Vec::new();
                for p in props {
                    if p.kind == PropKind::Spread {
                        let obj = new_object(Some(self.intrinsics.object_proto.clone()));
                        if let Some(src) = value.as_object() {
                            for (k, v) in own_key_values(src) {
                                if !used.contains(&k) {
                                    define(&obj, &k, v);
                                }
                            }
                        }
                        if let Some(e) = &p.value {
                            let r = self.eval_ref(e, env)?;
                            self.ref_set(&r, Value::Object(obj))?;
                        }
                        continue;
                    }
                    let key = match &p.key {
                        PropKey::Static(s) => s.clone(),
                        PropKey::Computed(e) => {
                            let k = self.eval_expr(e, env)?;
                            self.to_property_key(&k)?
                        }
                    };
                    used.push(key.clone());
                    let v = self.get_member(value.clone(), &key)?;
                    if let Some(e) = &p.value {
                        self.assign_pattern_value(e, v, env)?;
                    }
                }
                Ok(())
            }
            _ => {
                let r = self.eval_ref(target, env)?;
                self.ref_set(&r, value)
            }
        }
    }

    // ------------------------------------------------------------------
    // JSON and regex helpers used by builtins
    // ------------------------------------------------------------------

    pub fn json_stringify(&mut self, value: &Value) -> Result<String, JsError> {
        let mut out = String::new();
        self.json_serialize(value, &mut out)?;
        Ok(out)
    }

    fn json_serialize(&mut self, value: &Value, out: &mut String) -> Result<(), JsError> {
        match value {
            Value::Undefined => out.push_str("null"),
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => {
                if n.is_finite() {
                    out.push_str(&number_to_string(*n));
                } else {
                    out.push_str("null");
                }
            }
            Value::String(s) => out.push_str(&json_quote(s)),
            Value::BigInt(_) => {
                return Err(JsError::type_error("Do not know how to serialize a BigInt"));
            }
            Value::Object(o) => {
                let kind_tag = {
                    let o = o.borrow();
                    match &o.kind {
                        ObjKind::Array(_) => "array",
                        ObjKind::Function(_) => "skip",
                        _ => "object",
                    }
                };
                if kind_tag == "skip" {
                    out.push_str("null");
                    return Ok(());
                }
                if kind_tag == "array" {
                    let values = array_values(value).unwrap_or_default();
                    out.push('[');
                    for (i, v) in values.iter().enumerate() {
                        if i > 0 {
                            out.push(',');
                        }
                        self.json_serialize(v, out)?;
                    }
                    out.push(']');
                } else {
                    let keys = own_keys(&o);
                    out.push('{');
                    let mut first = true;
                    for k in keys {
                        let v = match self.get_member(value.clone(), &k) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        if matches!(v, Value::Undefined) || v.is_callable() {
                            continue;
                        }
                        if !first {
                            out.push(',');
                        }
                        first = false;
                        out.push_str(&json_quote(&k));
                        out.push(':');
                        self.json_serialize(&v, out)?;
                    }
                    out.push('}');
                }
            }
        }
        Ok(())
    }

    pub fn json_parse(&mut self, text: &str) -> Result<Value, JsError> {
        let parsed = json::parse(text).map_err(|e| JsError::syntax_error(format!("invalid JSON: {e}")))?;
        Ok(json_to_value(&parsed, &self.intrinsics))
    }

    pub fn regex_for(&mut self, source: &str, flags: &str) -> Result<regress::Regex, JsError> {
        let key = format!("{source}/{flags}");
        if let Some(r) = self.regex_cache.get(&key) {
            return Ok(r.clone());
        }
        for f in flags.chars() {
            if !matches!(f, 'i' | 'm' | 's' | 'u' | 'g' | 'y' | 'd' | 'v') {
                return Err(JsError::syntax_error(format!("invalid regular expression flag '{f}'")));
            }
        }
        let parse_flags: String = flags.chars().filter(|f| matches!(f, 'i' | 'm' | 's' | 'u' | 'v')).collect();
        let re = regress::Regex::with_flags(source, regress::Flags::from(parse_flags.as_str()))
            .map_err(|e| JsError::syntax_error(format!("invalid regular expression: {e}")))?;
        self.regex_cache.insert(key, re.clone());
        Ok(re)
    }

    pub fn regexp_parts(&self, value: &Value) -> Option<(String, String)> {
        let obj = value.as_object()?;
        let o = obj.borrow();
        if let ObjKind::RegExp { source, flags, .. } = &o.kind {
            Some((source.clone(), flags.clone()))
        } else {
            None
        }
    }

    pub fn random(&mut self) -> f64 {
        self.rng = self
            .rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.rng >> 11) as f64) / ((1u64 << 53) as f64)
    }

    pub fn now_ms(&mut self) -> f64 {
        if self.now > 0.0 {
            self.now
        } else {
            self.host.now_ms()
        }
    }

    pub fn add_timer(&mut self, callback: Value, delay: f64, repeat: bool, args: Vec<Value>) -> f64 {
        let id = self.next_timer;
        self.next_timer += 1.0;
        let delay = if delay.is_finite() && delay > 0.0 { delay } else { 0.0 };
        let now = self.now_ms();
        self.timers.push(Timer {
            id,
            due: now + delay,
            repeat: if repeat { Some(delay.max(1.0)) } else { None },
            callback,
            args,
        });
        id
    }

    pub fn clear_timer(&mut self, id: f64) {
        self.timers.retain(|t| t.id != id);
    }
}

fn promise_data_of(obj: &ObjRef) -> Option<Rc<RefCell<PromiseData>>> {
    match &obj.borrow().kind {
        ObjKind::Promise(d) => Some(d.clone()),
        _ => None,
    }
}

fn callee_label(e: &Expr) -> String {
    match e {
        Expr::Ident(n) => n.clone(),
        Expr::Member { object, property, computed, .. } => {
            let base = callee_label(object);
            if *computed {
                format!("{base}[{}]", callee_label(property))
            } else if let Expr::String(name) = property.as_ref() {
                format!("{base}.{name}")
            } else {
                base
            }
        }
        Expr::Call { callee, .. } => format!("{}()", callee_label(callee)),
        _ => "expression".into(),
    }
}

fn empty_fn(name: Option<String>) -> FnNode {
    FnNode {
        name,
        params: Vec::new(),
        body: FnBody::Block(Vec::new()),
        is_arrow: false,
        is_async: false,
        is_generator: false,
        self_bind: false,
        line: 0,
    }
}

fn make_native_value(
    name: &str,
    length: usize,
    proto: Option<ObjRef>,
    f: Rc<NativeImpl>,
) -> Value {
    let obj = Rc::new(RefCell::new(Object {
        proto,
        props: Default::default(),
        kind: ObjKind::Function(Func::Native(Rc::new(NativeFunction {
            name: name.to_string(),
            length,
            f,
        }))),
        array_holes: None,
        extensible: true,
        proxy: None,
    }));
    Value::Object(obj)
}

fn assign_to_bin(op: AssignOp) -> Option<BinOp> {
    Some(match op {
        AssignOp::Add => BinOp::Add,
        AssignOp::Sub => BinOp::Sub,
        AssignOp::Mul => BinOp::Mul,
        AssignOp::Div => BinOp::Div,
        AssignOp::Mod => BinOp::Mod,
        AssignOp::Exp => BinOp::Exp,
        AssignOp::BitAnd => BinOp::BitAnd,
        AssignOp::BitOr => BinOp::BitOr,
        AssignOp::BitXor => BinOp::BitXor,
        AssignOp::Shl => BinOp::Shl,
        AssignOp::Shr => BinOp::Shr,
        AssignOp::UShr => BinOp::UShr,
        _ => return None,
    })
}

fn json_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_to_value(v: &json::Json, intr: &Intrinsics) -> Value {
    match v {
        json::Json::Null => Value::Null,
        json::Json::Bool(b) => Value::Bool(*b),
        json::Json::Num(n) => Value::Number(*n),
        json::Json::Str(s) => Value::string(s.clone()),
        json::Json::Arr(items) => {
            let values = items.iter().map(|v| json_to_value(v, intr)).collect();
            Value::Object(new_array(values, intr.array_proto.clone()))
        }
        json::Json::Obj(map) => {
            let obj = new_object(Some(intr.object_proto.clone()));
            for (k, v) in map {
                define(&obj, k, json_to_value(v, intr));
            }
            Value::Object(obj)
        }
    }
}

fn format_date(ms: f64) -> String {
    if !ms.is_finite() {
        return "Invalid Date".into();
    }
    let secs = (ms / 1000.0).floor() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    let _ = (y, m, d, hh, mm, ss);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        y,
        m,
        d,
        hh,
        mm,
        ss
    )
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
