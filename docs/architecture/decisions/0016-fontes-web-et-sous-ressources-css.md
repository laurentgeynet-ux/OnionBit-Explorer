# ADR-0016 : Fontes web (`@font-face`, WOFF) et premières sous-ressources CSS

- **Statut** : accepté
- **Date** : 2026-02
- **Contexte** : étape 46 — « Webfonts + charsets ». Les charsets
  étaient déjà couverts (`explorer-html::encoding` : BOM, meta
  sniff, windows-1252, étape 5). Restait la moitié « fontes » :
  `@font-face`, fetch des fichiers de fontes, WOFF/WOFF2 —
  majoritairement servies depuis des feuilles `<link>` externes
  sur le web réel.

## Décision

1. **`@font-face` conservé hors cascade d'éléments.** La
   cascade les ignorait purement ; `StyleEngine` stocke
   désormais les blocs avec leur pile `@media` englobante et
   les sert via `font_faces(env)` — descripteurs
   `font-family`, `src` (`url()`/`local()`), `font-weight`,
   `font-style`, `font-display`. Ce ne sont pas des règles de
   style : elles ne peuvent pas être sélectionnées par les
   éléments.
2. **`WebFontSource` : source composite en infrastructure.**
   `explorer-render::fonts::WebFontSource` enveloppe le
   `FontSource` local injecté : faces web d'abord (matching
   famille normalisée + meilleure graisse/style), repli local
   ensuite. Registre borné (`max_fonts`, 16), chaque face
   validée par `ttf-parser` à l'inscription, purge à chaque
   navigation (les `@font-face` sont par document, spec CSS
   Fonts §4.1 — jamais exposées à d'autres documents).
3. **Fetch synchrone avant layout** — comportement
   `font-display: block` pour toutes les faces. Une frame =
   un état cohérent : pas de swap/re-layout quand la fonte
   arrive. `font-display` est parsé et conservé mais
   sémantiquement ignoré (documenté, à revoir avec le
   pipeline incrémental).
4. **Décodeur WOFF dédié** (`explorer-render::woff`) :
   - **WOFF1 complet** (répertoire + tables zlib/brutes →
     reconstruction sfnt avec checksums).
   - **WOFF2 à transform nulle uniquement** : le flot brotli
     est décompressé et les tables reconstruites, mais les
     prétransformations `glyf`/`loca`/`hmtx` (spec §5) ne
     sont pas décodées — la fonte est refusée proprement et
     la famille retombe sur la suivante. Une part non
     négligeable des WOFF2 réels passe par le glyf transformé ;
     c'est la limite connue la plus importante de l'étape.
   - Collections `ttcf` refusées.
   - Bornes : `numTables ≤ 64`, table ≤ 32 Mio, sortie ≤
     64 Mio, décompression via lecteurs plafonnés.
5. **Premières sous-ressources CSS** : `<link rel=stylesheet>`
   fetché séquentiellement borné (`max_sheets`, 8) — les
   feuilles rejoignent la cascade en origine auteur (ordre
   après le `<style>` inline) ET fournissent leurs
   `@font-face`. Sans cela les fontes web ne se déclencheraient
   quasiment jamais sur le web réel. `@import` dans les
   feuilles n'est pas encore suivi (le parser expose
   `ImportRule` — étape suivante naturelle).

## Conséquences

- `local(name)` dans `src:` fonctionne sans code : la famille
  retombe sur la source locale interne, qui résout par nom.
- Un échec individuel (résolution, fetch, statut, décodage,
  validation) ignore la source — dégradation vers le fallback,
  jamais d'erreur de navigation.
- `font-stretch`, `unicode-range`, la synthèse gras/italique
  et le matching de graisse fin (algorithme CSS Fonts §4.5)
  restent hors périmètre — le score retenu est « italique
  d'abord, distance de graisse ensuite ».
- Le fetch séquentiel borné est volontairement simple ; la
  parallélisation des sous-ressources relève de la passe perf
  de l'étape 48, pas d'un compromis d'architecture.
