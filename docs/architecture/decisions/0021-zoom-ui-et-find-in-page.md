# ADR-0021 — Zoom de page côté UI ; recherche dans la page reclassée

Statut : Acceptée (2026-11-07) — arbitrage de l'étape 57
(UI Flutter v2).

## Contexte

L'étape 57 (refonte de l'interface) posait deux candidats au
périmètre : le zoom de page et la recherche dans la page
(find-in-page). Le ticket exigeait un arbitrage explicite.

## Décision

### Zoom — livré, entièrement côté UI

Le moteur expose déjà les deux paramètres nécessaires via
`POST /api/tabs/{id}/viewport` : la taille du viewport **en px
CSS** (`w`, `h` — largeur de layout) et le facteur DPI
(`scale` — uniquement raster). Le zoom classique s'obtient sans
aucune nouvelle API :

```
setViewport(w / zoom, h / zoom, zoom × dpr)
```

le layout est refondu à `w/zoom` unités CSS (reflow réel, pas
un étirement de bitmap), puis rasterisé à `w×dpr` px physiques
— le texte reste net. Les coordonnées d'entrée se divisent par
`zoom × dpr` (1 px CSS = `zoom×dpr` px logiques affichés,
l'image étant peinte `fit:none`).

Contrôles : menu « zoom » de la barre de navigation et
raccourcis Ctrl `+`/`−`/`0` ; facteur borné `[0,25 ; 4]` par
pas de ×1,25 ([`ZoomController`] en session applicative, non
persisté — un zoom survit rarement à la session).

### Find-in-page — reclassé hors étape 57

Une recherche honnête exige une surface moteur : parcourir le
DOM textuel, surligner les occurrences dans la display list et
faire défiler jusqu'à elles. C'est un contrat
`explorer-core`/`explorer-render` (NAV consomme une API
PIPELINE possible : surlignage = items supplémentaires ou
dirty-rects annotés), pas un travail de widgets. Reclassé à
une étape dédiée de la roadmap NAVIGATEUR qui ouvrira la
demande dans `docs/interface-equipes.md`.

## Conséquences

- Zoom disponible dès l'étape 57 sans toucher le moteur ni le
  contrat REST — coût nul, conforme ADR-0003 (l'app ne parle
  que l'API).
- En mode FFI le même mécanisme s'applique : `explorer_set_viewport`
  porte le même triplet `(w, h, scale)`.
- La recherche dans la page reste absente de l'UI (pas de
  Ctrl+F) jusqu'à l'étape dédiée — documenté dans la roadmap.
