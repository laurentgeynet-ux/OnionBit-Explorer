# Rapport de vol — AGENT 6 (international / charsets exotiques)

Date : 2026-10-26 — daemon `explorer-daemon` release (HEAD 7a01534), port 9390,
viewport 1366×900. Périmètre : sites CJK/cyrillique — erreurs console, rendu,
charset, interactions. Captures dans `captures/vol6_*.png`, dumps dans
`logs/vol6_*_{dom,layout}.txt` + `*_console.json`.

Résumé transversal : **aucun problème de charset mesuré** — tout le texte
CJK/cyrillique extrait du DOM est en UTF-8 propre (0 occurrence de `�` sur
les 4 sites), les titres d'onglets sont corrects. Les dégâts sont côté **JS**
(3 sites sur 4 ont leur JS mort en cascade) et **layout/contenu** (pages
majoritairement blanches ou squelettiques).

---

## 1. https://www.baidu.com — tab 1

- URL finale : `https://www.baidu.com/` — titre « 百度一下，你就知道 ».
- `doc_h = 900` (= viewport : aucune hauteur supplémentaire générée).
- Console : **14 erreurs**, toutes JS. Top-5 verbatim :

  1. `exception js : TypeError: \`toLowerCase\` is not a function (on Object)` — jquery-1-edb203c114.10.2.js:135:103 (jQuery 1.10.2 plante au chargement).
  2. `exception js : ReferenceError: $ is not defined` — src `(function(){var amd={keys:{},addPaths:…`.
  3. `exception js : ReferenceError: $ is not defined` — src `bds.util.domain=function(){var list=$.extend(…`.
  4. `exception js : TypeError: array method called on non-array` — polyfill_9354efa.js:1:2.
  5. `exception js : ReferenceError: F is not defined` — src `F._setMod("superman");F._fileMap(…` (loader AMD maison).

