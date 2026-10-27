# Rapport de vol — Agent 9 — Google (vol9)

Date : 2026-10-26 — Daemon : `explorer-daemon --port 9393`, viewport 1366×900,
binaire `C:/obe-vol-head/target/release/explorer-daemon.exe`.

## Synthèse

| Cible | URL finale | Rendu | Erreurs console | Sévérité |
|---|---|---|---|---|
| google.com (home) | `https://www.google.com/` | Page d'accueil complète (logo, champ, boutons, footer) | « pas de journal » | — |
| google.fr | redirigé → `https://www.google.com/` | Identique à la home | « pas de journal » | — |
| Consentement | `consent.google.com/ml?...` | Page « Avant d'accéder à la recherche Google » **entièrement rendue** (boutons « Tout refuser » / « Tout accepter » hit-testables et cliquables) | « pas de journal » | — |
| SERP `q=a` puis `q=onion+architecture` | `google.com/search?...` | **Coquille JS** : page de repli « Si vous avez des difficultés à accéder à … cliquer ici » (doc_h 26 px) ; résultats **jamais construits** | « pas de journal » (aucune erreur JS loguée) | **Haute** |
| maps.google.com | redirigé → `support.google.com/maps/answer/6186115` (« Mettre à jour votre navigateur ») | Page d'aide Google rendue (doc ~1140 px) | 19 exceptions JS | **Haute** |

## 1. google.com — page d'accueil

- DOM 91 023 o, layout complet : header (Gmail/Images), formulaire,
  champ `input` (512×32 à [375,235]), boutons « Recherche Google » /
  « J'ai de la chance », footer. doc_h = 452,5 px.
- **Pas de mur de consentement sur la home** — il n'apparaît qu'à la
  soumission du formulaire (voir §3).
- Interactions OK : `mousemove` sur le champ → `{"editable":true}`,
  `click` → `{"result":"focused"}`, `key "a"` → `{"result":"edited"}`.
- Console : `pas de journal` — aucune erreur JS remontée alors que la
  page embarque des scripts (silencieux ou stubbés).

## 2. google.fr

- Redirigé côté serveur vers `https://www.google.com/` — comportement
  identique à §1. Capture `vol9_google_fr.png`.

## 3. Mur de consentement — PRÉSENT et fonctionnel

- Le clic sur « Recherche Google » (soumission GET du `<form>`) navigue
  vers `https://consent.google.com/ml?continue=...&gl=FR&hl=fr`.
- La page de consentement est du HTML/CSS classique, **rendue
  correctement sans JS** : titre « Avant d'accéder à la recherche
  Google », paragraphes, listes, deux `<input type=submit>` dans deux
  `<form>` inline (« Tout refuser » à [434,924 240×40], « Tout
  accepter » à [682,924 250×40]), lien « Plus d'options ».
- Clic sur « Tout accepter » (806,944) → `{"result":"navigated"}` →
  retour sur l'URL `continue=` (`google.com/search?q=a&...&gbv=2`).
- Capture `vol9_consent.png` (2,76 Mo) — page pleine, bien posée.

## 4. SERP — cible principale : ÉCHEC, résultats script-morts

- URL finale après consentement : `google.com/search?q=a&sa=G&...&gbv=2`.
  Navigation directe sur `?q=onion+architecture` : même résultat.
- Layout : **un seul div de 18 px** — « Si vous avez des difficultés à
  accéder à cliquer ici ou envoyer des commentaires. » doc_h ≈ 26 px.
