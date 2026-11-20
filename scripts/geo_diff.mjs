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
// `--styles <site.styles.json>` (sidecar `--styles` de
// `shot`) : enrichit chaque divergent de `class` (famille de
// feature soupçonnée), `suspect` (zones WPT candidates) et
// `ancestors` (chaîne de boîtes parentes) ; les styles
// calculés utiles sont embarqués dans le geodiff — le JSON
// devient auto-porteur pour la transposition vers WPT.
//
// État de page différent : un delta |dy|/|dh| > --pagestate-tol
// (2 000 px) n'est pas un bug par élément mais un appariement
// au plus proche entre deux états de page (pub/lazy-load/DOM
// divergent — sofascore +54 000, nypost −45 000) : flag
// `page_state` + compteur dédié + pattern, sans les sortir
// du score (la page diverge réellement).
// Orphelins : `eng_only_top`/`ref_only_top` = top 20 clés
// — un ref_only massif qualifie un mur JS/hydration en un
// coup d'œil, sans lire la liste plate.
//
// Usage :
//   node scripts/geo_diff.mjs site.geom.json site.ref.json
//     [--tol 2] [--out site.geodiff.json] [--top 12]
//     [--text] [--min-matched 20] [--moved-tol 50]
//     [--pagestate-tol 2000] [--styles site.styles.json]

import { readFileSync, writeFileSync } from 'node:fs';

const args = process.argv.slice(2);
// Options qui consomment la valeur suivante — toute nouvelle
// option à valeur DOIT être ajoutée ici, sinon sa valeur
// devient un positionnel (crash ENOENT sur engFile).
const VALUE_OPTS = new Set(['--tol', '--out', '--top', '--min-matched', '--moved-tol', '--styles', '--pagestate-tol']);
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
const pageStateTol = Number(opt('pagestate-tol', 2000));
const keepText = args.includes('--text');

const eng = JSON.parse(readFileSync(engFile, 'utf8'));
const ref = JSON.parse(readFileSync(refFile, 'utf8'));
const key = (b) => `${b.tag}|${b.id || ''}|${b.cls || ''}`;

// ── Sidecar d'attribution (--styles) ─────────────────────
// `node` → {parent, tag, id, cls, computed}. La jointure
// avec geom se fait par index d'arène : les GeomBox portent
// `node` depuis l'origine, il suffisait de le conserver
// dans les entrées divergentes.
const stylesPath = opt('styles', '');
let stylesByNode = null;
if (stylesPath) {
  const sr = JSON.parse(readFileSync(stylesPath, 'utf8'));
  stylesByNode = new Map();
  for (const n of sr.nodes || []) stylesByNode.set(n.node, n);
}

// Familles de feature soupçonnées — la forme du delta +
// la nature de la boîte désignent une zone WPT candidate
// (pas un verdict : une porte d'entrée de prospection,
// la transposition reste éditoriale via wpt-scan/fetch).
const INLINE_TAGS = new Set([
  'a', 'span', 'em', 'strong', 'code', 'b', 'i', 'u', 'small',
  'sup', 'sub', 'label', 'abbr', 'cite', 'q', 'samp', 'kbd',
  'var', 'time', 'mark', 'dfn', '#text',
]);
const classify = (tag, d, computed) => {
  const disp = (computed && computed.display) || '';
  const displayDirs =
    disp === 'flex' || disp === 'inline-flex' ? ['css/css-flexbox']
    : disp === 'grid' || disp === 'inline-grid' ? ['css/css-grid']
    : disp.startsWith('table') ? ['css/css-tables', 'CSS2/tables']
    : [];
  const pack = (cls, dirs) => ({ class: cls, suspect: [...displayDirs, ...dirs] });
  const ax = { x: Math.abs(d.dx), y: Math.abs(d.dy), w: Math.abs(d.dw), h: Math.abs(d.dh) };
  // `moved` ne qualifie qu'un déplacement : si la distance
  // vient de dw/dh la boîte n'a pas bougé — c'est un écart
  // de taille, classé plus bas (box-size).
  if (d.moved && Math.max(ax.x, ax.y) >= Math.max(ax.w, ax.h)) {
    return pack('moved', ['css/css-position', 'css/css-flexbox']);
  }
  if (INLINE_TAGS.has(tag) && (ax.x > 0.5 || ax.w > 0.5)) {
    return pack('inline-measure', ['css/css-text', 'css/css-fonts', 'css/css-inline']);
  }
  if (ax.h > 0.1 && ax.x < 0.5 && ax.w < 0.5 && ax.y < 2.5) {
    return pack('line-metric', ['css/css-inline', 'css/css-text', 'css/css-fonts']);
  }
  if (ax.y > 0.5 && ax.x < 0.5 && ax.w < 0.5 && ax.h < 0.5) {
    return pack('flow-position', ['css/CSS2/margin-padding-clear', 'css/css-box', 'css/css-position']);
  }
  if (ax.w > 2 || ax.h > 2) {
    return pack('box-size', ['css/css-sizing', 'css/css-box', 'css/css-align']);
  }
  if (ax.x > 0.5) {
    return pack('position-x', ['css/css-position', 'css/css-logical', 'css/css-writing-modes']);
  }
  return pack('mixed', []);
};

