# ADR-0012 — Pipeline incrémental : sous-ressources, entrée par chunks, invalidation

Statut : Acceptée (2026-10-19).

## Contexte

Le schéma `fetch → parse → style → layout → render` de
`docs/architecture.md` décrit le **sens des données**, pas un
pipeline linéaire à un seul passage. Une page réelle impose trois
réalités absentes d'un découpage naïf :

1. **Sous-ressources parallèles** : `<link rel=stylesheet>`,
   `<img>`, `<script src>` déclenchent des fetches *pendant* le
   parse, avec des politiques (`async`/`defer`, priorités) qui
   ordonnent la fin du pipeline.
2. **Entrée incrémentale** : la page se télécharge en chunks ; le
   navigateur doit pouvoir parser/peindre des données partielles
   (AGENTS.md §8) — y compris le sniffing d'encodage qui consomme
   les premiers octets avant de stabiliser le transcodage.
3. **Invalidation continue** : mutation DOM (parser ou script),
   feuille CSS tardive, pseudo-classes (`:hover`, `:focus`),
   redimensionnement viewport → le style et le layout doivent être
   recalculés *partiellement*, jamais en rasterisant un arbre en
   cours de mutation (« une frame = un état cohérent »).

Trancher ces mécaniques à la phase où elles deviennent pressantes
imposerait de revisiter l'API de chaque couche — elles structurent
dès maintenant la forme des entrées/sorties.

## Décision

- **Scheduler de sous-ressources dans `explorer-core`** : le
  parseur émet des événements `Subresource{kind, url, attrs}` dès la
  rencontre d'une balise ; `core` les route vers `HttpTransport` via
  des files bornées par onglet (priorité document > CSS > images ;
  `Referer`/`Origin` et politique mixed-content calculés à
  l'émission — cf. étape 8). Le domaine (`html`, `css`) ne connaît
  que des IDs de ressource opaques, jamais du réseau.
- **Entrée par chunks bornés** : le tokenizer HTML accepte un flux
  `push(&str)` (étape 10) ; le transcodeur d'encodage est un état
  stable après la fenêtre de sniffing (BOM → `<meta>` → défaut).
  Chaque couche tolère des données partielles et signale ses points
  d'invalidation.
- **Modèle d'invalidation en trois niveaux** : `DirtyStyle`
  (recascade sur sous-arbre), `DirtyLayout` (reboîtage/refixage),
  `DirtyPaint` (dirty rects → display list partielle). Mutation DOM,
  arrivée d'une feuille externe, changement de pseudo-classe ou
  resize posent des drapeaux ; le pipeline recalcule à frontière de
  tick (jamais en ligne de la mutation) et ne publie que des états
  complets — le single-writer (AGENTS.md §4.8) est préservé.
- **`<script>`** : en phase 8, collecte + ordre d'exécution
  (document order, `defer`/`async`) seront des règles du scheduler ;
  d'ici là les balises sont ignorées proprement.

## Justification

- Un pipeline à passage unique ne peut pas charger un site réel :
  sans scheduler, les CSS externes et images restent lettre morte.
- Les trois niveaux de dirty flags sont la forme la plus simple qui
  couvre restyle (nouvelle feuille), reflow partiel (mutation) et
  repaint seul (scroll/dirty rects, ADR-0004).
- Le différer coûterait une refonte des signatures publiques de
  `html`, `css`, `layout` à la phase où l'équipe a le moins de
  marge.

## Conséquences

- `explorer-html` ne fait pas de `fetch()` : il émet des événements
  de sous-ressources ; `explorer-core` orchestre — règle de
  dépendance intacte.
- Les tests peuvent injecter des sous-ressources in-process via
  `explorer-test-support` (transport simulé).
- Les points d'invalidation de chaque couche sont documentés dans
  sa rustdoc dès la phase où la couche est écrite (phases 2–5).
- Les budgets par onglet (ADR-0011) s'appliquent aussi aux files de
  sous-ressources : une page qui spamme des fetches est bornée, pas
  ses voisins.
