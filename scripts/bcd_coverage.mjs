// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

// bcd_coverage.mjs — « où en est le navigateur ? » : couverture
// mesurée du moteur croisée avec @mdn/browser-compat-data.
//
// Quatre surfaces, un verdict chacune :
//
//   props      bcd.css.properties   → `explorer-cli check prop`
//   routed     bcd.css.properties   → `explorer-cli check routed`
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
//   --only     une seule surface (props|routed|sel|at|js)
//
// Critères (biais documentés) :
// - props/sel/at : nom *reconnu* (prop_supported /
//   selector_supported / at_rule_supported — même biais que
//   `@supports` : la valeur n'est pas type-checkée, un pseudo
//   dormant compte) → plafond de conformité, pas plancher.
//   `check prop` sature dès que `SUPPORTED_PROPS` couvre le
//   catalogue — le critère devient muet (vécu R-243 : 652/652).
// - routed : déclaration `{name: inherit}` **appliquée** dans
//   le ComputedStyle — distingue « nommé » de « consommé » ;
//   c'est le score discriminant une fois `prop` saturé.
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

const sections = only ? [only] : ['props', 'routed', 'sel', 'at', 'js'];

for (const sec of sections) {
  if (sec === 'props') {
    const entries = relevantEntries(bcd.css.properties);
    report('css.prop', entries, checkBatch('prop', entries.map(e => e.name)),
      `sur ${Object.keys(bcd.css.properties).length} clés BCD`);
  } else if (sec === 'routed') {
    const entries = relevantEntries(bcd.css.properties);
    report('css.routed', entries, checkBatch('routed', entries.map(e => e.name)),
      'déclaration appliquée — score discriminant post-saturation `prop`');
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
    // Intégration hôte non observable en moteur nu (cf. F/B plus
    // bas) — retirée du dénominateur, annoncée dans la note.
    const EXCLUDED = new Set(['Promise.incumbent_settings_object_tracking']);
    const excludedNames = tests.filter(t => EXCLUDED.has(t.name)).map(t => t.name);
    const kept = tests.filter(t => !EXCLUDED.has(t.name));
    tests.length = 0;
    tests.push(...kept);
    // Un script unique : `chk` descend le chemin, `ok`/`miss`
    // par test. Deux corrections de mesure (biais documentés) :
    //
    // - `B` : bases *abstraites* jamais exposées en global par
    //   la spec (`%TypedArray%`, `%GeneratorPrototype%`,
    //   `GeneratorFunction`, `AsyncFunction`, `%AsyncIteratorPrototype%`…)
    //   — `typeof TypedArray === "undefined"` même sous V8.
    //   La sonde les atteint par leur chemin spec, sinon elles
    //   compteraient miss pour toujours sans rien mesurer.
    // - `F` : sous-features BCD qui ne sont PAS des propriétés
    //   (`Error.cause`, `serializable_object`,
    //   `key_equality_for_zeros`, `unicode_code_point_escapes`…)
    //   — testées *comportementalement* : la fonctionnalité
    //   doit exister et marcher, pas seulement être nommée.
    //
    // `EXCLUDED` : sous-features d'intégration hôte (HTML spec)
    // non testables dans le moteur nu — hors dénominateur,
    // listées dans la note du rapport.
    const BEHAVIOR = {
      'AggregateError.errors':
        'new AggregateError([1,2],"m").errors.length===2',
      'AggregateError.serializable_object':
        'structuredClone(new AggregateError([1],"m")) instanceof AggregateError',
      'Atomics.Atomic_operations_on_non_shared_buffers':
        '(function(){var t=new Int32Array(new ArrayBuffer(8));Atomics.store(t,1,7);return Atomics.add(t,1,3)===7&&Atomics.load(t,1)===10})()',
      'Date.toTemporalInstant':
        'new Date(0).toTemporalInstant().epochMilliseconds===0',
      'Error.cause': 'new Error("x",{cause:7}).cause===7',
      'Error.serializable_object':
        'structuredClone(new Error("m")) instanceof Error',
      'EvalError.serializable_object':
        'structuredClone(new EvalError("m")) instanceof EvalError',
      'Intl.Segments':
        'new Intl.Segmenter("en",{granularity:"word"}).segment("a b").containing(0).index===0',
      'JSON.json_superset':
        'JSON.parse("\\"\\\\u2028\\"").charCodeAt(0)===0x2028',
      'Map.key_equality_for_zeros':
        'new Map().set(-0,1).get(0)===1',
      'parseInt.leading_zero_strings_as_decimal':
        'parseInt("08")===8',
      'RangeError.serializable_object':
        'structuredClone(new RangeError("m")) instanceof RangeError',
      'ReferenceError.serializable_object':
        'structuredClone(new ReferenceError("m")) instanceof ReferenceError',
      'RegExp.lastIndex':
        '(function(){var r=/a/g;r.lastIndex=3;return r.lastIndex===3})()',
      'Set.key_equality_for_zeros':
        '(function(){var s=new Set();s.add(-0);return s.has(0)})()',
      'String.unicode_code_point_escapes': "'\\u{1F600}'.length===2",
      // `typeof globalThis.undefined` no puede distinguir nada —
      // la presencia se verifica como binding global real.
      'undefined': 'typeof undefined==="undefined"',
      'SuppressedError.error':
        'new SuppressedError(1,2,"m").error===1',
      'SuppressedError.suppressed':
        'new SuppressedError(1,2,"m").suppressed===2',
      'SyntaxError.serializable_object':
        'structuredClone(new SyntaxError("m")) instanceof SyntaxError',
      'TypeError.serializable_object':
        'structuredClone(new TypeError("m")) instanceof TypeError',
      'TypedArray.constructor_without_parameters':
        'new Uint8Array().length===0',
      'TypedArray.index_properties_not_consulting_prototype':
        '(function(){var P=Object.getPrototypeOf(Uint8Array.prototype);P[0]=9;var ok=new Uint8Array(1)[0]!==9;delete P[0];return ok})()',
      'TypedArray.iterable_in_constructor':
        'new Uint8Array((function*(){yield 4;yield 5})())[1]===5',
      'TypedArray.named_properties':
        '(function(){var t=new Uint8Array(2);t.foo=7;return t.foo===7})()',
      'URIError.serializable_object':
        'structuredClone(new URIError("m")) instanceof URIError',
      'WeakMap.symbol_as_keys':
        '(function(){var s=Symbol();return new WeakMap().set(s,7).get(s)===7})()',
      'WeakSet.symbol_as_keys':
        '(function(){var s=Symbol();return new WeakSet().add(s).has(s)})()',
    };
    const src = [
      // Bases intrinsèques non-globales (spec-only paths).
      'var B={',
      // %TypedArray% no es un objeto real: BCD `TypedArray.x` =
      // "los ctors tipados tienen x" → se sondea sobre Uint8Array
      // (proto encadenado a %TypedArray%.prototype, `in` la ve).
      'TypedArray:Uint8Array,',
      'Generator:(function*(){}).prototype,',
      'GeneratorFunction:(function*(){}).constructor,',
      'AsyncGenerator:(async function*(){}).prototype,',
      'AsyncGeneratorFunction:(async function*(){}).constructor,',
      'AsyncFunction:(async function(){}).constructor,',
      'AsyncIterator:Object.getPrototypeOf(Object.getPrototypeOf((async function*(){}).prototype))',
      '};',
      // Présence testée par `in` — lire `A.prototype.byteLength`
      // invoquerait le getter (TypeError sur récepteur
      // incompatible, réel aussi sous V8).
      'function chk(p){var o=B[p[0]]||globalThis[p[0]],i=1;',
      'if(p.length===1)return typeof o!=="undefined";',
      'while(i<p.length-1){if(o==null)return false;o=o[p[i]];i++}',
      'return o!=null && p[i] in o}',
      'var T=' + JSON.stringify(tests.map(t => t.cand)) + ';',
      'var N=' + JSON.stringify(tests.map(t => t.name)) + ';',
      'var F={',
      ...Object.entries(BEHAVIOR)
        .filter(([k]) => tests.some(t => t.name === k))
        .map(([k, v]) => `${JSON.stringify(k)}:function(){try{return ${v}}catch(e){return false}},`),
      '};',
      'for(var i=0;i<N.length;i++){var hit=false;',
      'if(F[N[i]]){hit=F[N[i]]()===true}else{var c=T[i];',
      'for(var j=0;j<c.length;j++){if(chk(c[j])){hit=true;break}}}',
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
      `moteur nu — sous-features sondées comportementalement, exclues: ${excludedNames.join(', ')}`);
  }
}
