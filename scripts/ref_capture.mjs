// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// ref_capture.mjs — oracle de référence batch (étape 213 —
// sprint 2/3 du plan « recensement des features » PIPE).
//
// Rend chaque URL du corpus dans Chrome/Chromium réel via le
// protocole DevTools (CDP) — zéro dépendance npm : WebSocket
// natif Node ≥ 22 + API CDP brute. Sortie par site :
//
//   <nom>.ref.png   capture viewport (pour `explorer-cli diff`)
//   <nom>.ref.json  géométrie de référence — scrollWidth/Height
//                   + getBoundingClientRect() par élément
//                   (diff géométrique du sprint 3 : localise
//                   l'élément qui diverge, pas le pixel)
//
// Le nom de fichier suit la règle de site_harvest.ps1 —
// `url` sans schéma, hors [A-Za-z0-9_.-] → '_'.
//
// Usage :
//   node scripts/ref_capture.mjs --list sites.txt --out refs/
//     [--viewport 1366x900] [--timeout 45000] [--settle 2000]
//     [--exe "C:\...\chrome.exe"] [--max N]
//     [--cookie "SOCS=CAI@.youtube.com"]  (répétable, pré-nav)
//   node scripts/ref_capture.mjs https://example.com --out refs/

import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdirSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';

const args = process.argv.slice(2);
function opt(name, dflt) {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : dflt;
}
const listFile = opt('list', '');
const outDir = resolve(opt('out', 'target/harvest/refs'));
const [vw, vh] = (opt('viewport', '1366x900').split('x').map(Number));
const timeoutMs = Number(opt('timeout', 45000));
const settleMs = Number(opt('settle', 2000));
const maxSites = Number(opt('max', 0));
const exeOpt = opt('exe', '');
// `--cookie nom=valeur@domaine` répétable — posés via CDP avant
// la première navigation (mur de consentement EU : `SOCS=CAI@.youtube.com`).
const cookieArgs = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--cookie' && i + 1 < args.length) cookieArgs.push(args[i + 1]);
}
// URLs positionnelles : tout argument non-option qui n'est pas
// la valeur d'une option précédente.
const VALUE_OPTS = new Set(
  ['--list', '--out', '--viewport', '--timeout', '--settle', '--max', '--exe', '--cookie']);
const urls = args.filter(
  (a, i) => !a.startsWith('--') && !VALUE_OPTS.has(args[i - 1]));

