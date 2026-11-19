// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Arbitrage des sondes `$262` du corpus oracle — shim test262
// minimal adossé à `node:vm` (realms réels par `createContext`,
// `detachArrayBuffer` via `transfer`). Complément de
// `oracle_audit.mjs` pour les sondes marquées NA.
// Usage : node scripts/oracle_262_audit.mjs

import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const ROOT = new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const CORPUS = ROOT + '/crates/explorer-jscore/tests/oracle_corpus.rs';
const GOLDEN = ROOT + '/crates/explorer-jscore/tests/data/oracle_golden.txt';

const corpus = readFileSync(CORPUS, 'utf8');
const re = /\("([A-Za-z0-9_]+)",\s*"((?:[^"\\]|\\.)*)"\s*\)/gs;
const srcByName = new Map();
let m;
while ((m = re.exec(corpus))) {
  const lit = m[2].replace(/\\u\{([0-9a-fA-F]+)\}/g, (_, h) =>
    JSON.stringify(String.fromCodePoint(parseInt(h, 16))).slice(1, -1));
  try { srcByName.set(m[1], JSON.parse('"' + lit + '"')); } catch { /* skip */ }
}
const golden = new Map();
for (const line of readFileSync(GOLDEN, 'utf8').split('\n')) {
  const t = line.indexOf('\t');
  if (t > 0) golden.set(line.slice(0, t), line.slice(t + 1));
}

// Shim $262 — realms réels via vm contexts (intrinsèques séparés).
const prelude = `
globalThis.$262 = {
  createRealm() {
    const ctx = vm.createContext({});
    let dead = false;
    return {
      evalScript(src) {
        if (dead) throw new TypeError('realm destroyed');
        return vm.runInContext(src, ctx);
      },
      destroy() { dead = true; },
    };
  },
  detachArrayBuffer(b) { if (!b.detached) b.transfer(); },
  evalScript(src) { return vm.runInThisContext(src); },
  gc() {},
};`;

const targets = [...srcByName.keys()].filter(n =>
  srcByName.get(n) && srcByName.get(n).includes('$262') && golden.has(n));

for (const name of targets) {
  const src = srcByName.get(name);
  let kind, text;
  try {
    const wrapped = `(function(){const vm=eval("require('node:vm')");${prelude};return eval(${JSON.stringify(src)})})()`;
    // eval direct ne marche pas pour var-global ; on exécute la sonde comme script dans un contexte où $262 est posé.
    const ctx = vm.createContext({ vm });
    vm.runInContext(prelude, ctx);
    const v = vm.runInContext(src, ctx);
    kind = 'Value';
    try { text = String(v); } catch { text = '<tostring>'; }
  } catch (e) {
    if (e instanceof SyntaxError) { kind = 'Syntax'; text = e.message || ''; }
    else if (e instanceof Error) { kind = 'Throw'; text = ((e.constructor && e.constructor.name) || 'Error') + ': ' + (e.message || ''); }
    else { kind = 'Throw'; try { text = String(e); } catch { text = '<tostring>'; } }
  }
  const got = `${kind}("${text.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n')}")`;
  const want = golden.get(name);
  console.log(`${got === want ? '✓' : '!'} ${name}\n   golden: ${want}\n   v8    : ${got}`);
}
