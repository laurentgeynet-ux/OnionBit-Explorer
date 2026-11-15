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
//   api        bcd.api (interfaces + membres) → `api` jscore
//              (moteur avec surface navigateur installée)
//   values     bcd.css.properties.<p>.<v> → `check affectsv` ∪ `check paintv`
//              (fixture par famille — la déclaration exacte
//              doit changer le box tree)
//   html       bcd.html.elements → `check html`
//              (oracle par élément : interface JS + membres
//              IDL + effet géométrique UA — « le tag parse »
//              ne suffit pas, étape 219)
//
// Chaque ligne rapporte `x/y (z %)` avec son dénominateur —
// règle ADR-0010 : standard-track livrées par ≥1 des 3 moteurs,
// obsolètes/non-standard exclues (jamais un % sans dénominateur).
//
// Usage : node scripts/bcd_coverage.mjs [--cli <binaire>]
//           [--missing N] [--only props|routed|sel|at|js|api|values|html]
//   --cli      explorer-cli (défaut : $EXPLORER_CLI puis
//              target/debug/explorer-cli — cargo build -p explorer-cli)
//   --missing  nb de manques listés par section (défaut 12,
//              0 = aucun)
//   --only     une seule surface (props|routed|sel|at|js|api|values|html)
//   --families tableau ok/total par prop de css.values,
//              trié par miss — la file v2 en une passe
//   --gate f   cliquet : exit 1 si un score < baseline f.json
//   --save f   écrit la baseline (attention : écrase tel quel)
//   --save-if-better f  cliquet + sauvegarde sûre : refuse
//              toute régression (scores + familles),
//              fusionne les clés, n'écrit que sur
//              progression (étape 278)
//   --history f.jsonl   ajoute {ts, sha, scores, families}
//              à un journal append-only — dérive
//              attribuable à un commit (étape 278)
//
// Baseline v2 : `{scores, families}` — `families` porte les
// compteurs ok par prop de css.values (cliquet intra-
// surface ; les fichiers v1 `{scores}` restent acceptés).
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
import { appendFileSync, existsSync, writeFileSync, rmSync, readFileSync } from 'node:fs';
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

const scores = {};
// Baseline v2 (étape 278) — compteurs `ok` par prop pour
// les surfaces à granularité familiale (css.values) :
// cliquet fin, une régression intra-surface ne peut plus
// être absorbée par la progression d'une autre famille.
const families = {};
// Totaux mesurés par famille (hors `na`) — affichage
// `--families` seulement ; la baseline ne stocke que `ok`.
const famTotals = {};
function report(label, entries, okSet, note) {
  const ok = entries.filter(e => okSet.has(e.name)).length;
  scores[label] = ok;
  const pct = (100 * ok / entries.length).toFixed(1);
  console.log(`${label.padEnd(10)} : ${ok}/${entries.length} (${pct} %)${note ? '  — ' + note : ''}`);
  const missing = entries.filter(e => !okSet.has(e.name))
    .sort((a, b) => b.engines - a.engines || a.name.localeCompare(b.name));
  for (const e of missing.slice(0, nMissing)) {
    console.log(`    miss ${e.name}  (moteurs: ${e.engines}/3)`);
  }
}

