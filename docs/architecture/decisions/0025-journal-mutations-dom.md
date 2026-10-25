# ADR-0025 — Synchro miroir JS : du re-parse HTML au journal de mutations

Statut : **Acceptée** — pilote livré (étape 111) ; reliquat
ouvert 2026-10-25 (§ Ouverture, étape 137). Issue de la
revue d'architecture externe validée au journal
`interface-equipes` (08/11).

## Contexte

La synchro DOM actuelle sérialise le miroir JS en une
chaîne HTML complète, puis `apply_payload` re-parse ce
document (`TreeBuilder`) — chaque apply régénère **tous
les `NodeId`**. Conséquences mesurées :

- **Toute table indexée `NodeId` meurt à chaque apply** —
  `PaintImages.replaced`/`backgrounds` (Étape 108 : tout
  `<img>`/fond http(s) disparaissait à la première synchro
  post-fetch — logo du portail Wikipedia), `find`, popup
  `<select>`, focus (`focused_key` re-ancre par identité),
  `prev_styles` (clé = ordinal d'élément, pas NodeId, pour
  survivre), canvases (re-résolution par chemin à chaque
  apply), `relayout_subtree` (remap `element_path`).
- **Coût par apply** : re-parse complet + re-cascade +
  re-key de toutes les tables, même pour une mutation
  d'un caractère. La coalescence `sync_floor` borne la
  cadence, pas le coût unitaire.
- **Complexité compensatoire** : chaque donnée
  `NodeId`-keyée invente son propre mécanisme de survie
  (`img_decoded`/`bg_decoded` par clé d'attribut, ordinals,
  chemins d'élément) — des recettes individuelles là où
  un invariant unique (« les `NodeId` sont stables »)
  suffirait.

Le re-parse a une vertu à préserver : il est
**auto-réparant** — le DOM engin est toujours la
sérialisation exacte du miroir, un apply en échec ne
laisse pas d'état divergent.

L'infrastructure différentielle existe déjà : JS-C26
(`MutationObserver`) produit des `MutationRecord`
(`childList`/`attributes`/`characterData`) sur le miroir
— le signal de diff est capturé, aujourd'hui consommé
par les listeners JS puis perdu pour la synchro.
**ADR-0013 §limites nomme déjà ce point d'extension** :
« un diff d'arbre miroir→réel (même interface `sync_dom`,
autre transport) » — cette ADR n'invente pas la
direction, elle la met en œuvre.

## Décision

Remplacer la synchro par sérialisation+re-parse par un
**journal de mutations** : le miroir JS émet une liste
ordonnée d'opérations différentielles appliquées
directement à l'arène `explorer-dom` :

```
InsertNode(parent_key, subtree_html | node_ref, index)
RemoveNode(key)
SetAttr(key, name, value | None)
SetText(key, text)
```

`key` = identité stable de nœud à travers la couture
(ordre document au commit ou chemin d'élément — à figer
au pilote ; le `NodeId` brut ne traverse pas, il n'a de
sens que dans son arène).

**Pilote scopé** : opérations structurelles
(`appendChild`/`insertBefore`/`removeChild`) +
`characterData`/`setAttribute` — les mutations qui
alimentent >90 % des synchros réelles. Tout record non
mappable déclenche le **repli re-parse** — la vertu
auto-réparante est conservée comme garde-fou, pas
comme chemin nominal. Cas explicitement hors pilote
(le re-parse leur offre les fixups HTML5 gratuitement) :
**foster parenting** (`<table>` — insertion redirigée
hors table), fragments `innerHTML` (parsing contextuel),
`<script>` exécuté dans un fragment inséré, mutations
massives (seuil à calibrer au pilote).

**Garde de cohérence** : chaque journal porte le compte
d'ops + une empreinte (hash structurel tag/ordre, pas de
texte) du miroir post-mutation ; l'apply vérifie
l'empreinte du DOM résultant — divergence → re-parse
immédiat + compteur de repli `tracing` (la fréquence de
repli est la mesure de fiabilité du journal).

## Alternatives

- **Statu quo (re-parse systématique)** — auto-réparant
  et simple, mais détruit les `NodeId` (cause racine
  d'Étape 108 et des mécanismes de survie associés) et
  impose le coût parse+cascade complet à chaque battement
  JS. Rejeté comme chemin nominal, conservé comme repli.
- **Hybride par taille** (journal ≤ N ops, re-parse au-
  delà) — absorbé dans le pilote : le repli re-parse
  couvre naturellement les mutations massives.
- **DOM partagé unique** (le miroir JS *est* le Dom
  moteur) — supprime la synchro mais casse l'isolation
  du moteur JS (`explorer-jscore` autonome, mini-DOM
  harnais) et le single-writer de l'arène ; rejeté.

## Conséquences

- **`NodeId` stables entre applys** → les tables
  `NodeId`-keyées survivent nativement. `img_decoded`/
  `bg_decoded` restent utiles comme caches de décodage
  (dédup), mais n'ont plus à assurer la survie — code à
  simplifier quand le journal est le chemin nominal.
- **Le diff est le signal dirty** : les ops portent la
  cible exacte → ciblage `relayout_subtree` précis sans
  `take_dirty_path` ni remap.
- **Risque = désynchronisation silencieuse** (op mal
  rejouée, ordre faussé) : atténué par l'empreinte +
  repli re-parse + compteur de repli ; un journal
  qui replie >seuil sera re-haussé en investigation.
- **Travail non couvert par cette ADR** : identité de
  nœud à travers la couture (choix de `key`), mapping
  des records JS-C26 → ops, ordre des ops imbriquées —
  à figer dans le pilote (étape NAV à créer).

## Ouverture 2026-10-25 — reliquat (étape 137)

Pilote soldé (étape 111) : voie différentielle tentée
d'abord à chaque synchro, empreinte + repli re-parse +
compteurs (`journal_applies`/`journal_fallbacks`/
`journal_classified`, exposés `explorer-cli`), `NodeId`
stables → survie native des tables. Le journal est de
fait le **chemin nominal** — l'ADR est acceptée. Items
restants, ouverts en étape 137 :

1. **Ciblage relayout par les ops** (conséquence « le
   diff est le signal dirty » — non livrée) : après
   rejeu en place, `apply_journal` paie encore
   `relayout_dom_payload` complet ; les ops portent la
   cible exacte → `relayout_subtree` direct, sans
   `take_dirty_path` ni remap. Le chemin
   `try_incr_payload` reste alimenté par `job.dirty`
   (R-12), parallèle au journal — à unifier.
2. **`DomMutation::AttachShadow`** : l'attache
   programmatique `attachShadow` voyage par re-parse
   (borne documentée R-96) — nouvelle op de couture
   `(host, mode, sous-arbre)` ; `Dom::attach_shadow_root`
   existe côté PIPE, consommation pure (périmètre NAV).
3. **Seuil de repli calibré** : mesurer
   `journal_fallbacks` sur corpus réel (vols terrain),
   fixer le seuil « repli > x → investigation » de la
   section Conséquences.
4. **Simplification `img_decoded`/`bg_decoded`
   (optionnelle)** : le journal nominal rend la survie
   par clé inutile — les caches restent utiles en dédup
   de décodage ; nettoyage de confort seulement.
5. **Limite connue** : index d'insertion décalé d'un
   cran face aux commentaires résiduels — invisible à
   l'empreinte ; à traiter si un cas réel le révèle.
