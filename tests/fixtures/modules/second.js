import { lateValue } from './late-lib.js';
const box = document.createElement('div');
box.id = 'late-module';
box.textContent = 'second module: ' + lateValue;
document.body.appendChild(box);