- Bissection (dom.txt, 93 ko) : le body ne contient que `<noscript>`
  (repli + meta-refresh vers `/httpservice/retry/enablejs?...`) et des
  dizaines de `<script>` inline (bootstrap closure/trustedTypes).
  **Aucun `<h3>`, aucun lien de résultat, aucun texte de résultat** —
  les résultats n'existent que post-script ; ils ne sont pas masqués,
  ils ne sont jamais construits. Classification : **JS** (DOM non
  construit faute d'exécution des scripts Google).
- Clic sur « cliquer ici » (536,17) → `navigated` → nouvelle URL avec
  `emsg=SG_REL` → **même page de repli** (boucle : le lien « enablejs »
  sert à nouveau la coquille). `gbv=2` (version « sans JS ») ne produit
  pas de HTML de résultats pour un client moderne.
- Console : `pas de journal` — **aucune erreur JS n'est loguée** pour
  ces pages alors que les scripts tournent manifestement en erreur
  (cf. §5 où les exceptions remontent). Suspect : erreurs avalées par
  les `try/catch` de Google ou ring console non alimenté sur ce chemin.
- Captures `vol9_serp_a.png`, `vol9_serp_real.png`, `vol9_serp_onion.png`
  (1,86 Mo chacune — viewport blanc + ligne de texte).

## 5. maps.google.com — détection « navigateur non supporté »

- URL finale : `https://support.google.com/maps/answer/6186115?hl=fr&
  p=unsupported&rd=1` — « Mettre à jour votre navigateur pour utiliser
  Google Maps ». Redirection serveur/JS vers la page d'aide.
- La page d'aide est rendue (DOM 1,6 Mo, layout ~1140 px, logos, liens).
- **19 exceptions JS verbatim** (extrait représentatif, console
  complète dans `logs/vol9_maps_*`) :

```
exception js : TypeError: `createContextualFragment` is not a function (on Range)
    at pm (1024:2)
exception js : TypeError: `sc_initPage` is not a function (on Object)
    at <anonymous> (1:2)
exception js : TypeError: `sc_initQueryParameterPreserver` is not a function (on Object)
exception js : TypeError: `sc_initStats` is not a function (on Object)
exception js : TypeError: cannot read `ehn` of Undefined
    at <anonymous> (4:16) at khb (2:1) at u3 (1:1)
exception js : TypeError: cannot read `enable_footprints_set_time_on_page_logging` of Undefined
    at t4a (7:1) at PX (6:34)
exception js : TypeError: `build` is not a function (on MA)
    at $Pa (13:1) at pL (11:2) at S_ (2:1)
exception js : TypeError: `sc_trackStatsEvent` / `sc_trackStatsPageView` /
    `sc_trackStatsLatency` is not a function (on Object)
exception js : TypeError: cannot read `dark` of Undefined  at ln (1104:1)
exception js : TypeError: cannot read `fbid` of Undefined  at VNa (324:1) / TLa (1:1)
exception js : TypeError: cannot set `834` of Undefined
exception js : TypeError: cannot set `1024` of Undefined
exception js : TypeError: cannot read `au` of Undefined
    at YK (8:30) at vub (48:1) at Kvb (210:1)
exception js : TypeError: `sc_initSelects` / `sc_initLanguageSelector` /
    `sc_initFeedback` / `startSurveyWithDelay` is not a function
```

- Classification : **JS** — API manquante `Range.createContextualFragment`
  (amorce de la cascade) puis cascade « `sc_*` is not a function » /
  lectures sur `Undefined` : les bibliothèques `sc`/`hcfe`/`MA`/`vh`
  n'ont pas survécu aux exceptions initiales. Sévérité haute mais la
  page de repli est lisible.

## Constats actionnables

1. **[JS, haute] Implémenter `Range.createContextualFragment`** —
   première exception fatale sur l'écosystème Google (Maps/support) ;
   bloque l'initialisation des libs `sc_*` (cascade de 18 erreurs).
2. **[JS, haute] SERP Google = DOM 100 % post-script** : sans exécution
   correcte du bootstrap, zéro résultat n'est construit — la cible de
   compat minimale est « faire survivre le bootstrap sans exception »
   (cf. constat 1 + APIs de timing `google.tick`, `trustedTypes`).
3. **[JS/console, moyenne] Trou noir d'erreurs** : sur google.com et la
   SERP, aucune erreur n'atteint le ring console (`pas de journal`)
   alors que les scripts ne produisent rien — vérifier si les
   exceptions sont interceptées par les try/catch Google ou si le canal
   d'erreur n'est pas alimenté sur ces pages.
4. **[DOM/HTML, basse] Meta-refresh noscript non suivi** : le repli
   noscript contient `<meta http-equiv="refresh" url=/httpservice/
   retry/enablejs>` — le clic manuel suit bien le lien, mais la
   redirection automatique n'est pas tentée (comportement à confirmer
   vs spec : le meta-refresh dans noscript devrait être honoré quand
   le scripting échoue partiellement ?).
5. **[Positif] Mur de consentement** : présent, rendu et **franchissable**
   (clic « Tout accepter » → navigation de retour fonctionnelle,
   cookie posé, pas de boucle). Hit-test `editable:true` et saisie clavier
   OK sur la home.
6. **[Net, info] Redirections serveur correctement suivies** : google.fr
   → .com, maps → support.google.com (page d'aide « unsupported »,
   détection UA/navigateur).

## Artifacts

- Captures : `captures/vol9_google_com.png`, `vol9_google_fr.png`,
  `vol9_consent.png`, `vol9_serp_a.png`, `vol9_serp_real.png`,
  `vol9_serp_onion.png`, `vol9_maps.png`.
- Dumps : `logs/vol9_*_{dom,layout}.txt`.
