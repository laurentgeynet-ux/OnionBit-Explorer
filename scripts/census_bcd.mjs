// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

// census_bcd.mjs — croise le census moteur (`explorer-cli shot
// --report out.json`, table `census` de R-207) avec
// @mdn/browser-compat-data : le Pareto des manques n'est plus
// trié par fréquence brute mais par **impact réel** — une prop
// standard livrée par les trois moteurs pèse plus qu'une relique
// non-standard jamais supportée.
//
// Usage :
//   node scripts/census_bcd.mjs <report.json> [<report2.json>…] [--top N]
//
// Dépendance : `npm i` dans scripts/ (une fois —
// @mdn/browser-compat-data).
//
// Sortie : tableau trié `score | count | clé | support` où
// score = occurrences × poids standard (3 = les 3 moteurs,
// 2 = deux, 1 = un, 0.2 = non-standard/obsolète, 0 = absent de
// BCD → compté mais annoté `?` — souvent un faux positif du
// census, signal utile en soi).

import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
let bcd;
try {
  bcd = require('@mdn/browser-compat-data');
} catch {
  console.error('npm i requis dans scripts/ (@mdn/browser-compat-data)');
  process.exit(2);
}

const args = process.argv.slice(2);
const topIdx = args.indexOf('--top');
const top = topIdx >= 0 ? Number(args[topIdx + 1] || 30) : 30;
const paths = args.filter((a, i) => a !== '--top' && (topIdx < 0 || i !== topIdx + 1));
if (paths.length === 0) {
  console.error('usage : node scripts/census_bcd.mjs <report.json>… [--top N]');
  process.exit(2);
}

// Agrège les census de tous les reports passés.
const census = {};
for (const p of paths) {
  const j = JSON.parse(readFileSync(p, 'utf8'));
  for (const [k, v] of Object.entries(j.census ?? j)) {
    census[k] = (census[k] ?? 0) + (typeof v === 'number' ? v : 0);
  }
}

const browsers = ['chrome', 'firefox', 'safari'];

// Poids d'impact : nb de moteurs livrant la feature ; les flags
// deprecated/non-standard divisent fortement.
function weight(compat) {
  if (!compat?.__compat) return { w: 0, sup: '?' };
  const c = compat.__compat;
  const n = browsers.filter(br => {
    const s = c.support?.[br];
    const one = Array.isArray(s) ? s[0] : s;
    return one && one.version_added !== false && !one.version_removed;
  }).length;
  let w = n;
  if (c.status?.deprecated) w = Math.min(w, 1) * 0.2;
  else if (c.status && c.status.standard_track === false) w *= 0.2;
  return { w, sup: String(n) + (c.status?.deprecated ? ' obsolète' : c.status?.standard_track === false ? ' non-std' : '') };
}

// Clés census → lookup BCD.
function lookup(key) {
  const prop = key.match(/^css\.(?:prop|value\.dropped|value\.unresolved)\.(.+)$/)?.[1]
    ?? key.match(/^css\.selector\.bad\.::?(.+)$/)?.[1];
  if (prop) {
    const c = bcd.css.properties[prop]
      ?? bcd.css.selectors?.[prop]
      ?? bcd.css.properties?.[`-${prop}`];
    return weight(c);
  }
  const at = key.match(/^css\.at\.(?:unknown|unapplied)\.(.+)$/)?.[1];
  if (at) return weight(bcd.css['at-rules']?.[at]);
  return { w: -1, sup: '' }; // clés hors CSS : fréquence seule
}

const rows = Object.entries(census)
  .filter(([k]) => k.startsWith('css.'))
  .map(([k, count]) => {
    const { w, sup } = lookup(k);
    return { k, count, w, sup, score: w < 0 ? count * 0.1 : count * w };
  })
  .sort((a, b) => b.score - a.score)
  .slice(0, top);

for (const r of rows) {
  console.log(
    `${r.score.toFixed(0).padStart(7)} | ${String(r.count).padStart(6)} | ${r.k.padEnd(48)} | ${r.sup}`,
  );
}
const skipped = Object.keys(census).filter(k => !k.startsWith('css.')).length;
console.log(`— ${rows.length} lignes (top ${top}) · ${skipped} clés non-css ignorées`);
