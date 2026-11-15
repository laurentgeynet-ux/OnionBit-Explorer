// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

// bcd_audit.mjs — lints de **qualification** de la jauge
// `css.values` (étape 277). La mesure reste à
// `bcd_coverage.mjs` ; ici on audite les artefacts qui
// l'ont déjà trompée — chaque lint est un bug passé devenu
// détectable mécaniquement :
//
//   [tables-mortes]      une ligne de INITIAL_EXTRA /
//                        NO_ORACLE / PAINT_NA dont la prop
//                        n'existe plus dans le catalogue BCD
//                        (renommage/retrait au bump du paquet)
//                        — variante du bug camelCase de
//                        l'étape 272 : une entrée que rien ne
//                        peut matcher. `check tables` dump les
//                        vraies tables du binaire — pas de
//                        parsing du source Rust.
//   [meta-littéraux]     une valeur BCD contenant `_` (nom de
//                        *sous-feature*, jamais un lexème CSS)
//                        qui atteint l'oracle (`decl:`/`probe:`)
//                        — régression de `decl_gate` : elle
//                        doit partir en `na` ou sonde.
//   [littéraux invalides] le littéral résolu (brut ou sonde
//                        substituée) échoue au parse/validation
//                        explorer-css (`check decl` →
//                        `invalid:`) — valeur BCD non littérale
//                        passée à l'oracle, ou sonde interne
//                        cassée (prop retirée de SUPPORTED_).
//   [fixtures inertes]   prop `check prop` ok + ≥3 valeurs
//                        mesurées + 100 % miss sous l'union
//                        affectsv∪paintv — soit un trou moteur
//                        complet (file PIPE), soit une fixture
//                        qui ne rend jamais l'effet observable
//                        (le tri est affiché, pas tranché).
//                        Nécessite la passe oracles (~30 s) —
//                        `--fast` le saute.
//   [na-creep]           part `na` par prop vs un cliché
//                        (`--na-base`) — un reclass trop large
//                        qui sortirait des valeurs mesurables
//                        du dénominateur sans motif. Écrire le
//                        cliché : `--save-na <f.json>`.
//
// Usage : node scripts/bcd_audit.mjs [--cli <binaire>]
//           [--fast] [--na-base <f.json>] [--save-na <f.json>]
//           [--limit N] [--strict]
//   --cli      explorer-cli (défaut : $EXPLORER_CLI puis
//              target/debug/explorer-cli)
//   --fast     lints sans oracle (tables + porte decl — la
//              passe affectsv∪paintv des fixtures inertes est
//              sautée, ~30 s économisés)
//   --na-base  cliché JSON `{prop: [na, total]}` de référence
//   --save-na  écrit le cliché `na` courant dans le fichier
//   --limit N  max de constats listés par lint (défaut 15)
//   --strict   exit 1 si au moins un constat

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
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
const fast = args.includes('--fast');
const strict = args.includes('--strict');
const limit = Number(arg('--limit', 15));
const naBasePath = arg('--na-base', null);
const saveNaPath = arg('--save-na', null);
const cli = arg('--cli', null)
  ?? process.env.EXPLORER_CLI
  ?? join(here, '..', 'target', 'debug',
    process.platform === 'win32' ? 'explorer-cli.exe' : 'explorer-cli');
if (!existsSync(cli)) {
  console.error(`explorer-cli introuvable : ${cli} — cargo build -p explorer-cli`);
  process.exit(2);
}

const browsers = ['chrome', 'firefox', 'safari'];

// Même dénominateur que bcd_coverage.mjs : standard-track,
// non-obsolète, livré par ≥ 1 des 3 moteurs.
const pairs = []; // {name:'p:v', prop, val, engines}
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
    pairs.push({ name: `${p}:${v}`, prop: p, val: v, engines: n });
  }
}
const bcdProps = new Set(Object.keys(bcd.css.properties));

const run = (kind, lines) => execFileSync(cli, ['check', kind], {
  input: lines.join('\n'),
  encoding: 'utf8', timeout: 600_000, maxBuffer: 64 * 1024 * 1024,
}).split(/\r?\n/).filter(Boolean);

const findings = []; // {lint, items:[{label, detail}]}
const lint = (name, items) => { if (items.length) findings.push({ lint: name, items }); };

// ── Lint 1 : tables mortes ────────────────────────────────
// `check tables` dump INITIAL_EXTRA/NO_ORACLE/PAINT_NA tels
// que le binaire les embarque — une prop absente du
// catalogue BCD = ligne que rien ne déclenchera jamais.
{
  const dead = [];
  for (const line of run('tables', [])) {
    const [table, prop] = line.split('\t');
    // Les tables stockent la forme normalisée (`nn` de
    // decl_gate épluche `-webkit-`) : `touch-callout` est
    // vivante si BCD connaît `-webkit-touch-callout`.
    if (table && prop && !bcdProps.has(prop) && !bcdProps.has(`-webkit-${prop}`)) {
      dead.push({ label: `${table}\t${prop}`, detail: 'prop absente de bcd.css.properties' });
    }
  }
  lint('tables-mortes', dead);
}

