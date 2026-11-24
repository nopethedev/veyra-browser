use veyra_js::{Host, Interp, NullHost};

fn eval(src: &str) -> String {
    let mut it = Interp::new(Box::new(NullHost::default()));
    it.eval_script(src).unwrap_or_else(|e| panic!("eval failed: {e}\nstack: {:?}", e.stack));
    it.run_microtasks().unwrap();
    it.get_global("result")
        .map(|v| it.to_string(&v).unwrap())
        .unwrap_or_default()
}

#[test]
fn functions_closures_arrays() {
    let out = eval(r#"
        const add = (a, b = 10) => a + b;
        const xs = [1, 2, 3].map(x => x * 2);
        function make(n) { return function () { return n++; }; }
        const next = make(5);
        const o = { x: 7, f() { return this.x; } };
        result = add(2) + ':' + xs.join(',') + ':' + next() + next() + ':' + o.f();
    "#);
    assert_eq!(out, "12:2,4,6:56:7");
}

#[test]
fn classes_inheritance_getters() {
    let out = eval(r#"
        class A {
            constructor(x) { this.x = x; }
            get v() { return this.x * 2; }
            inc() { return ++this.x; }
        }
        class B extends A {
            constructor(x) { super(x); this.y = x + 1; }
            sum() { return this.v + this.y; }
            who() { return 'B'; }
        }
        const b = new B(3);
        result = b.sum() + ':' + (b instanceof A) + ':' + (b instanceof B) + ':' + b.who();
    "#);
    assert_eq!(out, "10:true:true:B");
}

#[test]
fn let_and_var_loop_capture() {
    let out = eval(r#"
        const lets = [];
        for (let i = 0; i < 3; i++) lets.push(() => i);
        const vars = [];
        for (var j = 0; j < 3; j++) vars.push(() => j);
        result = lets.map(f => f()).join('') + ':' + vars.map(f => f()).join('');
    "#);
    assert_eq!(out, "012:333");
}

#[test]
fn destructuring_spread_templates() {
    let out = eval(r#"
        const { a, b: { c = 5 } = {} } = { a: 1, b: {} };
        const [x, ...rest] = [1, 2, 3];
        const merged = { ...{ p: 1 }, q: 2 };
        result = `${a}${c}:${x}${rest.length}:${merged.p}${merged.q}`;
    "#);
    assert_eq!(out, "15:12:12");
}

#[test]
fn last_index_of_honours_from_index() {
    let out = eval(r#"
        const s = 'a.b.c.d';
        result = [
            s.lastIndexOf('.'),
            s.lastIndexOf('.', 3),
            s.lastIndexOf('.', -2),
            s.lastIndexOf('', 2),
            s.lastIndexOf('z')
        ].join(',');
    "#);
    assert_eq!(out, "5,3,5,2,-1");
}

#[test]
fn bigint_arithmetic() {
    let out = eval(r#"
        const big = 9007199254740993n;
        result = String(big + 1n) + ':' + (big > 9007199254740992n) + ':' + typeof big;
    "#);
    assert_eq!(out, "9007199254740994:true:bigint");
}

#[test]
fn optional_call_keeps_receiver() {
    let out = eval(r#"
        const o = { x: 7, f() { return this.x; }, n: null };
        result = [o.f?.(), o.n?.(), o.missing?.()].join(',');
    "#);
    assert_eq!(out, "7,,");
}

#[test]
fn super_calls_and_async_class_methods() {
    let out = eval(r#"
        class A {
            constructor() { this.v = 1; }
            m() { return this.v; }
            async am() { return this.v + 1; }
        }
        class B extends A {
            m() { return super.m() + 1; }
            async am() { return (await super.am()) + 1; }
        }
        const b = new B();
        b.am().then(v => { result = b.m() + ':' + v; });
    "#);
    assert_eq!(out, "2:3");
}

#[test]
fn await_adopts_thenables() {
    let out = eval(r#"
        var out = '';
        (async function () {
            var v = await { then(resolve) { resolve(41); } };
            out = v + 1;
        })();
        result = out;
    "#);
    assert_eq!(out, "42");
}

#[test]
fn structured_clone_deep_copies_and_rejects_functions() {
    let out = eval(r#"
        const src = { list: [1, { n: 2 }], when: new Date(1234) };
        const copy = structuredClone(src);
        copy.list[1].n = 99;
        copy.list.push(4);
        let err = '';
        try { structuredClone(() => 1); } catch (e) { err = e.name; }
        result = src.list[1].n + ':' + copy.list[1].n + ':' + src.list.length + ':' +
                 copy.when.getTime() + ':' + err;
    "#);
    assert_eq!(out, "2:99:2:1234:TypeError");
}

#[test]
fn accessors_keep_receiver_on_every_path() {
    let out = eval(r#"
        const seen = [];
        const proto = { get g() { return this.x; } };

        const direct = Object.create(proto);
        direct.x = 42;
        seen.push(direct.g);

        const target = Object.create(proto);
        target.x = 7;
        const proxy = new Proxy(target, {
            get(t, k, r) { return Reflect.get(t, k, r); }
        });
        seen.push(proxy.g);

        const defined = { x: 9 };
        Object.defineProperty(defined, 'g', {
            get() { return this.x; },
            configurable: true
        });
        seen.push(defined.g);

        const wrapped = Object.create(proto);
        wrapped.x = 11;
        seen.push(wrapped.g);

        result = seen.join(',');
    "#);
    assert_eq!(out, "42,7,9,11");
}
