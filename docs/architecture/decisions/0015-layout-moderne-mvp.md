<!-- This file is part of Explorer. -->
<!-- Copyright (C) 2026 Laurent Geynet -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

# ADR-0015 — Layout moderne en MVP borné (flex, grid, transforms, sticky, var/calc)

- **Statut** : accepté
- **Date** : 2026-10-19
- **Contexte** : étape 42 — le web moderne repose sur
  `display:flex`/`grid`, `transform`, `position:sticky`,
  `calc()` et les custom properties. Les implémenter « à la
  spec complète » (max-content, wrap multi-lignes, spanning
  implicite, fragmentation) est un chantier de mois ; ne rien
  faire laisse les pages réelles dégradées en flux bloc.

## Décision

1. **`--var`/`var()`** : substitution textuelle au temps de
   valeur calculée (`compute_with_parent`, étape 5), récursion
   bornée à 8, chaîne `var()` cassée → valeur garantie-
   invalide → fallback ou retrait de la déclaration
   (IACVT approximé).
2. **`calc()`** : mini-évaluateur `+ − × ÷` + parenthèses +
   `calc(` imbriqué, branché dans `css_px` — goulot unique de
   résolution des longueurs du layout.
3. **Flex** : une ligne (`nowrap`), row/column, `order`,
   grow/shrink/basis, `justify-content`, `align-items`,
   `gap`. Les items sont les boîtes existantes posées par
   `layout_block`/`layout_anon_block` à largeur de piste.
4. **Grid** : template explicite (`px`/`%`/`fr`/`auto`/
   `repeat`/`minmax→max`), spans, auto-placement row-major
   sans backfill, `gap`.
5. **Transforms** : `PushTransform`/`PopTransform` dans la
   display list ; le raster compose `dpi · pile` et l'applique
   à tous les items (rects, bordures, **glyphes**, images —
   nearest si axis-aligned, `draw_pixmap` affine sinon).
6. **Sticky** : `Position::Sticky` reste en flux ; la
   contrainte est une **translation de peinture** bornée par
   le bas du parent — réutilise `PushTransform`, zéro re-flow.

## Alternatives écartées

- **Layout flex/grid dédié hors de `layout_block`** : aurait
  dupliqué marges/bordures/dimensionnement — le dispatch dans
  `flow_children` réutilise tout le sizing existant.
- **Mesure max-content réelle** : deux passes de layout par
  item ; reportée — les bases `auto` non résolues mesurent le
  texte direct puis se partagent le libre à parts égales.
- **Transform au niveau des items émis** : les coordonnées
  pré-transformées cassent les rotations de rects ; la pile
  de matrices dans le raster couvre uniformément texte,
  images et clips (les clips transformés restent en bbox —
  approximation assumée).
- **Sticky via re-layout au scroll** : coûteux ; la
  translation de paint est exacte tant que le sticky ne
  chevauche pas ses voisins (cas dominant).

## Conséquences / écarts connus (documentés en tête de modules)

- `flex-wrap`, `margin:auto` des items, `align-content`,
  baseline-align : absents.
- `stretch` ≈ `flex-start` (pas de forçage de taille cross).
- Grid : pas d'areas/noms de lignes, `fr` vertical ≈ `auto`,
  indices négatifs ignorés.
- Transforms : pas de `transform-style: preserve-3d`, les
  ancêtres transformés ne re-parentent pas les `fixed`.
- `transform` fonction inconnue → transform ignorée entière.
