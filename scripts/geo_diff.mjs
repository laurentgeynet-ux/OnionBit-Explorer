// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// geo_diff.mjs — comparateur géométrique boîte-à-boîte
// (étape 216, sprint 3 du plan « recensement » PIPE) :
// localise l'ÉLÉMENT qui diverge, pas le pixel.
//
//   moteur : GeometryReport (--geometry / GET /geometry)
//            {width,height,boxes:[{node,tag,id,cls,x,y,w,h}]}
//   réf.   : ref_capture {boxes:[{tag,id,cls,x,y,w,h}]}
//
// Jointure : clé `tag|id|cls`, appariement par occurrence
// ordinale (les deux listes sont en ordre document — le
// pré-ordre de pose suit l'ordre DOM). Divergence si un axe
// dépasse --tol px (défaut 2). Score = divergents / appariés
// + manques signalés séparément (un nœud absent d'un côté
// est un signal distinct : display:none, drop, shadow…).
//
// Usage :
//   node scripts/geo_diff.mjs site.geom.json site.ref.json
//     [--tol 2] [--out site.geodiff.json] [--top 12]

import { readFileSync, writeFileSync } from 'node:fs';

const args = process.argv.slice(2);
const positional = args.filter((a, i) => !a.startsWith('--') &&
  !['--tol', '--out', '--top'].includes(args[i - 1]));
const opt = (n, d) => {
  const i = args.indexOf(`--${n}`);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : d;
};
const [engFile, refFile] = positional;
if (!engFile || !refFile) {
  console.error('usage : geo_diff.mjs <engine.geom.json> <ref.ref.json> [--tol 2] [--out out.json]');
  process.exit(1);
}
const tol = Number(opt('tol', 2));
const topN = Number(opt('top', 12));
const out = opt('out', '');

const eng = JSON.parse(readFileSync(engFile, 'utf8'));
const ref = JSON.parse(readFileSync(refFile, 'utf8'));
const key = (b) => `${b.tag}|${b.id || ''}|${b.cls || ''}`;

// Groupes par clé → files d'occurrences (ordre document).
const byKey = (boxes) => {
  const m = new Map();
  for (const b of boxes) {
    const k = key(b);
    if (!m.has(k)) m.set(k, []);
    m.get(k).push(b);
  }
  return m;
};
const em = byKey(eng.boxes || []);
const rm = byKey(ref.boxes || []);

const divergent = [];
let matched = 0, engOnly = 0, refOnly = 0;
const allKeys = new Set([...em.keys(), ...rm.keys()]);
for (const k of allKeys) {
  const eb = em.get(k) || [], rb = rm.get(k) || [];
  const n = Math.max(eb.length, rb.length);
  engOnly += Math.max(0, eb.length - rb.length);
  refOnly += Math.max(0, rb.length - eb.length);
  for (let i = 0; i < n; i++) {
    const e = eb[i], r = rb[i];
    if (!e || !r) continue;
    matched++;
    const d = {
      dx: e.x - r.x, dy: e.y - r.y, dw: e.w - r.w, dh: e.h - r.h,
    };
    const bad = Math.abs(d.dx) > tol || Math.abs(d.dy) > tol ||
      Math.abs(d.dw) > tol || Math.abs(d.dh) > tol;
    if (bad) {
      divergent.push({
        key: k, i,
        eng: { x: e.x, y: e.y, w: e.w, h: e.h },
        ref: { x: r.x, y: r.y, w: r.w, h: r.h },
        d: { dx: +d.dx.toFixed(1), dy: +d.dy.toFixed(1), dw: +d.dw.toFixed(1), dh: +d.dh.toFixed(1) },
      });
    }
  }
}

// Pareto des tags divergents — trie le lot de techno à fixer.
const tagAgg = {};
for (const dv of divergent) {
  const t = dv.key.split('|')[0];
  tagAgg[t] = (tagAgg[t] || 0) + 1;
}
const pareto = Object.entries(tagAgg).sort((a, b) => b[1] - a[1]).slice(0, topN);
const score = matched ? +(divergent.length * 100 / matched).toFixed(1) : null;
const result = {
  score_pct: score, matched, divergent_n: divergent.length,
  eng_only: engOnly, ref_only: refOnly,
  eng_doc: { w: eng.width, h: eng.height },
  ref_doc: { w: ref.doc_w, h: ref.doc_h },
  pareto: pareto.map(([tag, n]) => ({ tag, n })),
  divergent: divergent.slice(0, 500),
};

if (out) writeFileSync(out, JSON.stringify(result, null, 1));
console.log(
  `geo : ${score ?? '—'}% divergents (${divergent.length}/${matched} appariés, ` +
  `+${engOnly} moteur-seuls, +${refOnly} réf-seuls)`);
for (const [tag, n] of pareto.slice(0, 6)) console.log(`      ${tag} ×${n}`);
process.exitCode = divergent.length ? 2 : 0;
