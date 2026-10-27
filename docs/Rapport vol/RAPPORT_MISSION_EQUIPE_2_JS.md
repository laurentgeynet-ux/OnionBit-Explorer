# 🛰️ RAPPORT D'ANALYSE EN VOL : ÉQUIPE 2 (SPAS & FRAMEWORKS JAVASCRIPT)

**Périmètre** : Frameworks réactifs (HTMX, Vue, React, Alpine, Svelte, Preact, Solid).  
**Objectif** : Traquer les régressions d'environnement JS, compatibilité DOM Host, et exécution des micro-tâches.

---

## 📋 Synthèse des vols (Sites Inédits)

| N° | Date / Heure | URL cible inédite | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---:|:---|
| **01** | 2026-10-26 12:58 | `https://htmx.org/` | 🔴 MUR JS | `TypeError: createExpression is not a function` + extensions preload/class-tools bloquées. |
| **02** | 2026-10-26 13:01 | `https://vuejs.org/` | 🟢 SUCCÈS | Rendu SSR hydraté, repli re-parse `why=Fingerprint` absorbé proprement. |
| **03** | 2026-10-26 13:04 | `https://getbootstrap.com/docs/5.3/getting-started/introduction/` | 🔴 MUR JS MULTIPLE | **Murs critiques Bootstrap 5.3** : Type check Popper/Tooltip échoué + `cannot read 'call' of Undefined`. |
| **04** | 2026-10-26 13:22 | `https://svelte.dev/` | 🔴 MUR JS SVELTEKIT | **Mur SvelteKit** : `TypeError: Invalid base URL` sur `new URL(".", location).pathname`. |
| **05** | 2026-10-26 13:30 | `https://alpinejs.dev/` | 💥 STACK OVERFLOW | **Crash critique moteur** : `thread 'explorer-jscore' (15432) has overflowed its stack` (récursion infinie parser/AST). |
| **06** | 2026-10-26 13:30 | `https://preactjs.com/` | 🟢 SUCCÈS PREACT | Rendu SSR + hydratation Preact, 5 replis re-parse contrôlés (`why=Fingerprint`), 0 crash JS. |

---

## 🔬 Fiches Techniques Détaillées d'Anomalies

### ✈️ Vol #05 — `https://alpinejs.dev/` (Alpine.js Documentation)
- **Stack technique** : Alpine.js 3.x, Tailwind CSS.
- **Rendu visuel** : Crash immédiat du process de test lors du parsing d'un bundle JS.

#### 💥 Mur Critique Détecté (`explorer-jscore`) :
```text
thread 'explorer-jscore' (15432) has overflowed its stack
```

#### 🔍 Analyse Racine pour le Spécialiste JS :
1. **Dépassement de pile récursive** : Dans `crates/explorer-jscore/src/parser/mod.rs` ou l'interpréteur AST, une structure syntaxique imbriquée dans le bundle d'Alpine (ex. chaînage d'expressions ternaires ou de fonctions fléchées imbriquées sans bornes) entraîne une récursion infinie de fonctions Rust sur le thread `explorer-jscore`.
2. **Priorité Haute** : Tout contenu hostile ou trop profond peut faire crasher le thread sans interception possible si la limite de récursion n'est pas bornée par un compteur (`depth_limit`).

#### 🧪 Repro Minimal Test (Pour Agent 3) :
```javascript
// Test: Borne de récursion d'analyse syntaxique (ne doit jamais faire crasher le thread Rust)
let code = "x" + ".y".repeat(5000);
eval(code);
```

---

### ✈️ Vol #04 — `https://svelte.dev/` (SvelteKit Official Site)
- **Stack technique** : Svelte 5 / SvelteKit router.
- **Mur Détecté** : `TypeError: Invalid base URL` sur `base: new URL(".", location).pathname`.
- **Analyse** : Bogue transverse partagé avec MkDocs/FastAPI. `Location` n'est pas converti en string dans `new URL(rel, base)`.


### ✈️ Vol du 2026-10-26 13:37 — `https://mithril.js.org/`
- **Ressource & Rôle** : Framework SPA client-side ultra-léger
- **Statut d'atterrissage** : ⚠️ REPLI DOM (ADR-0025)
#### 💥 Anomalies détectées :
```text
2026-10-26T12:37:38.081006Z  WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025) tab=1 why=NoBaseline
[media post-nav : 0 offres]
[round 0 media=0]
[round 1 media=0]
```


### ✈️ Vol du 2026-10-26 13:37 — `https://riot.js.org/`
- **Ressource & Rôle** : Framework micro-composants Riot.js
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:38 — `https://markojs.com/`
- **Ressource & Rôle** : Moteur de rendu réactif eBay Marko
- **Statut d'atterrissage** : ⏳ TIMEOUT : requête ou exécution script bloquée au-delà de 25s
#### 💥 Anomalies détectées :
```text
Timeout jprobe après 25s
```


### ✈️ Vol du 2026-10-26 13:38 — `https://aurelia.io/`
- **Ressource & Rôle** : Framework moderne Aurelia
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:38 — `https://emberjs.com/`
- **Ressource & Rôle** : Framework SPA structuré d'entreprise
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:38 — `https://knockoutjs.com/`
- **Ressource & Rôle** : Framework MVVM historique avec bindings déclaratifs
- **Statut d'atterrissage** : ⚠️ REPLI DOM (ADR-0025)
#### 💥 Anomalies détectées :
```text
2026-10-26T12:38:48.922448Z  WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025) tab=1 why=UnmappedKey
[media post-nav : 0 offres]
[round 0 media=0]
[round 1 media=0]
```


### ✈️ Vol du 2026-10-26 13:39 — `https://backbonejs.org/`
- **Ressource & Rôle** : Bibliothèque modèle/vue historique Backbone
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:39 — `https://infernojs.org/`
- **Ressource & Rôle** : Moteur haute performance compatible React
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.
