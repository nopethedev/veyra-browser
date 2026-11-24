//! Custom-element upgrade ordering for nodes created from markup.

use veyra_engine::{LoadOptions, Page};

#[test]
fn markup_upgrade_runs_once_with_instance_state() {
    let mut page = Page::load("veyra://newtab", LoadOptions::default());
    let result = page
        .js
        .eval_script(
            r#"
            (function () {
                const log = [];
                class Base extends HTMLElement {
                    constructor() {
                        super();
                        this.__data = { x: 7 };
                        log.push('base');
                    }
                    get data() { return this.__data.x; }
                }
                class Probe extends Base {
                    constructor() {
                        super();
                        this.__own = 'ready';
                        log.push('ctor-same:' + (this === document.querySelector('upgrade-probe')));
                    }
                    get g() { return this.__data.x; }
                    get h() { return this.data; }
                    connectedCallback() {
                        log.push('connected:' + this.__own + ':' + this.g + ':' + this.h);
                    }
                }
                customElements.define('upgrade-probe', Probe);
                document.body.innerHTML = '<upgrade-probe></upgrade-probe>';
                const el = document.querySelector('upgrade-probe');
                return JSON.stringify({
                    sameCtorThis: log[1] === 'ctor-same:true',
                    own: el.__own,
                    data: el.__data.x,
                    value: el.g,
                    nested: el.h,
                    instance: el instanceof Probe,
                    log
                });
            })();
            "#,
        )
        .expect("upgrade harness failed");
    let text = page.js.to_string(&result).expect("result is a string");
    assert_eq!(
        text,
        r#"{"sameCtorThis":true,"own":"ready","data":7,"value":7,"nested":7,"instance":true,"log":["base","ctor-same:true","connected:ready:7:7"]}"#
    );
}

#[test]
fn markup_upgrade_reports_constructor_exception() {
    let mut page = Page::load("veyra://newtab", LoadOptions::default());
    page.js
        .eval_script(
            r#"
            (function () {
                class BoomProbe extends HTMLElement {
                    constructor() {
                        super();
                        throw new Error('constructor exploded during upgrade');
                    }
                }
                customElements.define('boom-probe', BoomProbe);
                document.body.innerHTML = '<boom-probe></boom-probe>';
                return document.querySelector('boom-probe') instanceof BoomProbe;
            })();
            "#,
        )
        .expect("throw harness failed");
    let logs = page.console.borrow().join("\n");
    assert!(
        logs.contains("custom element <boom-probe> upgrade failed")
            && logs.contains("constructor exploded during upgrade"),
        "constructor error was not reported; console was:\n{logs}"
    );
}
