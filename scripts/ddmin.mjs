// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ddmin.mjs — minimiseur différentiel (étape 217, sprint 3
// du plan « recensement » PIPE) : bissection DOM/CSS vers un
// repro minimal — convertit « ce site casse » en cas de
// ~20 lignes qui peut entrer dans tests/ sans humain.
//
// Principe : le HTML candidat est servi en local, chaque
// variante est évaluée par `explorer-cli shot --report` et
// le prédicat (--grep regex sur le JSON census) décide si la
// divergence persiste. ddmin classique : retirer des chunks,
// raffiner la granularité tant que ça tient — fixpoint, puis
// deuxième passe à l'intérieur des <style> survivants
// (déclarations CSS comme chunks).
//
// Usage :
//   node scripts/ddmin.mjs --src page.html --grep 'css.at.unapplied'
//     [--cli <explorer-cli>] [--out min.html] [--viewport 1366x900]
//     [--rounds 3] [--keep-scripts]
//
// Le prédicat doit être VRAI sur la page entière — sinon la
// minimisation n'a pas de sens (le script le vérifie d'abord).

import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { readFileSync, writeFileSync, mkdtempSync, rmSync, unlinkSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

const args = process.argv.slice(2);
const opt = (n, d) => {
  const i = args.indexOf(`--${n}`);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : d;
};
const src = opt('src', '');
const grep = opt('grep', '');
const outFile = opt('out', 'min.html');
const viewport = opt('viewport', '1366x900');
const rounds = Number(opt('rounds', 3));
const keepScripts = args.includes('--keep-scripts');
let cli = opt('cli', '');
if (!src || !grep) {
  console.error('usage : ddmin.mjs --src page.html --grep <regex> [--cli exe] [--out min.html]');
  process.exit(1);
}
if (!cli) {
  const cands = [
    process.env.EXPLORER_CLI,
    'C:/cargo-target/OninionBit-Explorer-YoutubeFix/target/x86_64-pc-windows-msvc/release/explorer-cli.exe',
  ];
  cli = cands.find((c) => c && readFileSync(c));
  if (!cli) { console.error('explorer-cli introuvable — --cli <chemin>'); process.exit(1); }
}

// --- découpage en blocs top-level (tolérant web cassé) ------
// Raw-text ignoré comme structure : script/style/textarea/
// commentaires/doctype. Un « bloc » = élément top-level
// complet (du <tag au > du close correspondant) ou run de
// texte/balises void entre deux éléments.
const VOID = new Set(['area','base','br','col','embed','hr','img','input',
  'link','meta','param','source','track','wbr']);
const RAW = new Set(['script','style','textarea','title']);

function blocks(html) {
  const out = [];
  let i = 0, depth = 0, start = 0;
  const re = /<!--|<!doctype[^>]*>|<\/?([a-zA-Z][^>\s/]*)([^>]*)>/g;
  let m;
  const push = (end) => {
    const s = html.slice(start, end);
    if (s.trim()) out.push(s);
    start = end;
  };
  while ((m = re.exec(html))) {
    if (m[0].startsWith('<!--')) { re.lastIndex = html.indexOf('-->', m.index) + 3 || m.index + 4; continue; }
    const tag = (m[1] || '').toLowerCase();
    const closing = m[0][1] === '/';
    if (!tag) continue;
    if (!closing) {
      if (depth === 0) push(m.index);
      depth++;
      if (VOID.has(tag) || m[0].endsWith('/>')) depth--;
      else if (RAW.has(tag)) {
        const end = html.indexOf(`</${tag}`, re.lastIndex);
        if (end >= 0) { re.lastIndex = end; depth--; }
      }
    } else {
      depth = Math.max(0, depth - 1);
      if (depth === 0) push(re.lastIndex);
    }
  }
  push(html.length);
  return out.filter((b) => b.trim());
}

function cssDecls(css) {
  // Déclarations top-level : split sur `}` — les blocs
  // @media embarquent leurs accolades (granularité grossière
  // assumée : un at-bloc = un chunk entier).
  const out = [];
  let depth = 0, start = 0;
  for (let i = 0; i <= css.length; i++) {
    const c = css[i];
    if (c === '{') depth++;
    if (c === '}') { depth = Math.max(0, depth - 1); if (depth === 0) { out.push(css.slice(start, i + 1)); start = i + 1; } }
    if (i === css.length && start < i) { const t = css.slice(start); if (t.trim()) out.push(t); }
  }
  return out.filter((d) => d.trim());
}

// --- serveur local + prédicat --------------------------------
const work = mkdtempSync(join(tmpdir(), 'ddmin-'));
let current = '';
const server = createServer((_q, res) => {
  res.setHeader('content-type', 'text/html; charset=utf-8');
  res.end(current);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
const url = `http://127.0.0.1:${port}/`;
const repPath = join(work, 'rep.json');
const pngPath = join(work, 'out.png');
let evals = 0;

// IMPORTANT : `spawn` asynchrone — `spawnSync` gèlerait
// l'event loop et le serveur HTTP local ne répondrait plus
// (deadlock observé : « timeout io après 30000 ms »).
function shotEval() {
  return new Promise((resolve) => {
    const p = spawn(cli,
      ['shot', url, pngPath, viewport, '--report', repPath],
      { stdio: ['ignore', 'pipe', 'pipe'] });
    let errTail = '';
    p.stderr.on('data', (d) => { errTail = (errTail + d).slice(-400); });
    p.stdout.on('data', () => {});
    const t = setTimeout(() => { try { p.kill(); } catch {} resolve({ code: 'timeout', errTail }); }, 90000);
    p.on('exit', (code) => { clearTimeout(t); resolve({ code, errTail }); });
    p.on('error', (e) => { clearTimeout(t); resolve({ code: 'spawn', errTail: e.message }); });
  });
}

async function holds(html) {
  current = html;
  evals++;
  // Purge : un rep.json périmé de l'éval précédente ferait un
  // faux positif — on ne lit que le rapport de CETTE nav.
  try { unlinkSync(repPath); } catch {}
  const { code, errTail } = await shotEval();
  let body = '';
  try { body = readFileSync(repPath, 'utf8'); } catch { body = ''; }
  if (!body) {
    if (evals === 1) {
      console.error(`  éval 1 : shot sans rapport — exit=${code}` +
        ` stderr=…${errTail.slice(-250)}`);
    }
    return false;
  }
  return new RegExp(grep).test(body);
}

// --- ddmin ---------------------------------------------------
async function ddmin(items, keep) {
  let n = 2;
  while (items.length >= 2) {
    const size = Math.ceil(items.length / n);
    let reduced = false;
    for (let i = 0; i < items.length; i += size) {
      const sans = items.filter((_, j) => j < i || j >= i + size);
      if (await keep(sans.join(''))) {
        items = sans; n = Math.max(2, n - 1); reduced = true; break;
      }
      const only = items.slice(i, i + size);
      if (await keep(only.join(''))) {
        items = only; n = 2; reduced = true; break;
      }
    }
    if (!reduced) { if (n >= items.length) break; n = Math.min(items.length, n * 2); }
  }
  return items;
}

const html0 = readFileSync(src, 'utf8');
console.log(`ddmin : ${html0.length} octets, ${blocks(html0).length} blocs — vérif initiale…`);
if (!await holds(html0)) {
  console.error(`prédicat /${grep}/ FAUX sur la page entière — rien à minimiser`);
  server.close(); rmSync(work, { recursive: true, force: true });
  process.exit(2);
}

// Scripts exclus par défaut : le prédicat census CSS/HTML
// vise la statique — un <script> conservé changerait le
// document servi entre deux évals. --keep-scripts les garde.
let items = blocks(html0);
if (!keepScripts) {
  const noJs = items.filter((b) => !/^<script[\s>]/i.test(b));
  if (await holds(noJs.join(''))) items = noJs;
}

for (let r = 0; r < rounds; r++) {
  const before = items.length;
  items = await ddmin(items, (h) => holds(h));
  if (items.length === before) break;
}
let result = items.join('');

// Passe 2 : déclarations CSS à l'intérieur des <style>.
const styleSpans = [];
{
  const styleRe = /<style[^>]*>([\s\S]*?)<\/style>/gi;
  let sm;
  while ((sm = styleRe.exec(result))) {
    styleSpans.push({ index: sm.index, full: sm[0], css: sm[1] });
  }
}
for (const sp of styleSpans) {
  const decls = cssDecls(sp.css);
  if (decls.length < 2) continue;
  const kept = await ddmin(decls, async (inner) => {
    const cand = result.slice(0, sp.index) +
      `<style>${inner}</style>` + result.slice(sp.index + sp.full.length);
    return holds(cand);
  });
  result = result.slice(0, sp.index) + `<style>${kept.join('')}</style>` +
    result.slice(sp.index + sp.full.length);
}

writeFileSync(outFile, result);
console.log(`ddmin : ${html0.length} → ${result.length} octets (${evals} évals)`);
console.log(`        -> ${outFile}`);
server.close();
try { rmSync(work, { recursive: true, force: true }); } catch {}
