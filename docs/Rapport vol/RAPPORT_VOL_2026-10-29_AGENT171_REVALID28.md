<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 171 : REVALID28 `[V10-main]` + fix `Reflect.construct` (WIP)

- **Binaire** : `C:/vol100-wt/target/release/explorer-daemon.exe` — HEAD `de24aba1` **+ WIP non commité** (`ctors.rs`/`interp/mod.rs` : `ctor_fresh` dans `constructed_this` → couvre `Reflect.construct(String/Number/Boolean, args, newTarget)` pour le feature-detect Babel `_callSuper`/`_isNativeReflectConstruct`).
- **Mission** : REVALID28 — materiel.net, rueducommerce (bras « ctor sans new »), gamekult, millenium.
- **Daemon** : PID **2608**, port 9704, watchdog armé (pic 1 753 Mo) ; arrêt propre, DELETE propres ×4.
- **Verdict** : **`Cannot call a class as a function` et `ctor without new` DISPARUS ×2 sites** — materiel 3→1 entrée (`isConnected`), rdc 5 entrées mais le ctor est passé au maillon suivant ; millenium ~60 % (`category` getjan) ; gk heap ×1 capé.

## Synthèse

| Cible | Verdict | Console | Preuve fix |
|---|---|---|---|
| materiel.net | **~70 %** | `isConnected` (était `Cannot call a class`) | **ctor DISPARU** |
| rueducommerce.fr | **~60 %** | `in requires an object`, `firstChild`, `Invalid scheme` | **ctor DISPARU** |
| gamekult.com | **~65 %** | heap ×1 capé | §B-12 |
| millenium.org | **~60 %** | `category` getjan + gsi | §B-14 +1 |

## Re-vols reportés au vol 172
- Fix `Reflect.construct` validé terrain ×2 — attendre le commit JS-C205 pour l'entrée changelog officielle.
- §B-14 file ; §B-62 ; §B-60 ×5 ; §B-59 ×5 ; `Invalid URL` ×5 ; §B-56/§B-57 résiduels ; §B-54/§B-23 ; §B-1-variante.
