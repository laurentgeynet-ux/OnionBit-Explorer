<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 106 : MEMWATCH `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD
  `400eaf7d`, JS-C182 inclus — aucun nouveau fix depuis).
- **Mission** : MEMWATCH — validation du harnais
  `scripts/vol_memwatch.ps1` en vol réel + cibles prévues :
  github, franceinfo, deezer (suite vol 106 avorté par le
  crash station).
- **Daemons** : PID 2176 port 9639 + **watchdog actif**
  (seuil 5 120 Mo, sondage 15 s, log
  `logs/daemons/_vol106_watch.log`).
- **Verdict global** : **watchdog validé terrain** —
  github.com : explosion RAM **430 → 8 079 Mo en ~30 s**
  pendant la nav (journal watch horodaté, kill automatique
  par PID au seuil) — la station n'a **pas** crashé (vs
  hier : 7 Go + swap + mort station). Le constat se transforme
  en anomalie : **fuite mémoire massive à la nav github**
  (proposition §B-59). Pinterest : §B-4(a) persistant (10
  évals, console vide, coquille).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| github.com | **watchdog tiré** — RAM 356→430→**8 079 Mo en ~30 s** pendant `navigate`, kill auto PID au seuil 5 120 Mo ; log coupé en `commit.*` pompe | — | **→ §B-59 proposé : fuite mémoire nav github** — explosion sous pression pompe, signature §B-26-like mais pendant la nav (avant tout apply journalisé ?) |
| pinterest.fr | **§B-4(a) persistant** — doc_h=900, frame coquille, 10 évals seulement, `pas de journal` | 0 entrée | drains inertes toujours ouverts (~70 scripts site historique) |
| franceinfo / deezer | non volés — daemon tué par le watchdog sur github | | reportés |

## Preuve marquante — watchdog

```
00:42:50 pid=2176  356 Mo
00:43:06 pid=2176  430 Mo
00:43:21 WATCHDOG: pid 2176 8079 Mo > 5120 — kill
(journal daemon coupé net : derniers `commit.*` us=7-200)
```

~**250 Mo/s** de croissance en pleine nav — signature de
fuite non bornée (famille §B-26/§B-30 mais en amont : la nav
elle-même). Daemon tué proprement par PID, sessions voisines
intactes, station stable — le harnais tient son rôle :
protéger la machine sans masquer le constat.

## Arrêt

Watchdog a tué le daemon wedged (PID 2176) ; rien d'autre à
arrêter. Journaux archivés
`logs/daemons/_vol106_watch.log`, `_vol106_daemon.log`.

## Re-vols reportés au vol 107

- **github** → §B-59 fuite nav (reproduire avec `/mem` en
  direct + bissection verbose si daemon retente).
- franceinfo, deezer → exploration reportée ×2.
- flixbus → §B-54 vie-onglet ; vimeo → §B-1-variante.
- cryptpad/bookshop/caniuse → §B-23/§B-12/§B-57-chemin-3.
- uMap/mastodon/tumblr/wttj → §B-28/22/19/36.
