<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 134 : LIGHT `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité ; JS-C191/192/194/195/197 en attente).
- **Mission** : LIGHT — news.ycombinator, dev.to, larousse, corriere.
- **Daemon** : PID **7992**, port 9667, watchdog armé (pic 1 419 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **3/4 consoles VIDES** — HN, dev.to, larousse rendus propres (signe du net progressif : les sites sobres passent désormais intacts). corriere doc_h 48 593 (record du vol) mais `allocation exceeds engine limits` ×N (nielsen SDK — signature nouvelle : pas heap limit, **allocation brute**) + `useCanonical`/`hasWebVitals` marfeel ; frame timeout.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| news.ycombinator.com | **~95 %** — console vide | 0 entrée | net positif |
| dev.to | **~85 %** — console vide | 0 entrée | net positif |
| larousse.fr | **~85 %** — console vide | 0 entrée | net positif |
| corriere.it | **~70 %** — doc_h 48 593 | **`allocation exceeds engine limits` ×N** (nielsen), `useCanonical`/`hasWebVitals` | §B-14 +3 + signature nouvelle allocation |

## Arrêt
DELETE propres ×4 ; `Stop-Process -Id 7992` ; watchdog nominal.

## Re-vols reportés au vol 135
- **Rebuilder si `mse.rs` commité** → JS-C191/192/194/195/197.
- §B-14 : `allocation exceeds engine limits` (corriere nielsen — à rattacher §B-12 ou nouvelle entrée), `useCanonical`/`hasWebVitals`, + file.
- §B-59 ×2 sites ; §B-60 ; §B-56 résiduel ; §B-54/§B-23.
