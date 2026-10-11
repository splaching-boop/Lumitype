#!/usr/bin/env node
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { createServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';

const targetDir = resolve(process.env.CARGO_TARGET_DIR ?? 'target');
const profile = await mkdtemp(join(tmpdir(), 'sable-input-focus-'));
const processes = [];
const sockets = [];
let logs = '';

async function until(check, description) {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    if (await check()) return;
    await delay(50);
  }
  throw new Error(`Timed out: ${description}`);
}

function start(command, args, options) {
  const child = spawn(command, args, options);
  processes.push(child);
  child.stderr?.on('data', (data) => (logs += data.toString()));
  child.on('error', (error) => (logs += `${error}\n`));
  return child;
}

async function connect(url) {
  const socket = new WebSocket(url);
  sockets.push(socket);
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, { once: true });
    socket.addEventListener('error', reject, { once: true });
  });
  let nextId = 0;
  const pending = new Map();
  socket.addEventListener('message', (event) => {
    const message = JSON.parse(event.data);
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    if (message.error || message.result.exceptionDetails) {
      request.reject(new Error(JSON.stringify(message.error ?? message.result.exceptionDetails)));
    } else {
      request.resolve(message.result.result.value);
    }
  });
  return {
    evaluate(fn, argument) {
      const id = ++nextId;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        socket.send(
          JSON.stringify({
            id,
            method: 'Runtime.evaluate',
            params: {
              expression: `(${fn})(${JSON.stringify(argument) ?? ''})`,
              returnByValue: true,
            },
          })
        );
      });
    },
  };
}

