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
// Jointure : clé `tag|id|cls`, appariement SPATIAL en deux
// passes par clé (étape 287 — l'occurrence ordinale faisait
// cascader une absence en faux divergents : +567 orphelins
// lobste.rs dont 565 fantômes `#text`) :
//   1. pour chaque boîte moteur, la réf non appariée la plus
//      proche sous --tol sur les 4 axes → ok ;
//   2. les restants s'apparient au plus proche → divergent
//      (delta enregistré) ; sans pendant → orphelin.
//      Un divergent dont la distance excède --moved-tol est
//      classé `moved` (déplacement massif — boîte absente
//      d'un conteneur) : il reste compté divergent pour le
//      score et l'exit code, le compteur s'ajoute sans
//      rebranche (étape 293).
// `tag === '#text'` ignoré par défaut : le dump moteur couvre
// éléments + texte, la ref n'énumère que `body *` (éléments)
// — sans filtre chaque texte est un orphelin garanti.
// `--text` les conserve. `matched < --min-matched` → verdict
// `hors-mesure` (SPA vidée, mur), hors % agrégé ;
// `ref.boxes ≥ 4000` → `ref_truncated` + `score_partial`
// (le score ne couvre que l'échantillon sous le cap).
//
// Usage :
//   node scripts/geo_diff.mjs site.geom.json site.ref.json
//     [--tol 2] [--out site.geodiff.json] [--top 12]
//     [--text] [--min-matched 20] [--moved-tol 50]

import { readFileSync, writeFileSync } from 'node:fs';

const args = process.argv.slice(2);
// Options qui consomment la valeur suivante — toute nouvelle
// option à valeur DOIT être ajoutée ici, sinon sa valeur
// devient un positionnel (crash ENOENT sur engFile).
const VALUE_OPTS = new Set(['--tol', '--out', '--top', '--min-matched', '--moved-tol']);
const positional = args.filter((a, i) => !a.startsWith('--') &&
  !VALUE_OPTS.has(args[i - 1]));
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
const minMatched = Number(opt('min-matched', 20));
const movedTol = Number(opt('moved-tol', 50));
const keepText = args.includes('--text');

const eng = JSON.parse(readFileSync(engFile, 'utf8'));
const ref = JSON.parse(readFileSync(refFile, 'utf8'));
const key = (b) => `${b.tag}|${b.id || ''}|${b.cls || ''}`;

// `#text` : le dump couvre éléments + nœuds texte, la ref
// n'énumère que des éléments — orphelins garantis sinon.
const engBoxes = (eng.boxes || [])
  .filter(b => keepText || b.tag !== '#text');
const textDropped = (eng.boxes || []).length - engBoxes.length;
const refTruncated = (ref.boxes || []).length >= 4000;

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
const em = byKey(engBoxes);
const rm = byKey(ref.boxes || []);

// Distance Chebyshev sur les 4 axes — cohérente avec tol.
const dist = (e, r) => Math.max(
  Math.abs(e.x - r.x), Math.abs(e.y - r.y),
  Math.abs(e.w - r.w), Math.abs(e.h - r.h));

const divergent = [];
let ok = 0, engOnly = 0, refOnly = 0, moved = 0;
const allKeys = new Set([...em.keys(), ...rm.keys()]);
for (const k of allKeys) {
  const eb = em.get(k) || [], rb = rm.get(k) || [];
  const used = new Set();
  // Passe 1 — sous tolérance : la réf libre la plus proche.
  const rest = [];
  for (const e of eb) {
    let best = -1, bestD = Infinity;
    for (let j = 0; j < rb.length; j++) {
      if (used.has(j)) continue;
      const d = dist(e, rb[j]);
      if (d <= tol && d < bestD) { bestD = d; best = j; }
    }
    if (best >= 0) { used.add(best); ok++; } else rest.push(e);
  }
  // Passe 2 — appariement au plus proche : une absence ne
  // décale plus les occurrences suivantes (fin de la
  // cascade ordinale) ; sans pendant → orphelin.
  for (const e of rest) {
    let best = -1, bestD = Infinity;
    for (let j = 0; j < rb.length; j++) {
      if (used.has(j)) continue;
      const d = dist(e, rb[j]);
      if (d < bestD) { bestD = d; best = j; }
    }
    if (best < 0) { engOnly++; continue; }
    used.add(best);
    const r = rb[best];
    // Au-delà de --moved-tol ce n'est plus un nudge mais un
    // déplacement massif — flag `moved`, compté divergent.
    const dv = {
      key: k,
      eng: { x: e.x, y: e.y, w: e.w, h: e.h },
      ref: { x: r.x, y: r.y, w: r.w, h: r.h },
      d: {
        dx: +(e.x - r.x).toFixed(1), dy: +(e.y - r.y).toFixed(1),
        dw: +(e.w - r.w).toFixed(1), dh: +(e.h - r.h).toFixed(1),
      },
    };
    if (bestD > movedTol) { dv.moved = true; moved++; }
    divergent.push(dv);
  }
  refOnly += rb.length - used.size;
}
const matched = ok + divergent.length;

// Pareto des tags divergents — trie le lot de techno à fixer.
const tagAgg = {};
for (const dv of divergent) {
  const t = dv.key.split('|')[0];
  tagAgg[t] = (tagAgg[t] || 0) + 1;
}
const pareto = Object.entries(tagAgg).sort((a, b) => b[1] - a[1]).slice(0, topN);
// Mesurabilité relative : une petite page entièrement
// appariée reste mesurée (example.com : 8 paires / 12 boîtes
// ref = 66 % couvert) ; seul un faible taux de couverture
// absolue + paires rares sort le verdict `hors-mesure`
// (SPA vidée, mur — la ref est la vérité terrain).
const refN = (ref.boxes || []).length;
const threshold = refN === 0
  ? 1 : Math.min(minMatched, Math.ceil(0.5 * refN));
const measurable = matched >= threshold;
const score = measurable
  ? +(divergent.length * 100 / matched).toFixed(1) : null;
const result = {
  score_pct: score,
  verdict: measurable ? 'mesuré' : 'hors-mesure',
  matched, ok_n: ok, divergent_n: divergent.length,
  moved_n: moved,
  eng_only: engOnly, ref_only: refOnly,
  text_dropped: textDropped, ref_truncated: refTruncated,
  score_partial: refTruncated,
  eng_doc: { w: eng.width, h: eng.height },
  ref_doc: { w: ref.doc_w, h: ref.doc_h },
  pareto: pareto.map(([tag, n]) => ({ tag, n })),
  divergent: divergent.slice(0, 500),
};

if (out) writeFileSync(out, JSON.stringify(result, null, 1));
if (!measurable) {
  console.log(`geo : hors-mesure — ${matched} appariés < ${threshold} ` +
    `(+${engOnly} moteur-seuls, +${refOnly} réf-seuls)`);
} else {
  console.log(
    `geo : ${score}% divergents (${divergent.length}/${matched} appariés, ` +
    `${ok} ok, +${engOnly} moteur-seuls, +${refOnly} réf-seuls)` +
    `${moved ? ` — dont ${moved} déplacés (>${movedTol} px)` : ''}` +
    `${refTruncated ? ' — ref tronquée (4000), score partiel' : ''}` +
    `${textDropped ? ` — ${textDropped} #text ignorés` : ''}`);
}
for (const [tag, n] of pareto.slice(0, 6)) console.log(`      ${tag} ×${n}`);
// 2 = divergents · 4 = paire non mesurable · 0 = convergent.
process.exitCode = divergent.length ? 2 : (measurable ? 0 : 4);
