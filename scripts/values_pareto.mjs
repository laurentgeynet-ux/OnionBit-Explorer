// values_pareto.mjs — Pareto terrain des misses `css.values`
// (étape 299) : croise la table famille de `bcd_coverage` avec
// le corpus de vols (`Explorer-data/vols/*/*.census.json`,
// recensement `css.prop.*` de site_harvest, étapes 212/273).
//
// Question servie : « quelles familles en échec touchent le
// terrain réel ? » — `mask` à 0/30 est le plus gros trou de
// jauge, mais s'il n'apparaît sur aucun site volé la file de
// travail passe derrière une famille à 2 misses vue 1 000 fois.
//
//   node scripts/values_pareto.mjs [--cli <exe>] [--corpus <dir>]
//                                  [--top N] [--json]
//
//   --cli     explorer-cli (défaut : $EXPLORER_CLI puis
//             target/debug/explorer-cli)
//   --corpus  racine des vols (défaut : ../Explorer-data/vols)
//             — tout */​*.census.json est agrégé
//   --top N   n'affiche que les N premières familles du Pareto
//   --json    sortie JSON `{generated, corpus, families}`
//
// Verdicts par famille (colonne `gap`) :
//   rendu     ≥ 1 miss « cascade applique » (computedv) —
//             expansion/résolution au style OK, trou paint/layout
//   cascade   0 miss calculée — aveugle dès le style
//             (verbatim indécidable, expansion absente…)
//   mixte     les deux classes présentes dans la famille
//
// Le tri est par fréquence corpus (déclarations vues) —
// une famille jamais vue (sites=0, décls=0) est du volume
// de jauge : miss réel de conformité, priorité terrain basse.

import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
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
const cli = arg('--cli', null)
  ?? process.env.EXPLORER_CLI
  ?? join(here, '..', 'target', 'debug',
    process.platform === 'win32' ? 'explorer-cli.exe' : 'explorer-cli');
if (!existsSync(cli)) {
  console.error(`explorer-cli introuvable : ${cli} — cargo build -p explorer-cli`);
  process.exit(2);
}
const corpusDir = arg('--corpus', join(here, '..', 'Explorer-data', 'vols'));
const top = Number(arg('--top', 0));
const asJson = args.includes('--json');

const browsers = ['chrome', 'firefox', 'safari'];

// ── Pairs BCD (même filtre que bcd_coverage `values`) ──────
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
    pairs.push(`${p}:${v}`);
  }
}

// ── Trois passes oracle (mêmes sémantiques que bcd_coverage) ─
function checkLines(kind, names) {
  if (!names.length) return [];
  return execFileSync(cli, ['check', kind], {
    input: names.join('\n'),
    encoding: 'utf8', timeout: 600_000, maxBuffer: 64 * 1024 * 1024,
  }).split(/\r?\n/).filter(Boolean);
}
const verdicts = lines => ({
  ok: new Set(lines.filter(l => l.endsWith('\tok')).map(l => l.split('\t')[0])),
  na: new Set(lines.filter(l => l.endsWith('\tna')).map(l => l.split('\t')[0])),
});

const geo = verdicts(checkLines('affectsv', pairs));
const rest = pairs.filter(n => !geo.ok.has(n));
const paint = verdicts(checkLines('paintv', rest));
const okSet = new Set([...geo.ok, ...paint.ok]);
const naSet = new Set([...geo.na].filter(n => paint.na.has(n)));
const measured = pairs.filter(n => !naSet.has(n));
const missNames = measured.filter(n => !okSet.has(n));
const comp = verdicts(checkLines('computedv', missNames));

// Familles : prop → {miss, total, computed}
const fam = {};
for (const n of measured) {
  const p = n.slice(0, n.indexOf(':'));
  const e = (fam[p] ??= { miss: 0, total: 0, computed: 0 });
  e.total += 1;
  if (!okSet.has(n)) {
    e.miss += 1;
    if (comp.ok.has(n)) e.computed += 1;
  }
}

// ── Corpus : agrégat css.prop.* des census de vols ─────────
// sites = nb de fichiers census où la prop apparaît ;
// decls = somme des occurrences (le poids terrain).
const corpus = new Map();
let sitesTotal = 0;
if (existsSync(corpusDir)) {
  for (const dir of readdirSync(corpusDir, { withFileTypes: true })) {
    if (!dir.isDirectory()) continue;
    const dpath = join(corpusDir, dir.name);
    for (const f of readdirSync(dpath)) {
      if (!f.endsWith('.census.json')) continue;
      sitesTotal += 1;
      let c;
      try {
        c = JSON.parse(readFileSync(join(dpath, f), 'utf8')).census ?? {};
      } catch { continue; }
      for (const [k, v] of Object.entries(c)) {
        if (!k.startsWith('css.prop.')) continue;
        const p = k.slice(9);
        const e = corpus.get(p) ?? { sites: 0, decls: 0 };
        e.sites += 1;
        e.decls += Number(v) || 0;
        corpus.set(p, e);
      }
    }
  }
}

// ── Pareto : familles avec misses, triées par poids terrain ─
const rows = Object.entries(fam)
  .filter(([, e]) => e.miss > 0)
  .map(([p, e]) => {
    const c = corpus.get(p) ?? { sites: 0, decls: 0 };
    const gap = e.computed === 0 ? 'cascade'
      : e.computed === e.miss ? 'rendu' : 'mixte';
    return { prop: p, ...e, ...c, gap };
  })
  .sort((a, b) => b.decls - a.decls || b.miss - a.miss || a.prop.localeCompare(b.prop));

const terrain = rows.filter(r => r.decls > 0);
const jauge = rows.filter(r => r.decls === 0);
const sum = (rs, k) => rs.reduce((s, r) => s + r[k], 0);

if (asJson) {
  console.log(JSON.stringify({
    generated: new Date().toISOString(),
    corpus: { dir: corpusDir, census_files: sitesTotal, props: corpus.size },
    summary: {
      families_with_misses: rows.length,
      terrain_families: terrain.length,
      terrain_misses: sum(terrain, 'miss'),
      jauge_families: jauge.length,
      jauge_misses: sum(jauge, 'miss'),
      computed_applied: sum(rows, 'computed'),
    },
    families: rows,
  }, null, 2));
  process.exit(0);
}

console.log(`values_pareto — ${rows.length} familles avec misses, corpus ${sitesTotal} census (${corpus.size} props vues)`);
console.log(`  terrain : ${terrain.length} familles / ${sum(terrain, 'miss')} misses sur props présentes en vol`);
console.log(`  jauge   : ${jauge.length} familles / ${sum(jauge, 'miss')} misses sur props jamais vues`);
console.log(`  cascade applique : ${sum(rows, 'computed')} misses calculées-seules (gap rendu pur)`);
console.log('');
const shown = top > 0 ? rows.slice(0, top) : rows;
for (const r of shown) {
  console.log(
    `    ${r.prop.padEnd(34)} ${String(r.miss).padStart(2)}/${r.total}` +
    `  sites=${String(r.sites).padStart(2)}  décls=${String(r.decls).padStart(5)}` +
    `${r.computed ? `  c=${r.computed}` : '     '}  ${r.gap}`
  );
}
