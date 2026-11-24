//! Regression harness for receiver preservation through every accessor path.

use veyra_engine::{LoadOptions, Page};

#[test]
fn accessor_receivers_are_the_lookup_receiver() {
    let mut page = Page::load("veyra://newtab", LoadOptions::default());
    let result = page
        .js
        .eval_script(
            r#"
            (function () {
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

                // Real DOM wrapper layer: the getter lives on the interface
                // prototype while the receiver is the element wrapper.
                const el = document.createElement('div');
                el.__receiver = 'element';
                Object.defineProperty(Element.prototype, '__receiver_g', {
                    get() { return this.__receiver; },
                    configurable: true
                });
                seen.push(el.__receiver_g);

                return JSON.stringify({
                    seen,
                    instance: el instanceof Element,
                    node: el instanceof Node
                });
            })();
            "#,
        )
        .expect("harness script failed");
    let text = page.js.to_string(&result).expect("result is a string");
    assert_eq!(
        text,
        r#"{"seen":[42,7,9,"element"],"instance":true,"node":true}"#
    );
}
