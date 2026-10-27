# 🛰️ RAPPORT D'ANALYSE EN VOL : ÉQUIPE 1 (DOCS, SPÉCIFICATIONS & SAVOIR)

**Périmètre** : Documentation technique, spécifications formelles, portails de référence.  
**Objectif** : Vérifier le pipeline de rendu textuel, la gestion des encodages, les feuilles de style documentaires et l'isolation des scripts inline.

---

## 📋 Synthèse des vols (Sites Inédits)

| N° | Date / Heure | URL cible inédite | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---:|:---|
| **01** | 2026-10-26 12:57 | `https://sqlite.org/index.html` | 🟢 SUCCÈS | Rendu nominal complet, exécution script `toggle_search` et injection DOM sponsors réussie. |
| **02** | 2026-10-26 13:06 | `https://fastapi.tiangolo.com/` | 🔴 ANOMALIE JS | **Mur JS critique** : `TypeError: Invalid base URL` sur `new URL(".", location)` + syntax error bundle minifié. |
| **03** | 2026-10-26 13:09 | `https://go.dev/doc/` | 🟢 SUCCÈS | Repli re-parse ADR-0025 maîtrisé (`why=UnmappedKey`), layout et rasterisation 14.6 Mo parfaits. |
| **04** | 2026-10-26 13:21 | `https://doc.rust-lang.org/nightly/std/` | 🟢 SUCCÈS | Documentation officielle Rust Std (Nightly) : rustdoc moderne, 0 erreur JS, 0 repli DOM. |
| **05** | 2026-10-26 13:30 | `https://man7.org/linux/man-pages/man1/ls.1.html` | 🟢 SUCCÈS PUR | Manuel Linux Man7 (troff/HTML pur) : rendu instantané, hiérarchie DOM impeccable, 0 anomalie. |

---

## 🔬 Fiches Techniques Détaillées d'Anomalies

### ✈️ Vol #02 — `https://fastapi.tiangolo.com/` (MkDocs Material)
- **Stack technique** : MkDocs Material, SVG inlinés, scripts de thème sombre/clair, bundling moderne.
- **Rendu visuel** : Page peinte avec succès (`captures/fastapi.png` — 1.66 Mo), mais les fonctionnalités interactives (onglets de code, changement de thème) sont bloquées suite à un crash précoce du script d'initialisation.

#### 💥 Mur Détecté (JavaScript Engine `explorer-jscore`) :
```text
TypeError: Invalid base URL
  at <inline>:1
  src: __md_scope=new URL(".", location), __md_scope.pathname.endsWith("/")...

ReferenceError: __md_get is not defined
  src: var palette=__md_get("__palette");

SyntaxError 5:23 : expected `;`
  url: https://fastapi.tiangolo.com/assets/javascripts/bundle.c04ba553.min.js
```

#### 🔍 Analyse Racine pour le Spécialiste JS :
1. **`new URL(".", location)`** : Le constructeur `URL` dans `explorer-jscore` attend soit une chaîne valide soit échoue si le second argument n'est pas converti correctement en chaîne via `location.href` ou `ToString(base)`.
2. **`SyntaxError: expected ';'`** : Le parser de `explorer-jscore` achoppe sur un idiome ES6 moderne dans le bundle minifié.

#### 🧪 Repro Minimal Test (Pour Agent 3) :
```javascript
const base = window.location;
const u = new URL(".", base);
assert(u.href.endsWith("/"));
```


### ✈️ Vol du 2026-10-26 13:34 — `https://docs.rs/`
- **Ressource & Rôle** : Documentation officielle des crates Rust
- **Statut d'atterrissage** : ⚠️ REPLI DOM (ADR-0025)
#### 💥 Anomalies détectées :
```text
[media post-nav : 0 offres]
[round 0 media=0]
[round 1 media=0]
2026-10-26T12:34:17.712511Z  WARN explorer_core::session::apply: journal DOM rejeté → repli re-parse (ADR-0025) tab=1 why=UnmappedKey
```


### ✈️ Vol du 2026-10-26 13:34 — `https://devdocs.io/`
- **Ressource & Rôle** : Agrégateur moderne de documentations de dev
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:36 — `https://curl.se/docs/manpage.html`
- **Ressource & Rôle** : Documentation de référence cURL
- **Statut d'atterrissage** : ⏳ TIMEOUT : requête ou exécution script bloquée au-delà de 25s
#### 💥 Anomalies détectées :
```text
Timeout jprobe après 25s
```


### ✈️ Vol du 2026-10-26 13:36 — `https://archlinux.org/`
- **Ressource & Rôle** : Portail et documentation de distribution
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:36 — `https://wiki.debian.org/`
- **Ressource & Rôle** : Wiki communautaire Debian
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:36 — `https://www.freebsd.org/docs/`
- **Ressource & Rôle** : Portail de documentation FreeBSD
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:36 — `https://nginx.org/en/docs/`
- **Ressource & Rôle** : Documentation officielle serveur NGINX
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:37 — `https://httpd.apache.org/docs/2.4/`
- **Ressource & Rôle** : Documentation Apache HTTP Server
- **Statut d'atterrissage** : 🟢 SUCCÈS
- **Constat technique** : Exécution nominale, 0 exception JavaScript non interceptée.


### ✈️ Vol du 2026-10-26 13:37 — `https://developer.apple.com/documentation/`
- **Ressource & Rôle** : Documentation développeur Apple
- **Statut d'atterrissage** : ⏳ TIMEOUT : requête ou exécution script bloquée au-delà de 25s
#### 💥 Anomalies détectées :
```text
Timeout jprobe après 25s
```