// Chaîne d'ancêtres bornée (4 niveaux) — la cause d'un
// décalage vit presque toujours chez un parent (sa marge,
// sa largeur, son display) : on la lit au lieu de deviner.
const ancestorsOf = (nodeId, depth) => {
  const out = [];
  let cur = stylesByNode && stylesByNode.get(nodeId);
  while (cur && cur.parent != null && out.length < depth) {
    const p = stylesByNode.get(cur.parent);
    if (!p) break;
    out.push({ node: p.node, key: `${p.tag}|${p.id}|${p.cls}` });
    cur = p;
  }
  return out;
};

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
const engOnlyBoxes = [];
const engOnlyKeys = {}, refOnlyKeys = {};
let ok = 0, engOnly = 0, refOnly = 0, moved = 0, pageState = 0;
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
    if (best < 0) {
      engOnly++;
      engOnlyKeys[k] = (engOnlyKeys[k] || 0) + 1;
      // Orphelin moteur = boîte produite sans pendant réf —
      // conservée bornée pour attribution (élément que le
      // navigateur ne boxifie pas : display/content trou).
      if (stylesByNode && engOnlyBoxes.length < 500) {
        engOnlyBoxes.push({ key: k, node: e.node, class: 'orphan-eng' });
      }
      continue;
    }
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
    // État de page différent : au-delà de --pagestate-tol en
    // vertical, ce n'est plus un écart d'élément — la paire
    // ne représente pas le même contenu (appariement au plus
    // proche entre deux rendus). Reste compté divergent.
    if (Math.max(Math.abs(dv.d.dy), Math.abs(dv.d.dh)) > pageStateTol) {
      dv.page_state = true; pageState++;
    }
    // Attribution (--styles) : famille de feature + zone WPT
    // candidate + chaîne d'ancêtres de boîtes.
    if (stylesByNode) {
      const se = stylesByNode.get(e.node);
      dv.node = e.node;
      Object.assign(dv, classify(k.split('|')[0], { ...dv.d, moved: !!dv.moved },
        se && se.computed));
      if (dv.page_state) { dv.class = 'page-state'; dv.suspect = []; }
      dv.ancestors = ancestorsOf(e.node, 4);
    }
    divergent.push(dv);
  }
  const leftover = rb.length - used.size;
  refOnly += leftover;
  if (leftover) refOnlyKeys[k] = (refOnlyKeys[k] || 0) + leftover;
}
const matched = ok + divergent.length;

