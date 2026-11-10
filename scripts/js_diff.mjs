// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

// js_diff.mjs — différentiel explorer-jscore vs Node/V8 sur un
// corpus de snippets .js (zéro dépendance npm, Node >= 18).
//
// Chaque fichier est exécuté par `node` puis par
// `explorer-cli eval` ; les sorties stdout (console.*) et le
// statut d'erreur sont normalisés et comparés. Les divergences
// nomment les écarts sémantiques du moteur — la classe de bug
// qui tue les boot-chains réelles en vol (« mort silencieuse »).
//
// Usage :
//   node scripts/js_diff.mjs <corpus-dir|fichier.js> [--cli <binaire>]
//
//   <corpus-dir>   dossier de *.js (défaut : scripts/jsdiff_corpus)
//   --cli          chemin du binaire explorer-cli
//                  (défaut : $EXPLORER_CLI puis target/debug/
//                   explorer-cli — construire : cargo build -p explorer-cli)
//
// Sortie : une ligne PAR FICHIER DIVERGENT — `~ fichier` suivi
// des différences ; résumé final `n/N identiques`. Code de
// sortie 1 dès qu'une divergence existe (usable en gate).
//
// Notes de conception :
// - Timeout 10 s par exécution (contenu hostile = boucle
//   possible ; une divergences de timeout se reporte).
// - Normalisation : fin de ligne CRLF→LF, trim final — le reste
//   est strict (le moteur doit reproduire V8 à l'octet).
// - Les API navigateur absentes de Node (document, fetch…) ne
//   font pas partie du corpus attendu : les snippets doivent
//   rester langue+builtins (console.log compris des deux côtés).

import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, statSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const args = process.argv.slice(2);
const cliIdx = args.indexOf('--cli');
const cliArg = cliIdx >= 0 ? args[cliIdx + 1] : null;
const target = args.filter((a, i) => a !== '--cli' && (cliIdx < 0 || i !== cliIdx + 1))[0]
  ?? join(here, 'jsdiff_corpus');

const cli = cliArg
  ?? process.env.EXPLORER_CLI
  ?? join(here, '..', 'target', 'debug',
    process.platform === 'win32' ? 'explorer-cli.exe' : 'explorer-cli');
if (!existsSync(cli)) {
  console.error(`explorer-cli introuvable : ${cli}`);
  console.error('  → cargo build -p explorer-cli   (ou --cli <chemin>)');
  process.exit(2);
}

const files = statSync(target).isDirectory()
  ? readdirSync(target).filter(f => f.endsWith('.js')).sort()
      .map(f => join(target, f))
  : [target];
if (files.length === 0) {
  console.error(`aucun .js dans ${target}`);
  process.exit(2);
}

function run(cmd, argv, file) {
  try {
    const out = execFileSync(cmd, [...argv, file], {
      encoding: 'utf8',
      timeout: 10_000,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    return { code: 0, out, err: '' };
  } catch (e) {
    return {
      code: e.status ?? 'timeout',
      out: e.stdout ?? '',
      err: String(e.stderr ?? e.message).slice(0, 400),
    };
  }
}

const norm = s => s.replace(/\r\n/g, '\n').replace(/\s+$/, '');
let same = 0;
for (const f of files) {
  const a = run(process.execPath, [], f);                    // Node/V8
  const b = run(cli, ['eval'], f);                           // explorer-jscore
  const aOut = norm(a.out), bOut = norm(b.out);
  const aFail = a.code !== 0, bFail = b.code !== 0;
  if (aOut === bOut && aFail === bFail) { same++; continue; }
  console.log(`~ ${basename(f)}`);
  if (aOut !== bOut) {
    console.log(`    node  : ${JSON.stringify(aOut.slice(0, 200))}`);
    console.log(`    jscore: ${JSON.stringify(bOut.slice(0, 200))}`);
  }
  if (aFail !== bFail) {
    console.log(`    statut: node=${a.code} jscore=${b.code}  ${JSON.stringify(b.err.slice(0, 160))}`);
  }
}
console.log(`${same}/${files.length} identiques`);
process.exit(same === files.length ? 0 : 1);
