// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

// bcd_coverage.mjs — « où en est le navigateur ? » : couverture
// mesurée du moteur croisée avec @mdn/browser-compat-data.
//
// Quatre surfaces, un verdict chacune :
//
//   props      bcd.css.properties   → `explorer-cli check prop`
//   selectors  bcd.css.selectors    → `explorer-cli check sel`
//   at-rules   bcd.css['at-rules']  → `explorer-cli check at`
//   builtins   bcd.javascript.builtins (prof. 2) → `eval` jscore
//
// Chaque ligne rapporte `x/y (z %)` avec son dénominateur —
// règle ADR-0010 : standard-track livrées par ≥1 des 3 moteurs,
// obsolètes/non-standard exclues (jamais un % sans dénominateur).
//
// Usage : node scripts/bcd_coverage.mjs [--cli <binaire>]
//           [--missing N] [--only props|sel|at|js]
//   --cli      explorer-cli (défaut : $EXPLORER_CLI puis
//              target/debug/explorer-cli — cargo build -p explorer-cli)
//   --missing  nb de manques listés par section (défaut 12,
//              0 = aucun)
//   --only     une seule surface
//
// Critères (biais documentés) :
// - props/sel/at : nom *reconnu* (prop_supported /
//   selector_supported / at_rule_supported — même biais que
//   `@supports` : la valeur n'est pas type-checkée, un pseudo
//   dormant compte) → plafond de conformité, pas plancher.
// - builtins : `globalThis.A.b` ou `A.prototype.b` défini dans
//   le moteur NU (`explorer-cli eval`) — surface langage
//   seule : les bindings navigateur (fetch, DOM…) vivent en
//   session, absents par construction → `bcd.api.*` hors
//   dénominateur.

