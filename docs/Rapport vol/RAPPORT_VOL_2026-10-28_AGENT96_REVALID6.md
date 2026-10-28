<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 96 : REVALID6 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol96-wt/target/release/explorer-daemon.exe` (build
  `Finished` 21:52, HEAD `6b19dcf1` — post-Étape 151, R-128,
  JS-C173/174). Worktree éphémère `C:/vol96-wt` à HEAD —
  l'arbre principal porte du WIP (R-129 §B-27, R-130 §B-55,
  JS-C175 §B-37 **non committés** → leurs re-vols reportés
  au vol 97).
- **Mission** : REVALID6 — valider l'Étape 151 (§B-54) et les
  re-vols reportés du vol 95 : credit-agricole + flixbus
  (§B-54), marmiton (§B-20/É149), manomano (§B-30),
  frandroid (§B-46), bookshop (§B-12).
- **Daemon** : 2 process — PID **22020** port 9613 (wedged
  post-DELETE flixbus, arrêt forcé) puis PID **14548** port
  9614 (credit-agricole, arrêt propre). `EXPLORER_VIEWPORT=
  1366x900`, journaux `logs/daemons/_vol96_daemon{,2}.log`.
  Vol exécuté en boucle autonome (skill `/vol`, première
  campagne).
- **Verdict global** : **§B-54 RÉSOLU sur credit-agricole**
  (navigate retourné en 67,9 s vs jamais retourné vol 95 ;
  `poll_net` max **3,49 s** vs 110,8 s ; DELETE propre 0,49 s,
  API vivante ; mécanisme journalisé « livraisons différées
  (B-54) deferred=59 »). **§B-54 PARTIEL sur flixbus** :
  navigate retourné (95 s, page peinte) mais **wedge
  post-DELETE** — la pompe de l'onglet a continué et affamé
  l'API (>30 s, livelock, arrêt forcé) — recoupe §B-30
  (pompe orpheline). **§B-12 étendu à un 3ᵉ site** :
  frandroid ×6+ occurrences identiques (538 Mo) — la dédup
  `83916a6` reste inopérante terrain. **§B-46 non reproduit**
  (frandroid : navigate 32 s, DELETE effectif). **§B-20
  favorable confirmé** (marmiton peint au scroll). **Deux
  trous nouveaux** : §B-56 moteur regex (regex valides
  rejetées) et §B-57 `expression too deeply nested` sur
  polyfill cdnjs réel (manomano → mur incompatible).

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| marmiton.org | **§B-20 favorable** — photos peintes au scroll (bannière, packshots, cookies) ; doc_h=11 098 | 1 entrée : `TypeError: toLowerCase` nommée (concat.js) | js_heap **343 Mo** (vs 51,4 Mo vol 95 — hausse à investiguer) ; titres de cartes peints en double/clippés ; DELETE propre |
| bookshop.org → uk.bookshop.org | **§B-12 PERSISTANT** — `heap memory limit exceeded` **×7 identiques** (540,9 Mo) ; page peinte | `TypeError: left` nommée ; `import() → /null` 404 ×2 (J3) ; `unhandledrejection [object Object]` | dedup `83916a6` toujours inopérante ; js_heap 541 Mo ; DELETE propre |
| manomano.fr | **mur navigateur-incompatible** — redirect `/navigateur-incompatible`, doc_h=0 | `RangeError: expression too deeply nested` sur `polyfill.min.js` cdnjs | **→ §B-57** : la détection de compat meurt dans le polyfill ; DELETE propre |
| frandroid.com | **§B-46 NON REPRODUIT** — navigate 32,3 s, doc_h=17 348 (clampé), DELETE effectif | `SyntaxError: invalid regex /^[\w:-]{32,512}$/` (**→ §B-56**) ; `Element missing: script#wp-emoji-settings` ; heap limit ×6+ | **§B-12 étendu** (3ᵉ site : 538 Mo, ×6+) ; DELETE 1ᵉʳ appel en timeout client mais effectif (404 au 2ᵉ) |
| flixbus.fr | **§B-54 PARTIEL** — navigate **retourné** 95 s (vs wedge vol 93), page peinte (hero, menus) ; mais `poll_net` max **17,8 s** puis **wedge post-DELETE** : API morte >30 s, livelock, RAM 1,6 Go | — | pompe orpheline post-DELETE affame l'API — variante §B-54/§B-30 ; arrêt forcé PID 22020 |
| credit-agricole.fr | **§B-54 RÉSOLU** — navigate **67,9 s** (vs >150 s vol 95), `poll_net` max **3,49 s** (vs 110,8 s), DELETE 0,49 s, API vivante | 404 images `%5Ccontent%5C` (backslashes non normalisés) ; `TypeError: if` nommée | `livraisons différées (B-54) deferred=59` ×13 au journal — borne É151 tirée terrain |

