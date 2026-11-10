// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

// bcd_coverage.mjs — « où en est le navigateur ? » : couverture
// CSS mesurée de explorer-css croisée avec
// @mdn/browser-compat-data.
//
// Alimente `explorer-cli props` des clés `bcd.css.properties`
// (un nom par ligne sur stdin, verdict `nom\tok|miss` en
// sortie) et rapporte :
//
//   CSS properties : x/y supportées (z %) — dont s standard-track
//
// suivi des manques triés par impact (moteurs livrants, obsolètes
// déclassés, absents-BCD ignorés du dénominateur : une clé BCD
// sans statut standard n'est pas un manque honnête — règle
// ADR-0010 « jamais un % sans son dénominateur »).
//
// Usage : node scripts/bcd_coverage.mjs [--cli <binaire>] [--missing N]
//   --cli      explorer-cli (défaut : $EXPLORER_CLI puis
//              target/debug/explorer-cli — cargo build -p explorer-cli)
//   --missing  nb de manques listés (défaut 40, 0 = aucun)
//
// Code de sortie : 0 (le % est une mesure, pas un cliquet — le
// cliquet vit côté tests Rust).

import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
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
const cliIdx = args.indexOf('--cli');
const missIdx = args.indexOf('--missing');
const nMissing = missIdx >= 0 ? Number(args[missIdx + 1] || 40) : 40;
const cli = (cliIdx >= 0 ? args[cliIdx + 1] : null)
  ?? process.env.EXPLORER_CLI
  ?? join(here, '..', 'target', 'debug',
    process.platform === 'win32' ? 'explorer-cli.exe' : 'explorer-cli');
if (!existsSync(cli)) {
  console.error(`explorer-cli introuvable : ${cli} — cargo build -p explorer-cli`);
  process.exit(2);
}

const browsers = ['chrome', 'firefox', 'safari'];

// Dénominateur honnête : les propriétés standard-track,
// non-obsolètes, livrées par ≥ 1 des 3 moteurs (une relique
// jamais implémentée nulle part n'est pas un manque).
const entries = Object.entries(bcd.css.properties)
  .map(([name, p]) => {
    const c = p.__compat;
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
  });
const relevant = entries.filter(e => e.relevant);

const out = execFileSync(cli, ['props'], {
  input: relevant.map(e => e.name).join('\n'),
  encoding: 'utf8', maxBuffer: 64 * 1024 * 1024,
});
const verdict = new Map(out.split(/\r?\n/).filter(Boolean)
  .map(l => l.split('\t')));

let ok = 0;
const missing = [];
for (const e of relevant) {
  if (verdict.get(e.name) === 'ok') { ok++; continue; }
  missing.push(e);
}
missing.sort((a, b) => b.engines - a.engines || a.name.localeCompare(b.name));

const pct = (100 * ok / relevant.length).toFixed(1);
console.log(`CSS properties : ${ok}/${relevant.length} reconnues (${pct} %)`);
console.log(`— dénominateur : ${relevant.length} props standard-track livrées`
  + ` par ≥1 moteur sur ${entries.length} clés BCD (critère : nom`
  + ' reconnu, même biais que `@supports` — la valeur n\'est pas'
  + ' type-checkée)');
for (const e of missing.slice(0, nMissing)) {
  console.log(`  miss ${e.name}  (moteurs: ${e.engines}/3)`);
}