import { execFileSync } from 'node:child_process';
import { existsSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
let bcd;
try {
  bcd = require('@mdn/browser-compat-data');
} catch {
  console.error('npm i requis dans scripts/ (@mdn/browser-compat-data)');
  process.exit(2);
}

const here = fileURLToPath(new URL('.', import.meta.url));
const args = process.argv.slice(2);
const arg = (k, d) => {
  const i = args.indexOf(k);
  return i >= 0 ? args[i + 1] : d;
};
const nMissing = Number(arg('--missing', 12));
const only = arg('--only', null);
const cli = arg('--cli', null)
  ?? process.env.EXPLORER_CLI
  ?? join(here, '..', 'target', 'debug',
    process.platform === 'win32' ? 'explorer-cli.exe' : 'explorer-cli');
if (!existsSync(cli)) {
  console.error(`explorer-cli introuvable : ${cli} — cargo build -p explorer-cli`);
  process.exit(2);
}

const browsers = ['chrome', 'firefox', 'safari'];

// Dénominateur honnête : standard-track, non-obsolète, livré
// par ≥ 1 des 3 moteurs.
function relevantEntries(tree) {
  return Object.entries(tree)
    .map(([name, node]) => {
      const c = node?.__compat;
      const n = c ? browsers.filter(br => {
        const s = c.support?.[br];
        const one = Array.isArray(s) ? s[0] : s;
        return one && one.version_added !== false && !one.version_removed;
      }).length : 0;
      return {
        name,
        relevant: !!c && !c.status?.deprecated
          && c.status?.standard_track !== false && n > 0,
        engines: n,
      };
    })
    .filter(e => e.relevant);
}

// `explorer-cli check <kind>` sur un lot de noms → Set des ok.
function checkBatch(kind, names) {
  const out = execFileSync(cli, ['check', kind], {
    input: names.join('\n'),
    encoding: 'utf8', maxBuffer: 64 * 1024 * 1024,
  });
  return new Set(out.split(/\r?\n/).filter(Boolean)
    .filter(l => l.endsWith('\tok')).map(l => l.split('\t')[0]));
}

function report(label, entries, okSet, note) {
  const ok = entries.filter(e => okSet.has(e.name)).length;
  const pct = (100 * ok / entries.length).toFixed(1);
  console.log(`${label.padEnd(10)} : ${ok}/${entries.length} (${pct} %)${note ? '  — ' + note : ''}`);
  const missing = entries.filter(e => !okSet.has(e.name))
    .sort((a, b) => b.engines - a.engines || a.name.localeCompare(b.name));
  for (const e of missing.slice(0, nMissing)) {
    console.log(`    miss ${e.name}  (moteurs: ${e.engines}/3)`);
  }
}

const sections = only ? [only] : ['props', 'sel', 'at', 'js'];

for (const sec of sections) {
  if (sec === 'props') {
    const entries = relevantEntries(bcd.css.properties);
    report('css.prop', entries, checkBatch('prop', entries.map(e => e.name)),
      `sur ${Object.keys(bcd.css.properties).length} clés BCD`);
  } else if (sec === 'sel') {
    const entries = relevantEntries(bcd.css.selectors ?? {});
    report('css.sel', entries, checkBatch('sel', entries.map(e => e.name)),
      `sur ${Object.keys(bcd.css.selectors ?? {}).length} clés BCD`);
  } else if (sec === 'at') {
    const entries = relevantEntries(bcd.css['at-rules'] ?? {});
    report('css.at', entries, checkBatch('at', entries.map(e => e.name)),
      `sur ${Object.keys(bcd.css['at-rules'] ?? {}).length} clés BCD`);
  } else if (sec === 'js') {
    // Profondeur 2 de javascript.builtins — les feuilles
    // `A.b` avec __compat (Array.from, JSON.stringify,
    // Intl.DateTimeFormat…) + les racines `A` elles-mêmes.
    const tests = []; // {name, cand:[["A","b"],["A","prototype","b"]]}
    for (const [a, nodeA] of Object.entries(bcd.javascript.builtins)) {
      if (a === '__compat' || a.startsWith('@@')) continue;
      if (nodeA.__compat) {
        const c = nodeA.__compat;
        const n = browsers.filter(br => (Array.isArray(c.support?.[br]) ? c.support[br][0] : c.support?.[br])?.version_added !== false).length;
        if (!c.status?.deprecated && c.status?.standard_track !== false && n > 0) {
          tests.push({ name: a, cand: [[a]] });
        }
      }
      for (const [b, nodeB] of Object.entries(nodeA)) {
        if (b === '__compat' || b.startsWith('@@') || !nodeB?.__compat) continue;
        const c = nodeB.__compat;
        const n = browsers.filter(br => (Array.isArray(c.support?.[br]) ? c.support[br][0] : c.support?.[br])?.version_added !== false).length;
        if (c.status?.deprecated || c.status?.standard_track === false || n === 0) continue;
        // `builtins.A.A` = le constructeur lui-même
        // (`typeof A === 'function'`), pas une prop `A.A`.
        const cand = b === a ? [[a]] : [[a, b], [a, 'prototype', b]];
        tests.push({ name: `${a}.${b}`, cand });
      }
    }
    // Un script unique : `chk` descend le chemin, `ok`/`miss`
    // par test — langage ES5 volontaire (cible = jscore).
    const src = [
      // Présence testée par `in` — lire `A.prototype.byteLength`
      // invoquerait le getter (TypeError sur récepteur
      // incompatible, réel aussi sous V8).
      'function chk(p){var o=globalThis,i=0;',
      'if(p.length===1)return typeof o[p[0]]!=="undefined";',
      'while(i<p.length-1){if(o==null)return false;o=o[p[i]];i++}',
      'return o!=null && p[i] in o}',
      'var T=' + JSON.stringify(tests.map(t => t.cand)) + ';',
      'var N=' + JSON.stringify(tests.map(t => t.name)) + ';',
      'for(var i=0;i<N.length;i++){var c=T[i],hit=false;',
      'for(var j=0;j<c.length;j++){if(chk(c[j])){hit=true;break}}',
      'console.log(N[i]+"\\t"+(hit?"ok":"miss"))}',
    ].join('\n');
    const tmp = join(tmpdir(), `bcdcov-${process.pid}.js`);
    writeFileSync(tmp, src);
    let okSet;
    try {
      const out = execFileSync(cli, ['eval', tmp], {
        encoding: 'utf8', timeout: 60_000, maxBuffer: 64 * 1024 * 1024,
      });
      okSet = new Set(out.split(/\r?\n/).filter(Boolean)
        .filter(l => l.endsWith('\tok')).map(l => l.split('\t')[0]));
    } finally {
      rmSync(tmp, { force: true });
    }
    report('js.builtins', tests.map(t => ({ name: t.name, engines: 3 })), okSet,
      'moteur nu — bindings session (fetch, DOM) hors dénominateur');
  }
}