// Pareto des tags divergents — trie le lot de techno à fixer.
const tagAgg = {};
for (const dv of divergent) {
  const t = dv.key.split('|')[0];
  tagAgg[t] = (tagAgg[t] || 0) + 1;
}
const pareto = Object.entries(tagAgg).sort((a, b) => b[1] - a[1]).slice(0, topN);
// Patterns corpus : un delta quasi uniforme sur tous les
// divergents n'est pas un bug par élément — dw constant =
// écart de viewport/capture, dh constant = métrique de
// ligne globale, dy constant = décalage d'un ancêtre
// commun. Signalé pour ne pas chasser un faux bug.
const patterns = [];
// Viewport divergent = mesure biaisée, pas un bug layout :
// lobste vol897 à 1024 vs ref 1366 → tous les `dw`
// fantômes. La ref porte `viewport:"WxH"` (ref_capture) ;
// signalé bloquant pour court-circuiter la lecture.
const refVpW = Number((ref.viewport || '').split('x')[0]);
if (refVpW && eng.width && Math.abs(eng.width - refVpW) >= 4) {
  patterns.push({
    axis: 'viewport', blocking: true,
    eng_w: eng.width, ref_w: refVpW,
    delta: +(eng.width - refVpW).toFixed(1),
  });
}
// État de page différent : une fraction notable de deltas
// verticaux géants signale que les deux rendus ne sont pas
// dans le même état (mur JS, pubs, lazy-load) — lire les
// orphelins plutôt que ces paires.
if (pageState) {
  patterns.push({
    axis: 'page-state', n: pageState, of: divergent.length,
    note: `>${pageStateTol} px vertical — appariement entre états de page différents, pas un bug CSS`,
  });
}
if (divergent.length >= 4) {
  for (const axis of ['dw', 'dh', 'dy', 'dx']) {
    const hist = {};
    for (const dv of divergent) {
      const v = Math.round(dv.d[axis] * 2) / 2;
      hist[v] = (hist[v] || 0) + 1;
    }
    const [v, n] = Object.entries(hist).sort((a, b) => b[1] - a[1])[0];
    if (Math.abs(v) >= 2 && n >= Math.ceil(divergent.length * 0.6)) {
      patterns.push({ axis, delta: +v, n, of: divergent.length });
    }
  }
}
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
  // Top 20 des clés orphelines — un ref_only massif se lit
  // comme un mur JS/hydration sans parcourir la liste.
  eng_only_top: Object.entries(engOnlyKeys).sort((a, b) => b[1] - a[1]).slice(0, 20)
    .map(([key, n]) => ({ key, n })),
  ref_only_top: Object.entries(refOnlyKeys).sort((a, b) => b[1] - a[1]).slice(0, 20)
    .map(([key, n]) => ({ key, n })),
  page_state_n: pageState,
  text_dropped: textDropped, ref_truncated: refTruncated,
  score_partial: refTruncated,
  eng_doc: { w: eng.width, h: eng.height },
  ref_doc: { w: ref.doc_w, h: ref.doc_h },
  pareto: pareto.map(([tag, n]) => ({ tag, n })),
  patterns,
  divergent: divergent.slice(0, 500),
};

// Attribution (--styles) : map de styles élaguée aux nœuds
// référencés (divergents + ancêtres + orphelins moteur) —
// le geodiff devient auto-porteur pour la lecture et la
// transposition vers WPT, sans recopier tout le sidecar.
if (stylesByNode) {
  const used = new Set();
  for (const dv of result.divergent) {
    if (dv.node != null) used.add(dv.node);
    for (const a of dv.ancestors || []) used.add(a.node);
  }
  for (const o of engOnlyBoxes) used.add(o.node);
  const stylesOut = {};
  for (const n of used) {
    const s = stylesByNode.get(n);
    if (s) stylesOut[n] = s;
  }
  result.styles = stylesOut;
  if (engOnlyBoxes.length) result.eng_only_boxes = engOnlyBoxes;
}

if (out) writeFileSync(out, JSON.stringify(result, null, 1));
if (!measurable) {
  console.log(`geo : hors-mesure — ${matched} appariés < ${threshold} ` +
    `(+${engOnly} moteur-seuls, +${refOnly} réf-seuls)`);
} else {
  console.log(
    `geo : ${score}% divergents (${divergent.length}/${matched} appariés, ` +
    `${ok} ok, +${engOnly} moteur-seuls, +${refOnly} réf-seuls)` +
    `${moved ? ` — dont ${moved} déplacés (>${movedTol} px)` : ''}` +
    `${pageState ? ` — ${pageState} état-page (>${pageStateTol} px, appariement non fiable)` : ''}` +
    `${patterns.some(p => p.blocking) ? ' — VIEWPORT divergent, mesure biaisée' : ''}` +
    `${refTruncated ? ' — ref tronquée (4000), score partiel' : ''}` +
    `${textDropped ? ` — ${textDropped} #text ignorés` : ''}`);
}
for (const [tag, n] of pareto.slice(0, 6)) console.log(`      ${tag} ×${n}`);
// 2 = divergents · 4 = paire non mesurable · 0 = convergent.
process.exitCode = divergent.length ? 2 : (measurable ? 0 : 4);
