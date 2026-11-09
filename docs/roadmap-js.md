# Roadmap — File JS dédiée (spécialiste `explorer-jscore`)

File de travail **uniquement JS** : chaque entrée vit dans
`crates/explorer-jscore` (crate autonome, aucune dépendance
interne, `#![forbid(unsafe_code)]`) — **aucune couture
NAV exigée** : pas d'extension `JsEngine`, pas de commande
session, pas de fetch. Les gaps miroir se livrent côté
`dom_host.rs`/builtins, le langage côté `interp`/`parser`,
le GC côté `heap`/marquage.

**Désignation `JT-N`** — distincte de `JS-CNN` (piste JS de
la roadmap NAV), `V-N` (`roadmap-vitesse-js.md`), `R-N`
(PIPE), `Étape NN` (produit). Comme V-N : l'étape est
**re-numérotée JS-Cxx** dans
`docs/roadmap-equipe-navigateur.md` au moment de la prise —
le CHANGELOG porte le JS-Cxx, la case JT-N est cochée ici
avec le renvoi.

**Règle des déclencheurs** : tout item de cette file est
déjà nommé par un vol/site réel — la règle J-B (« pas de
couverture sans déclencheur ») ne s'applique qu'au palier
JT-J (dormants). Chaque livraison = surface minimale +
verrou de régression `cNNN_*`/`jt*` + jamais de panic sur
contenu hostile.

**Sources de vérité citées** : `§B-nnn` = registre des
murs de la roadmap NAV ; `vol NNN` = rapport de vol
terrain ; « mesuré » = compteur/sonde, pas intuition.

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.

**Ordre de la file — coût croissant, groupé par zone de
code** : (1) quick wins — fix prêt, recettes éprouvées,
décision tranchée ; (2) gaps `dom_host.rs` en batch
(sonde → fix) ; (3) surfaces builtins auto-contenues ;
(4) divergences langue (`interp`, risque moyen) ;
(5) rappel piste V ; (6) GC structural en dernier —
chantier long sur le hot path, à faire vivre en fond ou
en branche courte sans invalider les étapes courtes.

---

## JT-A — Quick wins : fix prêt, recettes éprouvées, décision tranchée

- [x] **JT-1. `drain_pending_msgs`/`drain_port_msgs` sous
  le realm routé** (`channel.rs`) — **livré (JS-C446)**.
  Constat au code : `drain_port_msgs` switchait déjà mais
  `PortMsg.ctx` = realm du **posteur** — `PortHost.home`
  (naissance) routé désormais ; `drain_pending_msgs`
  switchait sans garde → `msg.drop_ctx` ajouté (parité
  `port.drop_ctx`/microtasks). Verrou synthétique
  `c446_msgs_realm` ×3 — post cross-realm rouge sans fix
  (`false|20` → `true|20`).
- [x] **JT-2. `DocumentFragment` constructible** —
  `illegal constructor DocumentFragment` (gitlab) —
  **livré (JS-C447)** : vrai ctor dans la boucle
  d'interfaces (recette JS-C427) — `docfrag_ctor`
  retourne le wrapper du nœud `create_fragment()`
  (retour objet prime, spec `[[Construct]]`) ;
  `instanceof` gratuit via le mapping `Fragment →
  "DocumentFragment"`, chaîne `ShadowRoot` préservée.
  Verrous `c447_docfrag_ctor` ×5.
- [x] **JT-3. Globals CommonJS honnêtes en sandbox**
  (résidu §B-105) — **livré (JS-C448)** : repli sur
  échec dans `exec_program_inner` — `ReferenceError`
  `module`/`exports`/`require` → re-exec dans
  `Env::local(global_env)` semé CJS (`module`+
  `exports`+`require` borné, registre `nom → exports`
  FIFO 64, `Cannot find module` honnête). Les gardes
  `typeof` des bundles sains ne le voient jamais —
  `typeof module` reste `"undefined"` en global.
  Verrous `c448_umd_sandbox` ×6.

## JT-B — Gaps API miroir DOM (`dom_host.rs`) — batch sonde → fix

Chaque gap = une erreur nommée en vol qui abort un
bootstrap réel. Même recette pour tous : sonder l'objet
porteur d'abord (une erreur `X of Undefined` se fixe sur
l'objet qui aurait dû porter `X`, pas en avalant), puis
surface minimale — jamais un stub qui ment.

- [x] **JT-4. `toUpperCase of Undefined` (mw wikimedia)** —
  **livré (JS-C449)** : props chaîne `Document`
  (`compatMode`/`contentType`/`characterSet`/`charset`/
  `inputEncoding`) + `type` calculé spec sur
  `textarea`/`select` (hors table `TAGGED_ATTRS` —
  `el.type.toUpperCase()` mourait sur `Undefined`).
  Reflet tag-restreint §B-103 intact. Verrous
  `c449_doc_string_props` ×3.