// ── Porte de la jauge : `check decl` sur toutes les paires ─
// Verdicts : na | known:false | probe:<lit> | decl:<lit> |
// invalid:<lit> — le chemin de l'union (PAINT_NA + sondes
// paint, identique à ce que la mesure applique).
const gate = new Map();
for (const line of run('decl', pairs.map(e => e.name))) {
  const i = line.indexOf('\t');
  if (i > 0) gate.set(line.slice(0, i), line.slice(i + 1));
}

// ── Lint 2 : méta-noms passés à l'oracle ──────────────────
// `probe:` = la table subfeature a correctement substitué
// un littéral — chemin nominal, pas un constat. La
// régression à attraper : `_` servi `decl:` (la porte a
// traité le méta-nom comme un lexème). `invalid:` sur `_`
// = sonde cassée → déjà couvert par littéraux-invalides.
{
  const bad = [];
  for (const e of pairs) {
    const g = gate.get(e.name) ?? '';
    if (e.val.includes('_') && g.startsWith('decl:')) {
      bad.push({ label: e.name, detail: `méta-nom traité en littéral : ${g}` });
    }
  }
  lint('meta-littéraux', bad);
}

// ── Lint 3 : littéraux invalides ──────────────────────────
{
  const bad = [];
  for (const e of pairs) {
    const g = gate.get(e.name) ?? '';
    if (g.startsWith('invalid:')) {
      bad.push({ label: e.name, detail: `littéral résolu refusé : ${g.slice(8)}` });
    }
  }
  lint('littéraux-invalides', bad);
}

// ── Stats par prop (na/total + mesurés) ───────────────────
const perProp = new Map(); // p → {na, total, measured:[names]}
for (const e of pairs) {
  const s = perProp.get(e.prop) ?? { na: 0, total: 0, measured: [] };
  s.total++;
  const g = gate.get(e.name) ?? '';
  if (g.startsWith('na')) s.na++;
  else s.measured.push(e.name);
  perProp.set(e.prop, s);
}

// ── Lint 4 : fixtures inertes (passe oracles) ─────────────
// prop reconnue + ≥3 valeurs mesurées + 100 % miss → soit un
// trou moteur complet, soit une fixture qui ne rend jamais
// l'effet observable. `--fast` saute cette passe.
if (!fast) {
  const props = [...perProp.keys()];
  const propOk = new Set(run('prop', props)
    .filter(l => l.endsWith('\tok')).map(l => l.split('\t')[0]));
  const measured = pairs.map(e => e.name)
    .filter(n => { const g = gate.get(n) ?? ''; return !g.startsWith('na') && !g.startsWith('invalid:'); });
  const geoOk = new Set(run('affectsv', measured)
    .filter(l => l.endsWith('\tok')).map(l => l.split('\t')[0]));
  const rest = measured.filter(n => !geoOk.has(n));
  const paintOk = new Set(rest.length ? run('paintv', rest)
    .filter(l => l.endsWith('\tok')).map(l => l.split('\t')[0]) : []);
  const ok = new Set([...geoOk, ...paintOk]);
  const inert = [];
  for (const [p, s] of perProp) {
    if (s.measured.length >= 3 && propOk.has(p)
      && s.measured.every(n => !ok.has(n))) {
      inert.push({
        label: p,
        detail: `prop reconnue, ${s.measured.length} valeur(s) mesurée(s), 0 ok — trou moteur ou fixture inerte`,
      });
    }
  }
  inert.sort((a, b) => a.label.localeCompare(b.label));
  lint('fixtures-inertes', inert);
}

// ── Lint 5 : na-creep vs cliché ───────────────────────────
if (naBasePath) {
  const base = JSON.parse(readFileSync(naBasePath, 'utf8'));
  const creep = [];
  for (const [p, s] of perProp) {
    const b = base[p];
    // `na` en hausse alors que le total n'a pas baissé : des
    // valeurs mesurables ont quitté le dénominateur.
    if (b && s.na > b[0] && s.total >= b[1]) {
      creep.push({ label: p, detail: `na ${b[0]}→${s.na} (total ${b[1]}→${s.total})` });
    }
  }
  creep.sort((a, b) => a.label.localeCompare(b.label));
  lint('na-creep', creep);
}
if (saveNaPath) {
  const snap = {};
  for (const [p, s] of perProp) snap[p] = [s.na, s.total];
  writeFileSync(saveNaPath, JSON.stringify(snap, null, 2) + '\n');
  console.log(`cliché na écrit → ${saveNaPath} (${perProp.size} props)`);
}

// ── Rapport ───────────────────────────────────────────────
let nFind = 0;
for (const f of findings) {
  nFind += f.items.length;
  console.log(`\n[${f.lint}] ${f.items.length}`);
  for (const it of f.items.slice(0, limit)) {
    console.log(`    ${it.label} — ${it.detail}`);
  }
  if (f.items.length > limit) {
    console.log(`    … ${f.items.length - limit} autres (--limit)`);
  }
}
console.log(`\nbcd_audit : ${nFind} constat(s) sur ${pairs.length} paires`
  + `${fast ? ' (mode --fast : passe oracles sautée)' : ''}`);
if (strict && nFind > 0) process.exit(1);
