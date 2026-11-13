// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

// api_pareto.mjs — Pareto « clé BCD × fréquence terrain » des
// manques `api` (bcd_coverage --only api) : les 7 000 misses
// restants ne se valent pas — un membre IDL lu par les bundles
// réels du corpus vaut un chantier, un sensor jamais vu
// n'en vaut pas.
//
// Usage :
//   node scripts/api_pareto.mjs <corpus-dir> [--top N] [--cli <binaire>]
//
//   <corpus-dir>  dossier de captures (.js/.html/.mjs —
//                 vols EXPLORER_NET_RECORD, bundles sauvés,
//                 fixtures de sites).
//   --top         lignes affichées (défaut 40).
//   --cli         explorer-cli (transmis à bcd_coverage).
//
// Méthode : récupère les misses `api` via bcd_coverage
// (--missing 9000), puis compte pour chaque `A` / `A.b` les
// occurrences `\bA\b` (+ `\.\s*b\b` pour les membres) dans le
// corpus — score = occurrences membres + occurrences classe.
// Lecture seule du corpus, heuristique de bag-of-words
// documentée (les chaînes/commentaires comptent aussi —
// biais assumé : la présence du nom est le signal, pas sa
// sémantique exacte).

import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, extname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const args = process.argv.slice(2);
const arg = (k, d) => {
  const i = args.indexOf(k);
  return i >= 0 ? args[i + 1] : d;
};
const top = Number(arg('--top', 40));
const dir = args.find(a => !a.startsWith('--') && a !== arg('--cli', null) && a !== arg('--top', null))
  ?? args[0];
if (!dir || !existsSync(dir) || !statSync(dir).isDirectory()) {
  console.error('usage : node scripts/api_pareto.mjs <corpus-dir> [--top N] [--cli <binaire>]');
  process.exit(2);
}

// 1. Misses `api` — on réutilise bcd_coverage (source de
//    vérité du dénominateur + du probe).
const covArgs = [join(here, 'bcd_coverage.mjs'), '--only', 'api', '--missing', '9000'];
const cli = arg('--cli', null);
if (cli) covArgs.push('--cli', cli);
const out = execFileSync(process.execPath, covArgs, {
  encoding: 'utf8', timeout: 300_000, maxBuffer: 64 * 1024 * 1024,
});
const misses = [...out.matchAll(/^\s+miss (\S+)\s+\(moteurs: (\d)\/3\)/gm)]
  .map(m => ({ name: m[1], engines: Number(m[2]) }));
if (misses.length === 0) {
  console.error('aucun miss collecté — bcd_coverage a-t-il tourné ?');
  process.exit(2);
}

// 2. Corpus → un seul buffer (les captures du worktree sont
//    typiquement < 50 Mo ; borné à 200 Mo de lecture).
const EXT = new Set(['.js', '.mjs', '.html', '.htm', '.ts']);
const files = [];
(function walk(d) {
  for (const e of readdirSync(d, { withFileTypes: true })) {
    const p = join(d, e.name);
    if (e.isDirectory()) walk(p);
    else if (e.isFile() && EXT.has(extname(e.name)) && statSync(p).size < 32 * 1024 * 1024) files.push(p);
  }
})(dir);
let corpus = '';
for (const f of files) {
  if (corpus.length > 200 * 1024 * 1024) break;
  corpus += readFileSync(f, 'utf8');
}

// 3. Occurrences par miss : `A` nu + `A.b` **qualifié** —
//    un `.membre` non qualifié ne prouve rien (`.length`/
//    `.data` frappent n'importe quel objet — biais corrigé
//    : seule la forme `A.b` lie le membre à sa classe).
const esc = s => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const counted = misses.map(m => {
  const dot = m.name.indexOf('.');
  const cls = dot < 0 ? m.name : m.name.slice(0, dot);
  const member = dot < 0 ? null : m.name.slice(dot + 1);
  const nCls = (corpus.match(new RegExp(`\\b${esc(cls)}\\b`, 'g')) ?? []).length;
  const nMember = member
    ? (corpus.match(new RegExp(`\\b${esc(cls)}\\s*\\.\\s*${esc(member)}\\b`, 'g')) ?? []).length
    : 0;
  return { ...m, cls, member, n: nCls + nMember };
});
counted.sort((a, b) => b.n - a.n || b.engines - a.engines || a.name.localeCompare(b.name));

console.log(`api misses : ${misses.length} — corpus ${files.length} fichiers, ${(corpus.length / 1048576).toFixed(1)} Mio`);
console.log('score = occurrences de la classe `A` + du membre qualifié `A.b`\n');
for (const m of counted.slice(0, top)) {
  console.log(`${String(m.n).padStart(6)}  ${m.name.padEnd(52)} (moteurs: ${m.engines}/3)`);
}
const unseen = counted.filter(m => m.n === 0).length;
console.log(`\n${unseen}/${misses.length} misses jamais cités dans le corpus — non-prioritaires par construction.`);