- [x] **JT-5. `querySelectorAll` / `categories of Null`
  (jwplayer archive.org)** — **mesuré (JS-C450)** : qsa
  complet sur Document/Element/DocumentFragment/
  ShadowRoot, sélecteurs attr/complexe/liste —
  **le gap n'est pas l'API**. `categories of Null` =
  lecture de config interne (`item.categories` sur
  playlist item null) — à sonder au prochain vol,
  jamais fixé à l'aveugle. Verrous `c450_qsa_surface`
  ×2.
- [x] **JT-6. `FinalizationRegistry.unregisterToken`
  (odysee) — vérifié vs spec V8** : **livré (JS-C451)** —
  `CanBeHeldWeakly` validé : token présent non-objet →
  `TypeError` (parité V8), `undefined` = pas de jeton,
  `SameValue` intact. Deux tests `lang` basculés en
  tokens objet (acceptation non-spec des chaînes
  corrigée). Verrous `c451_finreg_token` ×2.
- [ ] **JT-7. `call of Undefined` core-js (framatube)** —
  invocation sur `Undefined` : nommer la cible (méthode
  builtin ou entrée miroir manquante) — core-js feature-
  détecte en appelant ; un `typeof` honnête suffit
  souvent à désarmer le chemin.
- [ ] **JT-8. `value is not a function` adapter Vercel
  (videojs)** — même classe que JT-7, surface adapter
  Vercel : l'objet exposé n'est pas callable là où le
  site attend une fonction (getter installé en valeur
  au lieu d'une natfn, ou méthode absente).

## JT-C — Surfaces builtins auto-contenues

- [ ] **JT-9. `crypto.subtle` disponible sans backend
  installé** (§B-117 corpus-root — `crypto.rs`) —
  odysee : bootstrap tardif abort sur `SubtleCrypto not
  available` (= le chemin `no crypto backend installed`
  de `crypto.rs`). État : JS-C79 a tranché « couture
  embedder » (jscore zéro-dep) et JS-C93 a livré
  clés/sign via `ring` **côté embedder** — mais un
  contexte câblé sans backend aborte au lieu de
  dégrader. Minimal requis par le site : `digest`
  (SHA-256) + `getRandomValues`. Deux voies à trancher
  à la prise : (a) **fallback natif dans jscore** —
  SHA-256 pur Rust ~150 lignes std-only dans
  `crypto.rs`, la promesse « autonome zéro-dep »
  survit ; (b) **dep pur Rust** (`sha2`, ≥7 jours,
  workspace-deps) — justifiée par §B-117 dans le
  commit, mais casse la promesse historique du crate.
  Préférence : (a) sauf si la table d'algos demandée
  grossit.
- [ ] **JT-10. `VTTCue`** — « chaîne suivante » nommée
  vol 722 : la classe est absente → `ReferenceError`
  sur les players qui pistonnent les sous-titres
  (videojs/jwplayer). Surface : `TextTrackCue` base +
  ctor `VTTCue(startTime, endTime, text)`, props spec
  (`vertical`/`snapToLines`/`line`/`position`/
  `positionAlign`/`size`/`align`), `getCueAsHTML()`
  minimal honnête, `onenter`/`onexit` stockés. Pas de
  moteur de rendu cue — présence API + état, la
  rasterisation n'est pas de ce périmètre.
- [ ] **JT-11. i18n bindings YouTube
  `[[getSimpleString]]`/`[[countryCode]]`** — résidu
  nommé vol 701, post-§B-97 : slots internes du binding
  i18n du player YT (syntaxe `[[…]]` = accès aux
  internes Polymer/ytcfg). Surface minimale : chaînes
  simples servies depuis `ytcfg`/la table de strings du
  miroir, `countryCode` depuis la config de session
  (jamais de géoloc inventée — valeur honnête ou
  absente documentée). Fichier : `dom_host.rs` /
  miroir bindings.

## JT-D — Divergences langue (`interp`, `parser`, `module.rs`)

- [ ] **JT-12. Throws avalés dans les promesses** (vol
  629, ouvert) — une exception dans un executor ou un
  handler disparaît au lieu de rejeter : tracer
  `promise.rs` chemin job→reject ; le symptôme site
  est un `.catch` jamais appelé + état `pending`
  éternel. Verrou : throw synchrone executor → rejet,
  throw dans `then` → propagation au rejet suivant.
- [ ] **JT-13. `import()` `inline:` non routé** ×2
  (vimeo 697) — JS-C414 a livré le repli du **referrer**
  `inline:N` → page ; le résidu est le specifier
  `import("inline:…")` **lui-même** non routé vers le
  registre des modules inline (`module.rs` — le
  parse/routage est jscore, pas NAV). Verrous :
  `c414_*` existants conservés + cas specifier inline
  résolu vers la source garée.
- [ ] **JT-14. `for..of` eager (vol 722)** — divergence
  nommée : reprendre le cas minimal du rapport de vol.
  Suspect de classe connue : l'héritage eager documenté
  (générateurs, JS-C34 a livré `GExec`/suspension
  réelle — `for..of` sur corps suspendable est le
  voisin logique). Verrou : fixture spec du cas + pas
  de régression `c34_*`.

## JT-E — Perf interprète (rappel — file tenue dans `roadmap-vitesse-js.md`)

- [ ] **JT-15. Résidus vitesse nommés — `fn_body`
  sous-parse ~7 ms, `close_table` partagée ~1,7 ms** —
  ferme les derniers sauts §B-1. **La file se tient
  dans `docs/roadmap-vitesse-js.md`** (ouvrir l'étape
  V-x correspondante) — JT-15 est le rappel ici que la
  piste V a du résidu terrain, pas une duplication de
  file. `close_table` partagée : la table `Rc<Vec>` est
  reconstruite par sub-parse alors que le stream
  `Rc<[Token]>` est déjà partagé — candidat : la porter
  sur le `Program`/sub-parser au lieu de la rebâtir
  (`parser.rs:31`, `:2477`).

## JT-F — GC/heap : résidus §B-114 + §B-110 (chantier long, `interp`/`heap`/marquage)

État : le gros a landé — chaînes hors arène `JsStr`
(JS-C420), safepoint intra-span (JS-C421), backoff de
croissance aux safepoints et frontières de job
(JS-C424/JS-C426). Reste deux résidus **du même chantier**
— le mécanisme JS-C398 (top-kinds) les a nommés ; le fix
est un seul design : **marquage incrémental** ou
**safepoint plus fin**.

- [ ] **JT-16. Churn `a.ads.rmbl.ws` (rumble) — +1 %
  overshoot résiduel** : post-backoff, la croissance heap
  dépasse encore le seuil sur ce script. Attribuer le site
  d'alloc avant tout fix (attribution J17 par éval JS-C425
  disponible, `EXPLORER_J17_PROF=1`) — l'overshoot est
  un symptôme, pas un diagnostic.