function siteName(u) {
  // Règle identique à site_harvest.ps1::Get-SiteName — le `/`
  // terminal est absorbé (`example.com/` = `example.com`).
  return u.replace(/^https?:\/\//, '').replace(/\/+$/, '')
    .replace(/[^A-Za-z0-9_.-]/g, '_');
}

function findBrowser() {
  if (exeOpt) return exeOpt;
  if (process.env.CHROME_PATH) return process.env.CHROME_PATH;
  const cands = [
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
    'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
    '/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  ];
  for (const c of cands) if (existsSync(c)) return c;
  throw new Error('Chrome/Chromium introuvable — --exe <chemin>');
}

function freePort() {
  return new Promise((res, rej) => {
    const s = createServer();
    s.listen(0, '127.0.0.1', () => {
      const p = s.address().port;
      s.close(() => res(p));
    });
    s.on('error', rej);
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// --- client CDP minimal : promesses par id, events par clé ---
class Cdp {
  constructor(wsUrl) { this.wsUrl = wsUrl; this.id = 0; this.pend = new Map(); this.evt = []; }
  async connect() {
    this.ws = new WebSocket(this.wsUrl);
    await new Promise((res, rej) => {
      this.ws.onopen = res;
      this.ws.onerror = (e) => rej(new Error('ws devtools : ' + (e.message || 'échec')));
    });
    this.ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id && this.pend.has(msg.id)) {
        const { res, rej } = this.pend.get(msg.id);
        this.pend.delete(msg.id);
        msg.error ? rej(new Error(msg.error.message)) : res(msg.result);
      } else if (msg.method) {
        this.evt.push(msg);
      }
    };
  }
  send(method, params = {}, sessionId) {
    const id = ++this.id;
    const payload = { id, method, params };
    if (sessionId) payload.sessionId = sessionId;
    this.ws.send(JSON.stringify(payload));
    return new Promise((res, rej) => this.pend.set(id, { res, rej }));
  }
  // Attend un event ; polling borné — un event loupé = timeout
  // honnête, jamais d'attente infinie.
  async waitEvent(method, sessionId, deadlineMs) {
    const end = Date.now() + deadlineMs;
    while (Date.now() < end) {
      const i = this.evt.findIndex(
        (e) => e.method === method && (!sessionId || e.sessionId === sessionId));
      if (i >= 0) return this.evt.splice(i, 1)[0];
      await sleep(50);
    }
    return null;
  }
}

async function main() {
  let sites = [];
  if (listFile) {
    sites = readFileSync(listFile, 'utf8')
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith('#'));
  }
  sites.push(...urls.filter((u) => u.startsWith('http')));
  sites = [...new Set(sites)];
  if (maxSites > 0) sites = sites.slice(0, maxSites);
  if (sites.length === 0) {
    console.error('aucun site — --list <fichier> ou URLs en args');
    process.exit(1);
  }
  mkdirSync(outDir, { recursive: true });

  const exe = findBrowser();
  const port = await freePort();
  const profile = join(tmpdir(), `ref-capture-${port}`);
  const chrome = spawn(exe, [
    '--headless=new',
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    '--no-first-run', '--disable-gpu', '--hide-scrollbars',
    '--force-device-scale-factor=1',
    'about:blank',
  ], { stdio: ['ignore', 'ignore', 'pipe'] });
  chrome.stderr.on('data', () => {});
  console.log(`oracle : ${exe} (port ${port})`);

  try {
    // Endpoint devtools — poll /json/version jusqu'à l'écoute.
    let wsUrl = null;
    for (let i = 0; i < 100 && !wsUrl; i++) {
      try {
        const v = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
        wsUrl = v.webSocketDebuggerUrl;
      } catch { await sleep(100); }
    }
    if (!wsUrl) throw new Error('devtools injoignable');
    const cdp = new Cdp(wsUrl);
    await cdp.connect();
    const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank' });
    const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
    await cdp.send('Page.enable', {}, sessionId);
    await cdp.send('Emulation.setDeviceMetricsOverride',
      { width: vw, height: vh, deviceScaleFactor: 1, mobile: false }, sessionId);

    // Cookies pré-navigation (`--cookie nom=valeur@domaine`).
    for (const spec of cookieArgs) {
      const [nv, domain] = spec.split('@');
      const eq = nv.indexOf('=');
      if (!domain || eq < 0) { console.log(`  SKIP cookie malformé : ${spec}`); continue; }
      await cdp.send('Network.setCookie',
        { name: nv.slice(0, eq), value: nv.slice(eq + 1), domain }, sessionId);
    }

    let ok = 0, fail = 0;
    for (const site of sites) {
      const name = siteName(site);
      try {
        // Purge : un loadEventFired périmé (about:blank, nav
        // précédente) ne doit pas valider l'attente courante.
        cdp.evt.length = 0;
        await cdp.send('Page.navigate', { url: site }, sessionId);
        await cdp.waitEvent('Page.loadEventFired', sessionId, timeoutMs);
        await sleep(settleMs);
        const shot = await cdp.send('Page.captureScreenshot', { format: 'png' }, sessionId);
        writeFileSync(join(outDir, `${name}.ref.png`), Buffer.from(shot.data, 'base64'));

        // Géométrie de référence : bornée (les pages géantes
        // plafonnent à 4000 boîtes — le diff géométrique reste
        // utilisable sur le haut du document, là où la casse
        // commence). JSON.stringify côté page = une valeur de
        // retour simple, pas de graphe sérialisé CDP.
        const expr = `(() => {
          const el = document.scrollingElement || document.documentElement;
          const boxes = [];
          for (const n of document.querySelectorAll('body *')) {
            if (boxes.length >= 4000) break;
            const r = n.getBoundingClientRect();
            if (!r.width && !r.height) continue;
            boxes.push({tag: n.tagName.toLowerCase(),
              id: n.id || '', cls: (n.className && n.className.baseVal === undefined
                ? String(n.className).split(/\\s+/)[0] : '') || '',
              x: Math.round(r.x * 10) / 10, y: Math.round(r.y * 10) / 10,
              w: Math.round(r.width * 10) / 10, h: Math.round(r.height * 10) / 10});
          }
          return JSON.stringify({doc_w: el.scrollWidth, doc_h: el.scrollHeight,
            url: location.href, title: document.title, boxes});
        })()`;
        const ev = await cdp.send('Runtime.evaluate',
          { expression: expr, returnByValue: true, awaitPromise: false }, sessionId);
        let meta = { url: site, final_url: null, viewport: `${vw}x${vh}`, boxes: [] };
        if (ev && ev.result && ev.result.type === 'string') {
          meta = { url: site, viewport: `${vw}x${vh}`, ...JSON.parse(ev.result.value) };
        }
        writeFileSync(join(outDir, `${name}.ref.json`), JSON.stringify(meta, null, 1));
        console.log(`  OK   ${site}  ->  ${name}.ref.png (+${meta.boxes?.length ?? 0} boîtes)`);
        ok++;
      } catch (e) {
        console.log(`  FAIL ${site}  (${e.message})`);
        fail++;
      }
    }
    console.log(`oracle : ${ok} OK / ${fail} FAIL — ${sites.length} sites`);
    process.exitCode = fail > 0 && ok === 0 ? 1 : 0;
  } finally {
    chrome.kill();
    try { rmSync(profile, { recursive: true, force: true }); } catch {}
  }
}

main().catch((e) => { console.error(e.message); process.exit(1); });