const SURFACES = ['props', 'routed', 'sel', 'at', 'js', 'api', 'values', 'html'];
// `--only <nom>` inconnu = zéro surface mesurée → gate vert
// à vide et `--save` écrirait une baseline vide : échec
// bruyant plutôt que cliquet creux.
if (only && !SURFACES.includes(only)) {
  console.error(`--only inconnu : ${only} — surfaces : ${SURFACES.join('|')}`);
  process.exit(2);
}
const sections = only ? [only] : SURFACES;

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
  } else if (sec === 'api') {
    // bcd.api.<Interface> → globalThis.Interface ; membres
    // `Interface.m` → `Interface.m` (statique) ou
    // `Interface.prototype.m`. Globals minuscules directs.
    const tests = [];
    const rel = c => {
      if (!c || c.status?.deprecated || c.status?.standard_track === false) return 0;
      return browsers.filter(br => {
        const s = c.support?.[br];
        const one = Array.isArray(s) ? s[0] : s;
        return one && one.version_added !== false && !one.version_removed;
      }).length;
    };
    // Sémantique des noms de membres BCD (étape 219 —
    // correction d'artefacts de mesure) :
    //  - `X.m_static`     → membre statique `X.m`
    //  - `X.e_event`      → handler `one` (proto ou instance)
    //  - `X.X`            → le constructeur existe
    //  - membre ordinaire → `X.m` ou `X.prototype.m` ou
    //    `new X().m` (attributs d'instance — `signal`,
    //    `aborted`… ne vivent pas sur le prototype : les
    //    sondés littéralement comptaient miss à tort).
    // `FACTORY` : classes à ctor « Illegal » dont l'objet
    // se fabrique ailleurs (AbortSignal via AbortController).
    const FACTORY = {
      AbortSignal: 'new AbortController().signal',
      AbortController: 'new AbortController()',
      // Singleton spec — les attributs d'instance
      // (`timeOrigin`, `timing`, `navigation`) vivent sur
      // `performance`, le ctor reste « Illegal ».
      Performance: 'performance',
      Navigator: 'navigator',
      Location: 'location',
      History: 'history',
      // Instances obtenues par les fabriques DOM (étape 219
      // — vérifiées non-null en moteur) : membres d'instance
      // (`length`, `data`, `value`…) mesurés sur l'objet
      // réel, pas le prototype.
      Document: 'document',
      Window: 'window',
      Element: 'document.body',
      HTMLElement: 'document.body',
      EventTarget: 'document.body',
      DOMTokenList: 'document.body.classList',
      NamedNodeMap: 'document.body.attributes',
      HTMLCollection: 'document.body.children',
      NodeList: 'document.body.childNodes',
      DocumentFragment: 'document.createDocumentFragment()',
      Selection: 'window.getSelection()',
      Geolocation: 'navigator.geolocation',
      ServiceWorkerContainer: 'navigator.serviceWorker',
      Permissions: 'navigator.permissions',
      CookieStore: 'window.cookieStore',
      FontFaceSet: 'document.fonts',
      TreeWalker: 'document.createTreeWalker(document.body)',
      NodeIterator: 'document.createNodeIterator(document.body)',
      Range: 'document.createRange()',
      XPathResult: "document.evaluate('//body',document)",
      CSSStyleDeclaration: 'document.body.style',
      PerformanceEntry: "performance.getEntriesByName('x')[0]||performance.mark('x')&&performance.getEntriesByName('x')[0]",
      MutationObserver: 'new MutationObserver(function(){})',
      IntersectionObserver: 'new IntersectionObserver(function(){})',
      ResizeObserver: 'new ResizeObserver(function(){})',
      PerformanceObserver: 'new PerformanceObserver(function(){})',
    };
    for (const [a, nodeA] of Object.entries(bcd.api ?? {})) {
      if (a === '__compat' || a.startsWith('_')) continue;
      if (rel(nodeA.__compat) > 0) tests.push({ name: a, cand: [[a]] });
      for (const [b, nodeB] of Object.entries(nodeA)) {
        if (b === '__compat' || !nodeB?.__compat || rel(nodeB.__compat) === 0) continue;
        let cand;
        if (b === a) {
          cand = [[a]];
        } else if (b === 'worker_support') {
          // Marqueur BCD « l'API existe en contexte worker » —
          // pas une propriété réelle (`X.worker_support` n'a
          // jamais existé) : sonde le ctor `Worker` (littéral
          // `in` comptait miss à tort — même biais que les
          // attributs d'instance).
          cand = [['behavior', 'typeof Worker==="function"']];
        } else if (b === 'builtin_element_support') {
          cand = [['behavior', 'typeof customElements!=="undefined"']];
        } else if (b === 'supportsText') {
          // `CSSImportRule.supportsText` = attribut d'instance.
          cand = [['new:', 'CSSImportRule', 'supportsText']];
        } else if (b.endsWith('_static')) {
          cand = [[a, b.slice(0, -7)]];
        } else if (b.endsWith('_event')) {
          const h = 'on' + b.slice(0, -6);
          cand = [[a, 'prototype', h], ['new:', a, h]];
        } else {
          cand = [[a, b], [a, 'prototype', b], ['new:', a, b]];
        }
        tests.push({ name: `${a}.${b}`, cand });
      }
    }
    const src = [
      'var F=' + JSON.stringify(FACTORY) + ';',
      'function inst(a){if(F[a])try{return eval(F[a])}catch(e){return null}',
      'try{return new globalThis[a]()}catch(e){return null}}',
      'function chk(p){',
      'if(p[0]==="behavior"){try{return !!eval(p[1])}catch(e){return false}}',
      'if(p[0]==="new:"){var o=inst(p[1]);return o!=null&&p[2] in o}',
      'var o=globalThis,i=0;',
      'if(p.length===1)return typeof o[p[0]]!=="undefined";',
      'while(i<p.length-1){if(o==null)return false;o=o[p[i]];i++}',
      'try{return o!=null && p[i] in o}catch(e){return false}}',
      'var T=' + JSON.stringify(tests.map(t => t.cand)) + ';',
      'var N=' + JSON.stringify(tests.map(t => t.name)) + ';',
      'for(var i=0;i<N.length;i++){var c=T[i],hit=false;',
      'for(var j=0;j<c.length;j++){if(chk(c[j])){hit=true;break}}',
      'console.log(N[i]+"\\t"+(hit?"ok":"miss"))}',
    ].join('\n');
    const tmp = join(tmpdir(), `bcdapi-${process.pid}.js`);
    writeFileSync(tmp, src);
    let okSet;
    try {
      const out = execFileSync(cli, ['api', tmp], {
        encoding: 'utf8', timeout: 120_000, maxBuffer: 64 * 1024 * 1024,
      });
      okSet = new Set(out.split(/\r?\n/).filter(Boolean)
        .filter(l => l.endsWith('\tok')).map(l => l.split('\t')[0]));
    } finally {
      rmSync(tmp, { force: true });
    }
    report('api', tests.map(t => ({ name: t.name, engines: 3 })), okSet,
      'surface navigateur installée (install_dom) — présence seule');
  } else if (sec === 'values') {
    // bcd.css.properties.<p>.<v> — la déclaration `p:v` change-
    // t-elle le rendu ? Conformité par feature :
    // `display:ruby` ≠ `display:grid`. Double oracle
    // (étape 274) : `affectsv` (box tree + dump display-list)
    // ∪ `paintv` (diff de framebuffers — les effets peints
    // hors du dump : filter, clip-path, transform-origin,
    // mask-*, border-image*… servis par la bitmap data-URI
    // du harnais). `na` n'est retenu que si les **deux**
    // oracles classent neutre (initiale spec / CSS-wide /
    // hors-oracle) — un `na` géométrique peignable part en
    // mesure pixel.
    const pairs = [];
    for (const [p, node] of Object.entries(bcd.css.properties)) {
      for (const [v, vn] of Object.entries(node)) {
        if (v === '__compat' || !vn?.__compat) continue;
        const c = vn.__compat;
        const n = browsers.filter(br => {
          const s = c.support?.[br];
          const one = Array.isArray(s) ? s[0] : s;
          return one && one.version_added !== false && !one.version_removed;
        }).length;
        if (c.status?.deprecated || c.status?.standard_track === false || n === 0) continue;
        pairs.push({ name: `${p}:${v}`, engines: n });
      }
    }
    const lines = execFileSync(cli, ['check', 'affectsv'], {
      input: pairs.map(e => e.name).join('\n'),
      encoding: 'utf8', timeout: 600_000, maxBuffer: 64 * 1024 * 1024,
    }).split(/\r?\n/).filter(Boolean);
    const geoOk = new Set(lines.filter(l => l.endsWith('\tok')).map(l => l.split('\t')[0]));
    const geoNa = new Set(lines.filter(l => l.endsWith('\tna')).map(l => l.split('\t')[0]));
    // Seconde passe pixel sur tout ce qui n'est pas `ok`
    // géométrique : un `miss` de dump peut être un effet
    // peint réel (union honnête des deux référentiels).
    const rest = pairs.map(e => e.name).filter(n2 => !geoOk.has(n2));
    const plines = rest.length ? execFileSync(cli, ['check', 'paintv'], {
      input: rest.join('\n'),
      encoding: 'utf8', timeout: 600_000, maxBuffer: 64 * 1024 * 1024,
    }).split(/\r?\n/).filter(Boolean) : [];
    const paintOk = new Set(plines.filter(l => l.endsWith('\tok')).map(l => l.split('\t')[0]));
    const paintNa = new Set(plines.filter(l => l.endsWith('\tna')).map(l => l.split('\t')[0]));
    const okSet = new Set([...geoOk, ...paintOk]);
    // `na` = neutre sous les DEUX oracles (initiale spec /
    // CSS-wide / famille hors-référentiel) → hors
    // dénominateur, annoncée dans la note (ADR-0010).
    const naSet = new Set([...geoNa].filter(n2 => paintNa.has(n2)));
    const measured = pairs.filter(e => !naSet.has(e.name));
    report('css.values', measured, okSet,
      `déclaration → effet géométrique ∪ pixel (affectsv + paintv, jauge non-saturante) ; ${naSet.size} neutres hors dénominateur`);
    // Cliquet par famille (étape 278) : compteurs `ok` par
    // prop — une famille qui régresse pendant qu'une autre
    // progresse reste invisible du total agrégé ; la
    // baseline v2 les verrouille une à une.
    const perProp = {};
    const perPropTotal = {};
    for (const e of measured) {
      const p = e.name.slice(0, e.name.indexOf(':'));
      perPropTotal[p] = (perPropTotal[p] ?? 0) + 1;
      perProp[p] = (perProp[p] ?? 0) + (okSet.has(e.name) ? 1 : 0);
    }
    families['css.values'] = perProp;
    famTotals['css.values'] = perPropTotal;
  } else if (sec === 'html') {
    // bcd.html.elements.<tag> — oracle de comportement par
    // élément (`check html`, étape 219) : « le tag parse »
    // est trivial, la jauge exige l'interface JS spec
    // (`createElement(tag) instanceof HTML<Name>Element`),
    // les membres IDL clés (`showModal`, `content`,
    // `value`…) ET l'effet géométrique UA (boîte produite,
    // list-item, display:none des inertes, `details` replié,
    // `slot` → display:contents). Les ~80 conteneurs
    // génériques sondent `HTMLElement` + boîte rendue.
    const entries = relevantEntries(bcd.html.elements ?? {});
    report('html.elem', entries, checkBatch('html', entries.map(e => e.name)),
      `sur ${Object.keys(bcd.html.elements ?? {}).length} clés BCD — interface + IDL + géométrie UA`);
  }
}

