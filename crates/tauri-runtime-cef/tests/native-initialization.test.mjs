import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { test } from 'node:test';

const rust = readFileSync(fileURLToPath(new URL('../src/webview.rs', import.meta.url)), 'utf8');
const functionBody = rust.slice(rust.indexOf('fn devtools_initialization_script_source('));
const template = functionBody.match(/r#"([\s\S]*?)"#/)[1];
const init = 'Object.defineProperty(window, "postMessage", {value: () => {}});';

// Exercise the emitted URL guard with a non-configurable bootstrap property.
const source = template
  .replace('{custom_protocol}', '"http:"')
  .replace('{custom_domains}', '["tauri.localhost", "ipc.localhost", "custom.localhost"]')
  .replaceAll('{{', '{').replaceAll('}}', '}') +
  `if (!__TAURI_CEF_INIT_IS_CUSTOM_PROTOCOL__ && __TAURI_CEF_INIT_IS_MAIN_FRAME__) { ${init} }\n}`;

for (const [protocol, hostname, injected] of [
  ['tauri:', 'localhost', true],
  ['tauri:', 'app', true],
  ['custom:', 'localhost', true],
  ['http:', 'tauri.localhost', true],
  ['http:', 'localhost', false],
  ['https:', 'example.org', false],
]) {
  test(`${protocol}//${hostname}: bootstrap runs once`, () => {
    const window = {};
    window.top = window;
    const context = vm.createContext({window, location: {protocol, hostname}});
    vm.runInContext(source, context);
    if (injected) vm.runInContext(init, context);
    assert.equal(typeof window.postMessage, 'function');
  });
}

test('a main-frame-only bootstrap does not run in an external iframe', () => {
  const window = {top: {}};
  vm.runInContext(source, vm.createContext({window, location: {protocol: 'https:', hostname: 'example.org'}}));
  assert.equal(window.postMessage, undefined);
});
