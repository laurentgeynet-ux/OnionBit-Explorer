# Roadmap — Profil `state/` (piste PR)

Piste de travail dédiée au **contenu du profil utilisateur** —
désignation `PR-N` (distincte des `Étape NN` produit et des
`JS-CNN`) pour éviter toute collision de numérotation. Equipe
NAVIGATEUR (host : `explorer-core`, `explorer-ffi`, `app/`) ;
décidée 2026-10-30 avec le bundle portable (Étape 167).

## Règles transverses

- **Un fichier JSON par domaine** dans `state/` — écriture
  atomique (tmp + rename), pattern `JsonFileStore`.
- **Rien d'absolu persisté** — chemins relatifs à `state/`
  (lettre de clé variable).
- **Borné** — cap par store (comme l'historique à 5000).
- Tout accès fichier profil passe par le port `ProfileStore` —
  jamais `fs` direct : `ProfileVault` (ADR-0028) doit pouvoir
  sceller sans refactor.

## Déjà présent

`history.json`, `bookmarks.json`, `cookies.json` (+ CHIPS),
`storage-*.json` (localStorage), `cache/` (HTTP), `downloads/`,
`ui_settings.json`, `logs/`, `explorer.lock`.

## Palier PR-P1 — petits stores (un fichier chacun)

- [x] **PR-1. `downloads.json`** — historique des
  téléchargements : URL, chemin relatif à `state/downloads`,
  taille, date, état. `ProfileStore::record_download` au
  succès de `accept_download`, `downloads()` pour la lecture ;
  cap 2000, écriture atomique.
- [x] **PR-2. `zoom.json`** — origine → niveau de zoom
  (`ZoomStore` + `ZoomController` par origine — recharge au
  changement d'URL active ; voisin de `ui_settings.json`).
- [x] **PR-3. `hsts.json`** — capte `Strict-Transport-Security`,
  upgrade `http→https` avant fetch + expiry. Décorateur
  `HstsTransport` (`explorer-net`, port `HstsStore` injecté) +
  store `JsonHstsStore` (`explorer-core`) câblé dans les deux
  piles (daemon + FFI).
- [x] **PR-4. `session.json`** — restauration onglets/URLs/
  scroll au boot (`SessionStore` côté app, voisin de
  `ui_settings.json` ; scroll positionnel rejoué en
  `scroll(dy)` au premier `navigated`).
- [ ] **PR-5. État fenêtre** — taille/position/maximisé dans
  `ui_settings.json` (côté app). **Reporté** : exige un plugin
  fenêtre (`window_manager` refusé pour l'instant) — sans lui
  l'app lit la taille mais ne peut pas la fixer ; persister
  des bounds non restorables = store mort. À reprendre si un
  plugin fenêtre entre dans `pubspec`.

## Palier PR-P2 — stores avec consommateur à créer

- [ ] **PR-6. `autofill.json` (chiffré)** — détection `<form>`
  ADR-0014 existante ; exige ProfileVault.
- [ ] **PR-7. `permissions.json`** — notifications/géoloc/cam :
  store mort tant qu'aucune API ne le consomme — à faire avec
  le premier consommateur.
- [ ] **PR-8. `cert_decisions.json`** — exceptions certificats
  (rustls custom verifier — sensible sécurité).
- [x] **PR-9. `dns_cache.json`** — borné (cap 4000), TTL 300 s
  (`getaddrinfo` ne rapporte pas le TTL RR — borne documentée) ;
  `CachingResolver` + `JsonDnsStore`, câblé daemon + FFI.

## Palier PR-P3 — étapes dédiées (ADR probable chacune)

- [ ] **PR-10. IndexedDB** — moteur clé-valeur transactionnel
  par origine : base embarquée, pas un JSON.
- [ ] **PR-11. Mots de passe** — ProfileVault (AEAD par
  fichier, passphrase/invité) + détection + UI remplissage.
- [ ] **PR-12. Service Workers + CacheStorage** — cycle de
  vie + interception fetch : réécrit une partie du rail réseau.
- [ ] **PR-13. Extensions** — manifest, sandboxing, surface
  API : produit à part.
- [ ] **PR-14. PWA** — manifest, icônes, standalone ; après
  PR-12.
- [ ] **PR-15. Widevine/licences** — délégation CDM
  (ADR-0024/0027).
- [ ] **PR-16. Sync multi-device** — protocole + serveur.
- [ ] **PR-17. Crash dumps** — minidump ; instrumentation
  plus que store.
