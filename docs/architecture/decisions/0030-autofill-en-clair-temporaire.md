# ADR-0030 — Autofill en clair (déviation temporaire)

Date : 2026-10-30 · Statut : **accepté, conditionnel** — à
révoquer par `ProfileVault` **avant release prod**.

## Contexte

PR-6 ajoute `autofill.json` au profil — besoin de
développement (tests de formulaires réels). Le chiffrement
profil (`ProfileVault`, ADR-0028) n'existe pas encore.

## Décision

Stocker `autofill.json` **en clair** pendant le
développement, sous les garde-fous suivants :

1. **Jamais de secret** : `password`, `hidden`, `file`,
   `checkbox`/`radio` et boutons sont exclus en amont
   (`collect_autofill_data`) — seuls des champs textuels
   (nom, adresse, email, selects) sont persistés.
2. **Borné** : 10 valeurs par nom, 2000 noms.
3. **Pas de release** : cette déviation est interdite en
   distribution publique — le sealing `ProfileVault`
   (ADR-0028) **doit** couvrir `autofill.json` avant toute
   release prod.

## Conséquences

- Risque assumé en dev : une clé USB perdue expose les
  données de formulaire saisies (mais jamais de mot de passe).
- Quand `ProfileVault` arrive : `autofill.json` rejoint le
  périmètre scellé — la couche `ProfileStore` est le point
  d'injection prévu (règle transversale `roadmap-profil`).