// `--families` — tableau ok/total par famille (css.values),
// trié par miss décroissant : la file terrain lisible sans
// ré-interroger la baseline ni grep les misses.
if (args.includes('--families')) {
  for (const [label, perProp] of Object.entries(families)) {
    const tot = famTotals[label] ?? {};
    const rows = Object.keys(perProp)
      .map(p => ({ p, ok: perProp[p], t: tot[p] ?? perProp[p] }))
      .sort((a, b) => (b.t - b.ok) - (a.t - a.ok) || a.p.localeCompare(b.p));
    const zero = rows.filter(r => r.ok === 0).length;
    const full = rows.filter(r => r.ok === r.t).length;
    console.log(`familles ${label} : ${rows.length} — ${full} pleines, ${zero} à 0 ok`);
    for (const r of rows) {
      const pct = (100 * r.ok / r.t).toFixed(1);
      console.log(`    ${r.p.padEnd(34)} ${String(r.ok).padStart(3)}/${r.t} (${pct.padStart(5)} %)${r.t - r.ok ? `  miss=${r.t - r.ok}` : ''}`);
    }
  }
}

// ── Cliquet baseline (ADR-0010) ─────────────────────────────
// `--save <f.json>` écrit les scores de cette passe ;
// `--gate <f.json>` échoue (exit 1) si un score descend sous
// la baseline — les scores flottent au gré du build, le
// cliquet les verrouille.
const saveIdx = args.indexOf('--save');
const gateIdx = args.indexOf('--gate');
// Cliquet creux : refuser save/gate sur scores vides —
// `--save` écrirait une baseline vide (tout le cliquet
// perdu) et `--gate` passerait vert sur rien.
if ((saveIdx >= 0 || gateIdx >= 0 || args.includes('--save-if-better'))
  && Object.keys(scores).length === 0) {
  console.error(`cliquet refusé : aucune surface mesurée (--only inconnu ?)`);
  process.exit(2);
}
// Charge une baseline v1 (`{scores}`) ou v2 (`{scores,
// families}`) — v1 = cliquet agrégé seul.
const loadBase = path => {
  const j = JSON.parse(readFileSync(path, 'utf8'));
  return { scores: j.scores ?? {}, families: j.families ?? {} };
};
const writeBase = (path, sc, fam) => writeFileSync(path,
  JSON.stringify({ scores: sc, families: fam }, null, 2) + '\n');

