<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 125 : MASSA2 `[V10-main]`

- **Binaire** : `[V10-main]` — `C:/vol100-wt/target/release/explorer-daemon.exe` à `f20477ba` (JS-C190 — `main` toujours cassé : `mse.rs` non commité).
- **Mission** : MASSA2 — leboncoin, caf.fr, franceculture, lindependant.
- **Daemon** : PID **17164**, port 9658, watchdog armé — **tué par le watchdog** (5 996 Mo sur lindependant).
- **Verdict** : leboncoin 403 anti-bot (constat externe) ; caf ~50 % (`secp256k1` unknown curve + `tooltip`/`modal`/`javaEnabled` → §B-14) ; franceculture ~70 % (`no module loader` — bras module-loader partagé avec lequipe) ; **lindependant = 2ᵉ site §B-59** (486→5 996 Mo en ~45 s pendant les pompes, kill auto — la fuite n'est PAS github-spécifique : site presse lourd déclenche identique, racine pompe/miroir à nommer).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| leboncoin.fr | **403** anti-bot | `HTTP 403` | externe (comme fnac/darty) |
| caf.fr | **~50 %** — doc_h=900 | `secp256k1` unknown curve (stormcaster), `tooltip`/`modal` on ce, `javaEnabled`, `addEventListener of Undefined`, JSON token | §B-14 +5 (dont crypto nouvelle) |
| radiofrance.fr/franceculture | **~70 %** — doc_h 22 195 | `document of Null`, `no module loader ../chunks/` | §B-14 +2 (module-loader = bras partagé lequipe) |
| lindependant.fr | **§B-59 ×3 (2ᵉ site)** — nav retournée puis explosion 486→2 869→5 102→**5 996 Mo** en ~45 s → watchdog kill | — | fuite pompe/miroir généralisée (presse lourde) — preuve `_vol125_watch.log` |

## Preuve marquante — §B-59 2ᵉ site

```
07:05:39  486 Mo
07:05:54 2 869 Mo   (+ ~2,4 Go / 15 s)
07:06:09 5 102 Mo
07:06:24 5 996 Mo → kill watchdog
```

github ×2 et lindependant : signature identique en croissance
post-nav. **§B-59 généralisé** — ce n'est pas la cible, c'est
le churn de la pompe.

## Arrêt
Watchdog killé (PID 17164) ; DELETE antérieurs propres ;
sessions voisines intactes.

## Re-vols reportés au vol 126
- **Rebuilder si `mse.rs` commité** → allociné JS-C191.
- §B-59 à re-trier : reproduit sur 2 sites — chercher le
  déclencheur (pubs/trackers volumineux ? nombre de fetches ?).
- §B-14 file : `secp256k1` unknown curve, `tooltip`/`modal` on ce, `no module loader`/`spécificateur irrésolvable` (bras modules — lequipe+franceculture), `javaEnabled`, `document of Null`, + file.
- §B-60 fetch storm ; §B-54/§B-1-variante ; §B-23 cryptpad.
