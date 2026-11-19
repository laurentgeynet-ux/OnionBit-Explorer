// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0
//

// Audit oracle des sondes ajoutées au golden depuis JS-C588 —
// rejoue chaque sonde du corpus via un canal moteur externe et
// compare l'issue canonique au golden gravé.
// Usage : node scripts/oracle_audit.mjs [cmd...]
//   cmd : commande sonde lue sur argv (défaut : node scripts/v8_probe.js)
//   ex. : node scripts/oracle_audit.mjs bun scripts/bun_probe.js
//         node scripts/oracle_audit.mjs node scripts/qjs_probe.mjs
import { execFileSync, execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const ROOT = new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const CORPUS = ROOT + '/crates/explorer-jscore/tests/oracle_corpus.rs';
const GOLDEN = ROOT + '/crates/explorer-jscore/tests/data/oracle_golden.txt';
const BASE = '7bc0b59'; // JS-C588 — corpus migré, baseline figée
const argv = process.argv.slice(2);
// --grep <regex> : ne rejouer que les sondes dont le nom
// matche (vagues ciblées). --worktree : diff BASE vs arbre de
// travail (bless non commité — arbitrage avant commit).
const grepIdx = argv.indexOf('--grep');
const GREP = grepIdx >= 0 ? new RegExp(argv[grepIdx + 1]) : null;
const WORKTREE = argv.includes('--worktree');
const pargv = argv.filter((a, i) =>
  a !== '--worktree' && i !== grepIdx && i !== grepIdx + 1);
const PROBE = pargv.length ? { cmd: pargv[0], args: pargv.slice(1) }
                           : { cmd: 'node', args: [ROOT + '/scripts/v8_probe.js'] };
console.log(`# oracle : ${PROBE.cmd} ${PROBE.args.join(' ')}`);

// 1) Sondes ajoutées au golden depuis BASE
const diff = execSync(
  `git -C "${ROOT}" diff ${WORKTREE ? BASE : BASE + '..HEAD'} -- crates/explorer-jscore/tests/data/oracle_golden.txt`,
  { encoding: 'utf8' }
);
const added = new Map(); // nom -> outcome gravé
for (const line of diff.split('\n')) {
  if (!line.startsWith('+') || line.startsWith('++')) continue;
  const body = line.slice(1);
  const tab = body.indexOf('\t');
  if (tab < 0) continue;
  if (GREP && !GREP.test(body.slice(0, tab))) continue;
  added.set(body.slice(0, tab), body.slice(tab + 1));
}

// 2) Sources depuis oracle_corpus.rs — paires ("nom", "src")
const corpus = readFileSync(CORPUS, 'utf8');
const srcByName = new Map();
const re = /\("([A-Za-z0-9_]+)",\s*"((?:[^"\\]|\\.)*)"\s*\)/gs;
let m;
while ((m = re.exec(corpus))) {
  // Décode le littéral Rust — compatible JSON pour les échappements usuels,
  // sauf \u{NNNN} (Rust) que JSON ne connaît pas → on convertit.
  const lit = m[2].replace(/\\u\{([0-9a-fA-F]+)\}/g, (_, h) => {
    const cp = parseInt(h, 16);
    return JSON.stringify(String.fromCodePoint(cp)).slice(1, -1);
  });
  try { srcByName.set(m[1], JSON.parse('"' + lit + '"')); }
  catch { srcByName.set(m[1], null); }
}

// 3) Exécution V8 + comparaison
const errName = /([A-Za-z]+Error)/;
const canon = (kind, text) => `${kind}("${text.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\t/g, '\\t').replace(/\r/g, '\\r')}")`;

let ok = 0, msgdiff = 0, diverge = 0, na = 0;
for (const [name, goldenOutcome] of added) {
  const src = srcByName.get(name);
  if (src === undefined) { console.log(`? ${name}\tABSENT du corpus`); diverge++; continue; }
  if (src === null) { console.log(`? ${name}\tlittéral non décodé`); na++; continue; }
  if (src.includes('$262')) { console.log(`- ${name}\tNA (\$262)`); na++; continue; }
  if (goldenOutcome.startsWith('Internal')) { console.log(`- ${name}\tNA (Internal)`); na++; continue; }
  let out;
  try {
    out = execFileSync(PROBE.cmd, [...PROBE.args], { input: src, encoding: 'utf8', timeout: 15000 });
  } catch (e) {
    console.log(`? ${name}\tcrash sonde: ${e.status}`); diverge++; continue;
  }
  const tab = out.indexOf('\t');
  const kind = out.slice(0, tab);
  const text = JSON.parse(out.slice(tab + 1));
  const v8Outcome = canon(kind, text);
  if (v8Outcome === goldenOutcome) { ok++; continue; }
  // comparaison souple : même classe d'issue + même nom d'erreur
  const gk = goldenOutcome.slice(0, goldenOutcome.indexOf('('));
  if (gk === kind) {
    const ge = goldenOutcome.match(errName)?.[1];
    const ve = text.match(errName)?.[1];
    if (ge && ge === ve) { msgdiff++; console.log(`~ ${name}\tmsg-diff (${ge})\n   golden: ${goldenOutcome.slice(0, 160)}\n   v8    : ${v8Outcome.slice(0, 160)}`); continue; }
  }
  diverge++;
  console.log(`! ${name}\tDIVERGENCE\n   golden: ${goldenOutcome.slice(0, 200)}\n   v8    : ${v8Outcome.slice(0, 200)}`);
}
console.log(`\n=== ${ok} concordants / ${msgdiff} msg-diff / ${diverge} divergents / ${na} non-arbitrables sur ${added.size} ===`);
