// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

// oracle_coverage.mjs — « quelle surface implémentée n'a jamais
// été sondée ? » : le recensement mécanique du corpus oracle.
//
// Deux surfaces croisées :
//   installée   Object.getOwnPropertyNames(globalThis) + statiques
//               + membres prototype (via `explorer-cli eval`)
//   sondée      sources des 769 sondes de
//               crates/explorer-jscore/tests/oracle_corpus.rs
//
// Une globale est « couverte » si son identifiant apparaît mot
// entier dans au moins une source de sonde ; un membre
// (statique ou prototype) est couvert si `.membre` / `'membre'`
// apparaît dans une source. Heuristique délibérément grossière
// — une sonde « touche » une surface, elle ne la couvre pas
// sémantiquement ; le rapport est une checklist de backlog,
// pas un score (ADR-0010 : jamais un % sans sa portée).
//
// Usage : node scripts/oracle_coverage.mjs [--cli <binaire>]
//           [--members] [--missing N] [--strict]
//   --cli      explorer-cli (défaut : $EXPLORER_CLI puis
//              target/debug/explorer-cli puis `cargo build -p
//              explorer-cli` — localise via --message-format)
//   --members  détaille les membres non sondés des globales
//              couvertes (le backlog fin : `Uint8Array.fromBase64`…)
//   --missing  nb d'entrées listées par section (défaut 40,
//              0 = aucune)
//   --strict   exit 1 si une globale installée est sans sonde

import { readFileSync, writeFileSync, existsSync, mkdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const corpusPath = join(root, 'crates', 'explorer-jscore', 'tests', 'oracle_corpus.rs');

const args = process.argv.slice(2);
const flag = (n, dflt) => {
  const i = args.indexOf(n);
  return i >= 0 ? args[i + 1] : dflt;
};
const wantMembers = args.includes('--members');
const maxMissing = Number(flag('--missing', '40'));
const strict = args.includes('--strict');

// --- 1. sources des sondes -----------------------------------

const corpus = readFileSync(corpusPath, 'utf8');
const probes = [];
const re = /^\s*\("([^"]+)"\s*,\s*"((?:[^"\\]|\\.)*)"\s*\)/gm;
let m;
while ((m = re.exec(corpus))) probes.push({ name: m[1], src: m[2] });
if (probes.length === 0) {
  console.error(`aucune sonde extraite de ${corpusPath}`);
  process.exit(2);
}
// Toutes les sources concaténées : une seule regex par nom.
const allSrc = probes.map((p) => p.src).join('\n');

// --- 2. binaire explorer-cli ---------------------------------

function resolveCli() {
  if (process.env.EXPLORER_CLI) return process.env.EXPLORER_CLI;
  const exe = process.platform === 'win32' ? 'explorer-cli.exe' : 'explorer-cli';
  const local = join(root, 'target', 'debug', exe);
  if (existsSync(local)) return local;
  // `cargo build` idempotent — le champ `executable` du message
  // JSON donne le chemin exact, quelle que soit la config
  // `build-dir` (hash par workspace).
  const r = spawnSync(
    'cargo',
    ['build', '-p', 'explorer-cli', '--message-format', 'json'],
    { cwd: root, encoding: 'utf8', maxBuffer: 1 << 26 },
  );
  if (r.error || r.status !== 0) {
    console.error(`cargo build -p explorer-cli a échoué : ${r.stderr ?? r.error}`);
    process.exit(2);
  }
  for (const line of r.stdout.split('\n')) {
    try {
      const j = JSON.parse(line);
      if (j.executable && j.executable.includes('explorer-cli')) return j.executable;
    } catch { /* ligne de progression non-JSON */ }
  }
  console.error('explorer-cli introuvable après cargo build');
  process.exit(2);
}
const cli = resolveCli();

// --- 3. surface installée ------------------------------------

const dumpSrc = `(function(){
  var out = {};
  var g = Object.getOwnPropertyNames(globalThis);
  for (var i = 0; i < g.length; i++) {
    var n = g[i], v;
    try { v = globalThis[n]; } catch (e) { continue; }
    var rec = { statics: [], proto: [] };
    try {
      if (v && (typeof v === 'object' || typeof v === 'function'))
        rec.statics = Object.getOwnPropertyNames(v);
    } catch (e) {}
    try {
      if (v && v.prototype)
        rec.proto = Object.getOwnPropertyNames(v.prototype);
    } catch (e) {}
    out[n] = rec;
  }
  console.log(JSON.stringify(out));
})();`;

const dataDir = join(root, 'Explorer-data');
mkdirSync(dataDir, { recursive: true });
const dumpFile = join(dataDir, '_oracle_surface.js');
writeFileSync(dumpFile, dumpSrc);

const run = spawnSync(cli, ['eval', dumpFile], {
  encoding: 'utf8',
  maxBuffer: 1 << 26,
});
if (run.error || run.status !== 0) {
  console.error(`eval surface a échoué : ${run.stderr ?? run.error}`);
  process.exit(2);
}
const lastLine = run.stdout.trim().split('\n').pop();
const surface = JSON.parse(lastLine);

// --- 4. croisement -------------------------------------------

const META = new Set([
  'length', 'name', 'prototype', 'constructor', '__proto__',
  'arguments', 'caller',
]);
const IGNORE = /^__|^globalThis$/;

const escRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const globalHit = (n) => new RegExp(`\\b${escRe(n)}\\b`).test(allSrc);
const memberHit = (n) =>
  new RegExp(`\\.\\s*${escRe(n)}\\b|['"\`]${escRe(n)}['"\`]`).test(allSrc);

const globals = Object.keys(surface).filter((n) => !IGNORE.test(n));
const uncoveredGlobals = globals.filter((n) => !globalHit(n)).sort();
const coveredGlobals = globals.filter((n) => globalHit(n));

// Membres non sondés des globales couvertes — `window`/`self`
// sont des alias de `globalThis` (leurs « membres » = toute la
// surface, bruit pur) ; `__*` = internes jscore.
const uncoveredMembers = [];
for (const g of coveredGlobals) {
  if (g === 'window' || g === 'self') continue;
  const rec = surface[g];
  const miss = [...rec.statics, ...rec.proto.map((x) => `.${x}`)]
    .map((x) => x.replace(/^\./, ''))
    .filter((x) => !META.has(x) && !x.startsWith('__') && !memberHit(x));
  if (miss.length) uncoveredMembers.push(`${g}: ${miss.join(', ')}`);
}

// --- 5. rapport ----------------------------------------------

console.log(`sondes analysées : ${probes.length}`);
console.log(
  `globales installées : ${globals.length} — couvertes ${coveredGlobals.length}, ` +
    `sans sonde ${uncoveredGlobals.length}`,
);
console.log('\n=== globales sans sonde ===');
const gs = maxMissing ? uncoveredGlobals.slice(0, maxMissing) : [];
for (const g of gs) {
  const rec = surface[g];
  console.log(
    `  ${g}  (${rec.statics.length} statiques, ${rec.proto.length} proto)`,
  );
}
if (maxMissing && uncoveredGlobals.length > maxMissing)
  console.log(`  … +${uncoveredGlobals.length - maxMissing}`);

if (wantMembers) {
  console.log('\n=== membres non sondés (globales couvertes) ===');
  const ms = maxMissing ? uncoveredMembers.slice(0, maxMissing) : [];
  for (const line of ms) console.log(`  ${line}`);
  if (maxMissing && uncoveredMembers.length > maxMissing)
    console.log(`  … +${uncoveredMembers.length - maxMissing} globales`);
}

if (strict && uncoveredGlobals.length) process.exit(1);