- Visuel (`vol6_baidu.png`, 1366×900, 1145 couleurs) : fond quasi intégralement
  blanc (92 % de pixels #FFF), logo présent (pixels rouges #E10602 / bleus
  #4E6EF2 typiques du logo Baidu), quelques liens gris foncé — la page
  s'affiche mais appauvrie : barre de nav supérieure et champ de recherche
  textuel ; tout le contenu dynamique (suggestions, flux) absent car jQuery
  n'a jamais démarré.
- Charset : parfait — « 百度 », « 搜索 », « 更多 » extraits correctement,
  zéro mojibake.
- Interaction : `scroll` ×2 → `scroll_y` reste **0.0** (document sans
  débordement, cohérent avec doc_h=900) ; clic → `ignored`.
- Classification : **JS** (cause racine : `toLowerCase` appelé sur Object dans
  jQuery → cascade de `ReferenceError: $`). Sévérité : **majeure** — la page
  est un squelette statique.

## 2. https://www.nicovideo.jp — tab 2

- URL finale : `https://www.nicovideo.jp/` — titre « ニコニコ ».
- `doc_h = 5919` — frame capturée en document entier (1366×5919, 24 Mo).
- Console : **1 erreur**, mais fatale :

  `syntaxe js 2:1262168 : expected \`;\`` — `resource.video.nimg.jp/web/scripts/bundle/vendor.js` (bundle webpack complet rejeté au parse → tout le JS du site mort).

- Visuel (`vol6_nicovideo.png`, 278 couleurs) : page essentiellement en
  niveaux de gris (#FAFAFA/#F5F5F5/#252525) — la structure des cartes de
  vidéos est dessinée mais **aucune miniature d'image** ne semble rendue
  (gris uniforme), aucune couleur de marque visible.
- Layout : texte japonais correct dans l'arbre — « ニコニコ », « 動画 »,
  「さたぱんＰさんからのメッセージ…」, compteurs numériques ; le HTML statique
  est posé correctement, seul l'enrichissement JS/images manque.
- Charset : parfait, 0 `�`, kana/kanji intacts.
- Interaction : `scroll` ×2 → `scroll_y` 800→1600 OK ; clic → `ignored`.
- Classification : **JS** (parse error sur syntaxe moderne dans vendor.js)
  + suspect **render/net** (miniatures absentes). Sévérité : **majeure**.

## 3. https://www.rakuten.co.jp — tab 3 (+ onglet parasite 4)

- URL finale : `https://www.rakuten.co.jp/` — titre
  « 【楽天市場】Shopping is Entertainment! … » correct (brackets japonais
  【】 inclus).
- `doc_h = 1360`.
- Console : **3 erreurs** :

  1. `exception js : ReferenceError: jQuery is not defined`
  2. `exception js : ReferenceError: jQuery is not defined`
  3. `exception js : ReferenceError: IDBIndex is not defined` — vendors-038cc7061200ffaabfb2.bundle.js:2:927354 (IndexedDB non implémenté).

- **Bug produit mesuré** : un **onglet supplémentaire id=4** est apparu tout
  seul, navigué sur `https://grp15.ias.rakuten.co.jp/gw.js?v=2` — une URL de
  **script** (fichier .js publicitaire/tracking) a été ouverte comme document
  top-level, vraisemblablement via `window.open`/lien non intercepté.
  Un navigateur ne doit jamais afficher un .js brut comme onglet.
- Visuel (`vol6_rakuten.png`, 1366×1347, 445 couleurs) : page massivement
  blanche/beige clair (#FFF, #F7F7F7…) — structure présente mais quasiment
  aucune image bannière rendue ; layout très peu profond (layout.txt 15 Ko).
- Interaction : `scroll` ×2 et clic → tous **`ignored`** (trois refus
  consécutifs — l'input semble bloqué sur cette page).
- Charset : parfait — 「楽天」「市場」「ポイント」 intacts, 0 `�`.
- Classification : **JS** (jQuery + IndexedDB manquant), **DOM/navigation**
  (onglet parasite sur .js), **input** (gestes refusés). Sévérité : **majeure**.

## 4. https://yandex.com — tab 5

- URL finale : `https://yandex.com/` — titre « Yandex — fast Internet search ».
- `doc_h = 900`.
- Console : **« pas de journal » — 0 erreur**. Seul site propre du vol.
- Visuel (`vol6_yandex.png`, 1366×900, 391 couleurs) : page minimaliste mais
  correcte — fond blanc, bandeau sombre #252A38, jaune marque #FFDD57 et
  corail #F8604A présents ; page de recherche reconnaissable.
- Layout : arbre compact et sain (7 Ko) — `main`, header avec logo SVG
  133×33, zone de recherche ; SVG posé.
- Charset : page côté .com en anglais ; DOM contient du cyrillique dans les
  attributs/liens (57 « Search », aucun `�`).
- Interaction : scroll → `scroll_y` 0 (pas de débordement) ; clic →
  `focused` (champ de recherche focusé — seul clic fructueux du vol).
- Classification : **rien de bloquant** ; fonctionnalités JS non exercées.
  Sévérité : **faible**.

---

## Constats actionnables

1. **[JS — bloquant] `toLowerCase is not a function (on Object)`** dans
   jQuery 1.10.2 (baidu) : probablement `toLowerCase` résolu via la chaîne de
   prototypes sur un objet non-string. Un seul crash tue jQuery → cascade
   `ReferenceError: $ is not defined` (9 des 14 erreurs baidu en découlent,
   2 sur rakuten). Priorité maximale : un fix élargit d'un coup la
   compatibilité de tout l'écosystème jQuery-legacy.
2. **[JS — bloquant] Parse error `expected ;` dans un bundle webpack**
   (nicovideo vendor.js, offset ~1 262 168) : une syntaxe ES récente n'est
   pas reconnue → le bundle entier est rejeté et le site devient inerte.
   Reproduire en extrayant vendor.js et en localisant le token à l'offset.
3. **[JS] `IDBIndex is not defined`** (rakuten) : IndexedDB absent du moteur
   — stub no-op suffirait pour débloquer les feature-detections.
4. **[Navigation — produit] Un onglet top-level s'ouvre sur une URL de
   script `.js`** (`grp15.ias.rakuten.co.jp/gw.js`) : `window.open` d'une
   ressource non-document, ou politique de popup non filtrée. À tracer côté
   `explorer-core` (gestion des navigations initiées par JS).
5. **[Input] Gestes `ignored` en série sur rakuten** : scroll/click refusés
   (probablement pendant un apply job DOM ou un handler JS absent). Vérifier
   la file d'input quand le thread JS est occupé/mort.
6. **[Render] Miniatures/images distantes absentes** sur nicovideo et
   rakuten (aplats gris/blanc uniformes) : vérifier le pipeline de
   fetch/décodage d'images (lazy-loading JS-driven probable, mais à
   confirmer par un shot sur image directe).
7. **[Charset — RAS]** UTF-8 CJK/cyrillique parfait sur 4/4 sites : titres,
   textes, 【】、kana — le pipeline charset/document n'est **pas** le goulot
   actuel pour l'i18n. Non mesuré : sites en encodage legacy (Shift_JIS,
   Big5, windows-1251) — candidat pour un prochain vol.
8. **[Layout — informationnel] `doc_h = viewport` sur baidu/yandex** :
   cohérent avec des pages « une vue » ; mais sur baidu le contenu réel est
   plus long — l'absence de débordement est un symptôme de la mort JS (§1),
   pas un bug layout en soi.