## Preuves marquantes

### §B-54 — credit-agricole : borne É151 tirée, wedge évité

```
navigate : 67,88 s → retourné, doc_h=3 785,5
event.poll_net max : 3 492 752 µs (3,49 s)   [vol 95 : 110 783 422 µs]
WARN poll_net : livraisons différées à la prochaine passe (B-54)
      deferred=59 budget_ms=…   ×13 occurrences
DELETE tab=1 : 204 en 0,49 s — API vivante après coup
```

vs vol 95 : navigate jamais retourné, API entière morte,
arrêt forcé. **RÉSOLU `[V10-main]`**.

### §B-54 — flixbus : nav bornée mais wedge post-DELETE

```
navigate : 94,99 s → retourné, doc_h=3 632 (vs wedge vol 93)
event.poll_net max : 17 787 611 µs (17,8 s) — borne 2 s
                     dépassée en mesure (attente net réelle ?)
DELETE tab=5 : timeout client 30 s
puis /api/tabs et POST /api/tabs : 000 >10 s — API morte,
log actif (livelock), RAM 1 601 Mo → arrêt forcé PID 22020
```

Le navigate ne wedge plus (progrès net), mais la pompe de
l'onglet survit au DELETE et affame l'API — bras résiduel
§B-54/§B-30 (pompe orpheline post-DELETE). **OUVERT —
variante post-DELETE**.

### §B-12 — dédup inopérante, 3ᵉ site

```
bookshop : RangeError heap memory limit exceeded
  (1 078 900 cells, 540 869 388 B) ×7 identiques
frandroid : idem (1 075 004 cells, 538 077 450 B) ×6+
```

Trois sites désormais (lemonde hist., bookshop, frandroid) —
latch `heap_dead` + dedup `emit_pipeline_error` (`83916a6`,
J4 « livré ») **inefficaces terrain**. **OUVERT**.

### §B-56 — moteur regex : patterns valides rejetés

```
frandroid : SyntaxError: invalid regex /^[\w:-]{32,512}$/
            (Error compiling regex — quantificateur {n,m}
             sur classe ?)
airfrance vol 95 : \p{Pat_Syn}\p{Pat_WS} + notation octale
            \141\164 — rejetées, SyntaxError nommées
```

Gap récurrent du moteur regex sur des patterns du web réel —
**proposition §B-56** (`explorer-jscore`).

### §B-57 — `expression too deeply nested` sur polyfill réel

```
manomano → redirect /navigateur-incompatible, doc_h=0
exception js : RangeError: expression too deeply nested
  at ToObject (cdnjs polyfill.min.js?version=4.8.0&flags=
  gated&features=…toBlob,default-3.6,es5…es2017…Intl…)
```

Le bundle polyfill.io réel dépasse la borne de profondeur
(famille JS-C106 — le mur « navigateur incompatible » est la
conséquence : la détection elle-même meurt). **proposition
§B-57** (`explorer-jscore`).

### §B-20 — marmiton : images au scroll, js_heap en hausse

Photos peintes au scroll (`captures/vol96_marmiton_scroll.png`)
— repli `data-src` + `data-srcset` (É149) fonctionnels.
**Signal d'attention** : js_heap **343 Mo** vs 51,4 Mo au
vol 95 — hausse possiblement liée aux évals modulepreload
(É148) ; à suivre. Titre de cartes peints en double/clippés
(déborde au-dessus du cadre) — observation rendu mineure.

## Arrêt du daemon

- PID 22020 (port 9613) : wedge §B-54 post-DELETE flixbus →
  `Stop-Process` ciblé. Sessions voisines :9510/:9514 intactes.
- PID 14548 (port 9614) : arrêt propre post-credit-agricole.
- Journaux archivés `logs/daemons/_vol96_daemon{,2}.log`.

## Re-vols reportés au vol 97 (arbre principal committé)

- ouest-france → §B-27 (R-129 mosaïque flex — **WIP au vol 96**)
- ici.fr → §B-55 (R-130 panic resvg — WIP)
- caniuse → §B-37 (JS-C175 NodeId obsolète — WIP)
- flixbus → variante §B-54 post-DELETE (pompe orpheline)
- marmiton → js_heap 343 Mo (régression évals É148 ?)
- §B-56 regex : rejouer `/^[\w:-]{n,m}$/` + `\p{…}` +
  octal sur corpus réel
- §B-57 : polyfill cdnjs `flags=gated` multi-features
