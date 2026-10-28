<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 103 : REVALID12 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (build
  `Finished`, HEAD `53b3c145` — inclut **JS-C181 §B-57 bras
  2** + R-131/JS-C180/É155).
- **Commits post-binaire** : `5e14c375` (**Étape 157 — §B-54
  drain `animationstart` borné**) + doc `db426a4d` — vise le
  wedge vie-onglet flixbus reproduit ×2 → priorité 1 vol 104.
- **Mission** : REVALID12 — re-vols §B-57 bras 2
  (manomano+caniuse), §B-5 marmiton (`import()`), exploration
  nouvelle : jeuxvideo (gaming FR absent du corpus),
  dailymotion (vidéo FR), flickr (photo international).
- **Daemon** : PID **10952**, port 9634, viewport 1366×900,
  journal `_vol103_daemon.log` — un seul daemon, 0 wedge,
  DELETE propres ×6, RAM 474 Mo, arrêt propre.
- **Verdict global** : **§B-57 bras 2 PARTIEL** — manomano :
  `deeply nested` **disparu** (mur migré : `id of Undefined`
  SDK didomi → §B-14, redirect `/navigateur-incompatible`
  inchangé) ; caniuse : `deeply nested` PERSISTANT
  (`bundle.js`, chemin hors `fn.length`). **§B-5 favorable**
  (marmiton : `module non chargé` non reproduit, heap 54 Mo
  sain). **§B-14 matrice +4** : `hasOwnProperty` sur
  `CSSStyleDeclaration` (jvc), `finally`/`on` sur `Promise`
  (dailymotion), `configurable of Undefined` osano +
  `defineProperty QuickEffectIds` (flickr). JVC mégamenu
  peint ouvert → §B-17.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| manomano.fr | **§B-57 bras 2 RÉSOLU** — `deeply nested` 0 occurrence (vs `at ToObject` vol 100) ; redirect conservé | `cannot read id of Undefined` (sdk.privacy-center.org legacy) | mur déplacé → §B-14/§B-4 (didomi) ; doc_h=0 |
| caniuse.com | **§B-57 bras 2 PERSISTANT** — `deeply nested` ×1 `bundle.js:2:51736` (chemin hors `fn.length`) ; page rendue SSR ~95 % | + `osdlfm` iframe | dernier chemin de récursion restant |
| marmiton.org | **§B-5 favorable** — `module non chargé` non reproduit ; page ~90 % (nav, cartes actus, photos) ; js_heap **54,3 Mo** sain | 6 entrées nommées (adtech `DFPAdserver`, `toLowerCase`, `install`, `init`, `user`, `recipes` — toutes `of Undefined`) | cascade adtech §B-14 ; hero gris (image manquante) |
| jeuxvideo.com | **dégradé ~40 %** — header/nav OK, mégamenu peint **ouvert non stylé** (listes `<li>` déployées) | `hasOwnProperty is not a function (on CSSStyleDeclaration)` hp.js + `fetch network error` | **§B-17** (menus JS-closables) + **§B-14** (`hasOwnProperty` sur style decl) |
| dailymotion.com | **coquille vide** — doc_h=900, frame blanche | `config of Undefined` inline ; `finally`/`on` **not a function (on Promise)** ×3 ; `object is not iterable` runtime | **§B-14** : `Promise.finally` (et `.on` sur thenable) absents — mur lecteur vidéo |
| flickr.com | **~65 % rendu** — nav, recherche, hero « Welcome », crédits photos ; photos absentes (gris) | `WebFont is not defined` ; `configurable of Undefined` (osano cmp) ; `cannot define property QuickEffectIds` | **§B-14** (defineProperty frozen, osano) ; images lazy non collectées ? |

## Preuves marquantes

### §B-57 — le mur a migré (manomano)

```
vol 100 : RangeError: expression too deeply nested at ToObject
          (polyfill.min.js cdnjs)
vol 103 : 0 « deeply nested » — le polyfill parse ;
          mur suivant : TypeError: cannot read `id` of
          Undefined (sdk didomi legacy) → redirect identique
```

→ JS-C181 validé sur le site d'origine ; le site reste
incompatible par un autre défaut (§B-14).

### §B-14 — entrées fraîches

```
`hasOwnProperty` is not a function (on CSSStyleDeclaration) — jvc hp.js
`finally` is not a function (on Promise) — dailymotion player
`on` is not a function (on Promise) — dailymotion app.js
cannot define property `QuickEffectIds` — flickr schunk
`id` of Undefined — didomi legacy (manomano)
`addEventListener` on IDBRequest — mastodon (vol 102)
```

## Arrêt du daemon

DELETE propres ×6 ; `Stop-Process -Id 10952` ; RAM 474 Mo à
`{"tabs":[]}` — purge saine, zéro wedge sur le vol.

## Re-vols reportés au vol 104

- **flixbus** → §B-54 : CORRECTIF Étape 157 `5e14c375`
  (drain `animationstart` borné) — vise le wedge vie-onglet
  reproduit ×2 → **priorité 1**.
- **caniuse** → §B-57 bras 2 : chemin `bundle.js` restant
  (pas de correctif — à remonter).
- **manomano** → bras didomi `id of Undefined` (§B-14).
- cryptpad/bookshop/airfrance → §B-23/§B-12/§B-56-octal
  (reportés, sans correctif).
- uMap/mastodon/tumblr/wttj → §B-28/§B-22/§B-19/§B-36
  (ouverts, preuves fraîches vol 102).
