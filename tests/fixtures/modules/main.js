import defaultThing, { greet, count } from './lib.js';
import * as lib from './lib.js';
import { pkgName } from 'alias/pkg.js';

class ModulePanel extends HTMLElement {
  constructor() {
    super();
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = `
      <style>
        :host { display: block; margin-top: 12px; border: 3px solid #1565c0; padding: 8px; background: #e3f2fd; }
        h2 { margin: 0; color: #0d47a1; }
        ::slotted(span) { color: #b71c1c; font-weight: bold; }
      </style>
      <h2>Module-defined panel</h2>
      <slot name="title"></slot>
    `;
  }
}
customElements.define('module-panel', ModulePanel);
const panel = document.createElement('module-panel');
panel.innerHTML = '<span slot="title">slotted from module</span>';
document.body.appendChild(panel);

const out = document.createElement('section');
out.id = 'module-out';
out.textContent = [
  defaultThing(),
  greet('module'),
  'count=' + count,
  'namespace=' + typeof lib.greet,
  pkgName,
].join(' | ');
document.body.appendChild(out);

import('./lazy.js').then((mod) => {
  const lazy = document.createElement('div');
  lazy.id = 'dynamic-import';
  lazy.textContent = 'dynamic: ' + mod.lazyValue;
  document.body.appendChild(lazy);
  window.__dynamicDone = true;
});
