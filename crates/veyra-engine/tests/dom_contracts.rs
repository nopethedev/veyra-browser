//! Plain-DOM contracts exercised by Polymer/ShadyDOM during template stamping.

use veyra_engine::{LoadOptions, Page};

fn load_page() -> Page {
    Page::load("veyra://newtab", LoadOptions::default())
}

#[test]
fn template_content_is_stable_and_stamps_independently() {
    let mut page = load_page();
    let result = page
        .js
        .eval_script(
            r#"
            (function () {
                const template = document.createElement('template');
                template.innerHTML = '<div id="a"><span id="b">one</span><em id="c">two</em></div><p id="d">three</p>';
                const firstRead = template.content;
                const secondRead = template.content;
                const first = document.importNode(secondRead, true);
                const second = document.importNode(secondRead, true);
                first.querySelector('#b').textContent = 'changed';
                first.appendChild(document.createElement('i')).id = 'e';
                const order = (node) => {
                    const tags = [];
                    for (let n = node.firstChild; n; n = n.nextSibling) tags.push(n.localName);
                    return tags;
                };
                return JSON.stringify({
                    sameContent: firstRead === secondRead,
                    firstOrder: order(first),
                    secondOrder: order(second),
                    changedOnlyFirst: [first.querySelector('#b').textContent, second.querySelector('#b').textContent],
                    firstExtra: first.querySelector('#e') !== null,
                    secondExtra: second.querySelector('#e') !== null,
                    firstIndependent: first !== second
                });
            })();
            "#,
        )
        .expect("template contract harness failed");
    let text = page.js.to_string(&result).expect("result is a string");
    assert_eq!(
        text,
        r#"{"sameContent":true,"firstOrder":["div","p","i"],"secondOrder":["div","p"],"changedOnlyFirst":["changed","one"],"firstExtra":true,"secondExtra":false,"firstIndependent":true}"#
    );
}

#[test]
fn replace_child_and_legitimate_move_do_not_self_append() {
    let mut page = load_page();
    let result = page
        .js
        .eval_script(
            r#"
            (function () {
                const parent = document.createElement('div');
                parent.innerHTML = '<template is="dom-if"><span id="old"></span></template><b id="tail"></b>';
                const old = parent.firstChild;
                const replacement = document.createElement('dom-if');
                parent.replaceChild(replacement, old);
                replacement.appendChild(old);
                const move = parent.appendChild(replacement);
                return JSON.stringify({
                    inserted: parent.children.length,
                    tags: Array.from(parent.children).map((n) => n.localName),
                    oldParent: old.parentNode.localName,
                    oldFirst: replacement.firstChild.localName,
                    returnedSame: move === replacement,
                    stillFirst: parent.firstChild === replacement
                });
            })();
            "#,
        )
        .expect("replace/move harness failed");
    let text = page.js.to_string(&result).expect("result is a string");
    assert_eq!(
        text,
        r#"{"inserted":2,"tags":["b","dom-if"],"oldParent":"dom-if","oldFirst":"template","returnedSame":true,"stillFirst":false}"#
    );
}

#[test]
fn mutation_observer_receives_child_list_records() {
    let mut page = load_page();
    page.js
        .eval_script(
            r#"
            (function () {
                window.__records = null;
                const target = document.createElement('div');
                document.body.appendChild(target);
                const observer = new MutationObserver((records) => {
                    window.__records = records.map((r) => ({
                        type: r.type,
                        added: r.addedNodes.length,
                        removed: r.removedNodes.length,
                        targetSame: r.target === target
                    }));
                });
                observer.observe(target, { childList: true });
                target.appendChild(document.createElement('span'));
            })();
            "#,
        )
        .expect("observer setup failed");
    page.pump();
    let result = page
        .js
        .eval_script("JSON.stringify(window.__records)")
        .expect("observer read failed");
    let text = page.js.to_string(&result).expect("result is a string");
    assert_eq!(
        text,
        r#"[{"type":"childList","added":1,"removed":0,"targetSame":true}]"#
    );
}

#[test]
fn inserting_a_fragment_moves_its_children_in_order() {
    let mut page = load_page();
    let result = page
        .js
        .eval_script(
            r#"
            (function () {
                const host = document.createElement('div');
                document.body.appendChild(host);
                const source = document.createElement('template');
                source.innerHTML = '<span id="one"></span><b id="two"></b><i id="three"></i>';
                const fragment = source.content;
                host.insertBefore(fragment, null);
                const viaInsert = Array.from(host.children).map((n) => n.id);
                const empty = fragment.children.length;
                const second = document.createElement('template');
                second.innerHTML = '<em id="four"></em><u id="five"></u>';
                host.insertBefore(second.content, host.firstChild);
                return JSON.stringify({
                    viaInsert: viaInsert,
                    emptyAfterInsert: empty,
                    final: Array.from(host.children).map((n) => n.id)
                });
            })();
            "#,
        )
        .expect("fragment insertion harness failed");
    let text = page.js.to_string(&result).expect("result is a string");
    assert_eq!(
        text,
        r#"{"viaInsert":["one","two","three"],"emptyAfterInsert":0,"final":["four","five","one","two","three"]}"#
    );
}

#[test]
fn attributes_expose_named_node_map_lookup() {
    let mut page = load_page();
    let result = page
        .js
        .eval_script(
            r#"
            (function () {
                const el = document.createElement('div');
                el.setAttribute('id', 'root');
                el.setAttribute('data-title', 'Hello');
                const attrs = el.attributes;
                return JSON.stringify({
                    length: attrs.length,
                    firstName: attrs[0].name,
                    itemTitle: attrs.item(1).value,
                    outOfRange: attrs.item(99),
                    named: attrs.getNamedItem('data-title').value,
                    namedMissing: attrs.getNamedItem('missing'),
                    namespaced: attrs.getNamedItemNS(null, 'data-title').value,
                    local: attrs[1].localName
                });
            })();
            "#,
        )
        .expect("NamedNodeMap harness failed");
    let text = page.js.to_string(&result).expect("result is a string");
    assert_eq!(
        text,
        r#"{"length":2,"firstName":"id","itemTitle":"Hello","outOfRange":null,"named":"Hello","namedMissing":null,"namespaced":"Hello","local":"data-title"}"#
    );
}

#[test]
fn shadow_root_inner_html_can_be_replaced_by_a_sink_mount() {
    let mut page = load_page();
    let result = page
        .js
        .eval_script(
            r#"
            (function () {
                const host = document.createElement('div');
                document.body.appendChild(host);
                const root = host.attachShadow({ mode: 'open' });
                root.innerHTML = '<div id="server"><span id="skeleton"></span></div>';
                // A renderer may hand the host back among its own children.
                // That append is a no-op and must never make the host a child
                // of itself or abort the surrounding mount.
                host.appendChild(host);
                const replacement = document.createElement('section');
                replacement.id = 'mounted';
                replacement.innerHTML = '<b id="content">rendered</b>';
                root.replaceChild(replacement, root.firstChild);
                return JSON.stringify({
                    shadowMode: root.mode,
                    mounted: root.firstChild.id,
                    serverGone: root.querySelector('#server') === null,
                    content: root.querySelector('#content').textContent,
                    selfChild: host.firstChild === host,
                    stillConnected: host.isConnected
                });
            })();
            "#,
        )
        .expect("shadow sink harness failed");
    let text = page.js.to_string(&result).expect("result is a string");
    assert_eq!(
        text,
        r#"{"shadowMode":"open","mounted":"mounted","serverGone":true,"content":"rendered","selfChild":false,"stillConnected":true}"#
    );
}