try {
  // Reuse the lockfile while building the fixture separately.
  await writeFile(join(profile, 'Cargo.lock'), await readFile('Cargo.lock'));
  await writeFile(
    join(profile, 'Cargo.toml'),
    `
[package]
name = "sable-input-focus-probe"
version = "0.1.0"
edition = "2024"
[[bin]]
name = "input_focus"
path = ${JSON.stringify(resolve('examples/input_focus.rs'))}
[dependencies]
tauri = { version = "2.11.3", default-features = false, features = ["test", "devtools"] }
tauri-runtime-cef = { path = ${JSON.stringify(resolve(process.env.CEF_FOCUS_RUNTIME ?? '.'))}, features = ["devtools"] }
winit = "=0.31.0-beta.2"
[profile.dev.package."*"]
opt-level = 3
debug-assertions = false
debug = false
`
  );
  execFileSync('cargo', ['build', '--offline', '--manifest-path', join(profile, 'Cargo.toml')], {
    env: { ...process.env, CARGO_TARGET_DIR: targetDir },
    stdio: 'inherit',
  });
  const xvfb = start(
    'Xvfb',
    ['-displayfd', '3', '-screen', '0', '1600x1000x24', '-nolisten', 'tcp'],
    {
      stdio: ['ignore', 'ignore', 'pipe', 'pipe'],
    }
  );
  let displayNumber = '';
  xvfb.stdio[3].on('data', (data) => (displayNumber += data.toString()));
  await until(() => displayNumber.includes('\n'), 'isolated X11 display');
  const display = `:${displayNumber.trim()}`;
  const socket = createServer();
  await new Promise((done) => socket.listen(0, '127.0.0.1', done));
  const port = socket.address().port;
  await new Promise((done) => socket.close(done));
  const env = {
    ...process.env,
    DISPLAY: display,
    WAYLAND_DISPLAY: '',
    CEF_FOCUS_PORT: String(port),
    XDG_DATA_HOME: join(profile, 'data'),
    XDG_CONFIG_HOME: join(profile, 'config'),
    XDG_CACHE_HOME: join(profile, 'cache'),
    LD_LIBRARY_PATH: process.env.CEF_PATH ?? join(targetDir, 'debug'),
  };
  const fixture = start(join(targetDir, 'debug/input_focus'), [], {
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let fixtureOutput = '';
  fixture.stdout.on('data', (data) => (fixtureOutput += data.toString()));
  const endpoint = `http://127.0.0.1:${port}`;
  await until(async () => {
    if (fixture.exitCode !== null) throw new Error(`Fixture exited: ${fixture.exitCode}`);
    if (!fixtureOutput.includes('FOCUS-PROBE-READY')) return false;
    return fetch(`${endpoint}/json/version`)
      .then((r) => r.ok)
      .catch(() => false);
  }, 'CEF debugging endpoint');
  const pages = {};
  await until(async () => {
    const targets = await fetch(`${endpoint}/json/list`).then((response) => response.json());
    for (const target of targets) {
      if (!target.url.startsWith('focus-probe://localhost/')) continue;
      const label = new URL(target.url).pathname.slice(1);
      if ((label === 'a' || label === 'b') && !pages[label]) {
        pages[label] = await connect(target.webSocketDebuggerUrl);
      }
    }
    return (
      pages.a &&
      pages.b &&
      (await pages.a.evaluate(() => !!document.getElementById('input'))) &&
      (await pages.b.evaluate(() => !!document.getElementById('input')))
    );
  }, 'both fixture windows');
  const xdo = (...args) => execFileSync('xdotool', args, { env }).toString().trim();
  const windows = Object.fromEntries(
    ['a', 'b'].map((label) => [label, xdo('search', '--name', `^Sable focus probe ${label}$`)])
  );
  const value = (page, id) =>
    page.evaluate((id) => {
      const node = document.getElementById(id);
      return node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement
        ? node.value
        : node.textContent;
    }, id);
  const outside = () => xdo('mousemove', '1500', '900');
  const focus = async (label) => {
    xdo('windowfocus', windows[label]);
    // Let the runtime's 150 ms native-focus probe run before sending keys.
    await delay(250);
  };
  const type = async (text) => {
    xdo('type', '--clearmodifiers', '--delay', '10', text);
    await delay(100);
  };

  for (const [id, y] of [
    ['input', 80],
    ['textarea', 180],
    ['editable', 280],
  ]) {
    await focus('a');
    xdo('mousemove', '--window', windows.a, '100', String(y));
    xdo('click', '1');
    await type('inside');
    assert.equal(await value(pages.a, id), 'inside');
    const focused = xdo('getwindowfocus');
    outside();
    await type('outside');
    assert.equal(xdo('getwindowfocus'), focused, 'pointer exit preserves keyboard focus');
    assert.equal(await value(pages.a, id), 'insideoutside', `${id}: typing after pointer exit`);
    console.log(`PASS ${id}: typing after pointer exit`);
  }

  await focus('b');
  xdo('mousemove', '--window', windows.b, '100', '80');
  xdo('click', '1');
  await type('second');
  const secondFocus = xdo('getwindowfocus');
  xdo('mousemove', '--window', windows.a, '100', '80');
  await delay(400);
  assert.equal(xdo('getwindowfocus'), secondFocus, 'inactive window does not steal focus');
  await type('outside');
  assert.equal(await value(pages.b, 'input'), 'secondoutside');
  assert.equal(await value(pages.a, 'input'), 'insideoutside');
  console.log('PASS switching windows: input follows keyboard focus');

  // winit may suppress this parent focus event.
  for (let index = 0; index < 5; index++) {
    await focus('a');
    outside();
    await type('a');
    await focus('b');
    outside();
    await type('b');
  }
  assert.equal(await value(pages.a, 'editable'), 'insideoutsideaaaaa');
  assert.equal(await value(pages.b, 'input'), 'secondoutsidebbbbb');
  console.log('PASS repeated parent focus: browser regains keyboard input');

  await focus('a');
  xdo('windowunmap', windows.a);
  await focus('b');
  xdo('windowmap', windows.a);
  outside();
  await delay(400);
  assert.equal(xdo('getwindowfocus'), secondFocus, 'showing a window does not steal focus');
  await type('visible');
  assert.equal(await value(pages.b, 'input'), 'secondoutsidebbbbbvisible');
  await focus('a');
  outside();
  await type('restored');
  assert.equal(await value(pages.a, 'editable'), 'insideoutsideaaaaarestored');
  console.log('PASS hide/show: input resumes after focus is restored');
} catch (error) {
  console.error(logs);
  throw error;
} finally {
  for (const socket of sockets) socket.close();
  for (const child of processes.reverse()) {
    if (child.exitCode !== null) continue;
    child.kill('SIGTERM');
    await Promise.race([new Promise((done) => child.once('exit', done)), delay(2000)]);
    if (child.exitCode === null) child.kill('SIGKILL');
  }
  await rm(profile, { recursive: true, force: true });
}