// Régressions par famille : une prop dont le compteur `ok`
// descend sous la baseline = régression même si le total
// tient. Les props absentes de la baseline (nouvelles en
// BCD ou prop nouvellement mesurée) ne bloquent pas.
const familyRegressions = base => {
  const out = [];
  for (const [label, perProp] of Object.entries(families)) {
    for (const [p, v] of Object.entries(perProp)) {
      const b = base.families?.[label]?.[p];
      if (b != null && v < b) out.push(`${label}/${p} : ${v} < ${b}`);
    }
  }
  return out;
};

if (saveIdx >= 0) {
  writeBase(args[saveIdx + 1], scores, families);
  console.log(`baseline écrite → ${args[saveIdx + 1]}`);
}
// `--save-if-better <f.json>` (étape 278) : cliquet +
// sauvegarde en un seul flag — la baseline n'est réécrite
// que si aucun score mesuré ne régresse ; fusion des clés
// (un `--only values` préserve les autres surfaces) ;
// égalité stricte = pas d'écriture (zéro churn git).
const betterIdx = args.indexOf('--save-if-better');
if (betterIdx >= 0) {
  const path = args[betterIdx + 1];
  const base = existsSync(path) ? loadBase(path) : { scores: {}, families: {} };
  const regress = Object.entries(scores)
    .filter(([k, v]) => base.scores[k] != null && v < base.scores[k]);
  const famRegress = familyRegressions(base);
  if (regress.length || famRegress.length) {
    for (const [k, v] of regress) {
      console.log(`RÉGRESSION ${k} : ${v} < ${base.scores[k]} — baseline non sauvée`);
    }
    for (const r of famRegress) console.log(`RÉGRESSION famille ${r} — baseline non sauvée`);
    process.exit(1);
  }
  const gained = Object.keys(scores).filter(k => scores[k] !== base.scores[k]);
  const famGained = Object.keys(families).length > 0;
  if (gained.length === 0 && !famGained) {
    console.log(`aucune progression — baseline inchangée (${path})`);
  } else {
    // Fusion : les surfaces non mesurées (`--only`) gardent
    // leurs compteurs, familles rafraîchies si mesurées.
    writeBase(path, { ...base.scores, ...scores },
      { ...base.families, ...families });
    console.log(`baseline re-sauvée (${gained.join(', ') || 'familles'}) → ${path}`);
  }
}
if (gateIdx >= 0) {
  const base = loadBase(args[gateIdx + 1]);
  let regressions = 0;
  for (const [k, v] of Object.entries(scores)) {
    if (base.scores[k] != null && v < base.scores[k]) {
      console.log(`RÉGRESSION ${k} : ${v} < ${base.scores[k]}`);
      regressions++;
    }
  }
  for (const r of familyRegressions(base)) {
    console.log(`RÉGRESSION famille ${r}`);
    regressions++;
  }
  if (regressions > 0) process.exit(1);
  console.log(`cliquet vert — aucune régression vs baseline`);
}

// `--history <f.jsonl>` (étape 278) : une ligne JSON par
// passe mesurée — la dérive devient attribuable à un
// commit. Append-only, jamais réécrit.
const histIdx = args.indexOf('--history');
if (histIdx >= 0) {
  let sha = null;
  try {
    sha = execFileSync('git', ['rev-parse', '--short', 'HEAD'],
      { encoding: 'utf8' }).trim();
  } catch { /* hors dépôt — sha null toléré */ }
  const line = JSON.stringify({
    ts: new Date().toISOString(), sha, scores,
    families: Object.keys(families).length ? families : undefined,
  });
  appendFileSync(args[histIdx + 1], line + '\n');
  console.log(`historique ← ${args[histIdx + 1]}`);
}
