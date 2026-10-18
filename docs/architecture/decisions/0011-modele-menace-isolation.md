# ADR-0011 — Modèle de menace : mitigations actives, isolation processus différée

Statut : Acceptée (2026-10-19) — isolation OS **différée assumée**,
réévaluation obligatoire en phase 8.

## Contexte

`explorer-daemon` est un processus unique qui parse du contenu
hostile (HTML/CSS/HTTP arbitraires) pour tous les onglets. Les
mitigations déjà décidées : `#![forbid(unsafe_code)]`, budgets
bornés (temps/mémoire/profondeur — AGENTS.md §8), parsers tolérants,
loopback + clé API. Restent hors couverture : bug logique affectant
plusieurs onglets, épuisement de ressources d'un onglet pénalisant
les autres, fuite de données entre contextes de session.

## Décision

**Mitigations actives dès le MVP :**

- budgets par onglet et par page (mémoire, profondeur DOM, temps de
  pipeline, nombre de roots JS — ADR-0008) : un onglet hostile est
  tué ou dégradé, pas ses voisins ;
- namespaces de session : cookies, cache, historique cloisonnés par
  contexte de navigation — les futurs profils utilisateurs sont des
  contextes distincts, jamais partagés par défaut ;
- aucun bind non-loopback, jamais (ADR-0003) ;
- répertoire de profil déplaçable, permissions minimales.

**Isolation OS différée (documentée, pas implémentée) :**

- pas de sandbox (job objects/seccomp), pas de processus par site au
  MVP. La surface exploitable sans `unsafe` dans le code maison est
  réduite ; le vecteur principal est le DoS — couvert par les
  budgets ;
- **réévaluation obligatoire en phase 8** : exécuter du JS hostile
  change le modèle de menace (code arbitraire in-process au même
  titre que les données) — ADR dédié avant de brancher `dillo-js`.

## Justification

- La mémoire-safety de Rust + `forbid(unsafe_code)` élimine la
  classe dominante des exploits de parseur ; ce qui reste est
  logique, géré par cloisonnement de données et budgets.
- Site-isolation en phase 0 sur-dimensionnerait le MVP ; le différer
  sans l'oublier — avec un point de réévaluation fixe — est le bon
  niveau de rigueur.
- Le daemon peut être relancé sans tuer l'UI (ADR-0003) : un crash
  logique résiduel reste rattrapable.

## Conséquences

- Un bug logique peut encore affecter plusieurs onglets — accepté au
  MVP, explicitement.
- Toute fonctionnalité chargeant des sous-ressources (`<img>`,
  CSS `url()` — phase 9+) documente ses effets requêtes et le
  cloisonnement des credentials/cookies par contexte : la
  navigation provoque déjà des requêtes arbitraires, c'est le
  métier ; le point dur est que les secrets d'un contexte ne
  fuient pas dans un autre.
- Site isolation multi-processus reste un chantier de phase 10+ :
  le single-writer par onglet n'y fait pas obstacle, les pipelines
  sont déjà isolés logiquement.