- [ ] **JT-17. Sweep sous éval — PoW `SG_SS` (~500 B/op de
  churn)** : le collecteur se déclenche en plein span
  d'éval sur un script qui ne rend jamais la main. Le
  safepoint JS-C421 borne la pause mais ne change pas la
  cadence : à ce taux de churn le sweep repart sitôt
  terminé. Options (tranchées à la prise, mesures J17 à
  l'appui) : (a) **marquage incrémental** — tranches de
  mark bornées entrelacées aux safepoints, write-barrier
  ou re-scan borné à documenter ; (b) **safepoint plus
  fin** — granularité sous le span (tick de boucle déjà
  instrumenté). Verrous : pas de régression sur la suite
  `hostile` release, overshoot rumble mesuré ≤ seuil,
  `SG_SS` atteint son espace d'exception sans mourir au
  budget (réglages diagnostic Étape 123/124).

## JT-J — Dormants langue (file prête, règle J-B)

Rappel de file uniquement — les étapes restent
numérotées `JS-Cxx` dans la roadmap NAV et **ne
s'ouvrent pas sans déclencheur documenté** (bug site
réel, échec `site_smoke`, besoin produit) :

- **JS-C30.** Extension corpus test262 (sous-ensembles
  `builtins/`/`language/` — déclencheur : reporting).
- **JS-C31.** `Temporal` complet (`temporal.rs` existe —
  `ZonedDateTime`, `round`, calendriers).
- **JS-C32.** `Intl` multi-locale (CLDR via `icu4x` —
  `intl.rs` existe ; ADR de dépendance exigée).
- **JS-C33.** Décorateurs ES2026.

---

## Traçabilité constat → étape

| Constat terrain | Étape |
|---|---|
| `drain_*_msgs` sous dernier ctx (vol 732) | JT-1 |
| `illegal constructor DocumentFragment` (gitlab) | JT-2 |
| `module is not defined` UMD (§B-105) | JT-3 |
| `toUpperCase of Undefined` (mw wikimedia) | JT-4 |
| `querySelectorAll`/`categories of Null` (jwplayer, archive.org) | JT-5 |
| `FinalizationRegistry unregisterToken` (odysee) | JT-6 |
| `call of Undefined` core-js (framatube) | JT-7 |
| `value is not a function` adapter Vercel (videojs) | JT-8 |
| `SubtleCrypto not available` (odysee, §B-117) | JT-9 |
| `VTTCue` absent (vol 722) | JT-10 |
| `[[getSimpleString]]`/`[[countryCode]]` (vol 701) | JT-11 |
| Throws avalés promesses (vol 629) | JT-12 |
| `import()` `inline:` non routé ×2 (vimeo 697) | JT-13 |
| `for..of` eager (vol 722) | JT-14 |
| `fn_body` ~7 ms / `close_table` ~1,7 ms (§B-1) | JT-15 → piste V |
| Overshoot rumble `a.ads.rmbl.ws` +1 % post-backoff | JT-16 |
| Sweep en span d'éval, PoW `SG_SS` ~500 B/op churn | JT-17 |
