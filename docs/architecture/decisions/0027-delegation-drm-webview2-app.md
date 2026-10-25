# ADR-0027 — Sites DRM : délégation à une WebView2 au niveau `app/`

Statut : **Acceptée — livrée (étape 136a–h,
2026-10-25)** — stratégie actée, implémentée `app/`
(plugin nommé § Amendement).
Complète ADR-0024 (lecture média : responsabilité
d'hôte) qui nommait le DRM « hors portée
structurellement » : celle-ci fixe *quoi faire quand
même* — pour que la réponse ne soit pas redérivée à
chaque échec sur plateforme de streaming.

## Contexte

ADR-0024 a livré la seam `MediaSurface` : tout le web
non-DRM passe par l'hôte (player natif, overlay
au-dessus du rect peint par le moteur). Le mur restant
est nommé depuis cette ADR : les plateformes DRM
(Netflix, myCANAL, Disney+, Prime Video…) exigent un
CDM propriétaire (Widevine, PlayReady, FairPlay), une
signature VMP du binaire, un whitelisting contractuel
de la plateforme et une pile EME+MSE complète.
Constat vérifié et partagé par Servo/Ladybird :
aucun moteur indépendant n'y accède en natif, quelle
que soit la qualité du code — c'est un verrou
juridique et de certification, pas technique.

Trois voies pesées (journal `interface-equipes.md`,
note lead du 2026-10-22 — la présente ADR acte la
décision y consignée « garée ») :

1. **Délégation `app/` via WebView2** (retenue) —
   runtime préinstallé Windows, Widevine L1 +
   PlayReady matériels → 1080p/4K/HDR immédiat ;
   zéro changement au moteur.
2. **Pont C-API Widevine en FFI** — la C-API
   publique de Widevine plafonne à L3 (480p,
   décodage logiciel) ; exige du natif dans ou
   autour du workspace (règles §4.1 `unsafe`, §4.2
   pur Rust), une pile EME+MSE à écrire, des mois
   de travail et une fragilité permanente (blob
   fermé versionné par Google). Écartée.
3. **Exclusion assumée** — les sites DRM restent
   non lus, message honnête. Conservée comme
   **repli par construction** : c'est le
   comportement hors-Windows et celui d'une
   allowlist vide.

## Décision

**Un site DRM est un onglet délégué à la plateforme,
pas une page du moteur.**

1. **Couture unique = `app/`** : quand l'URL d'un
   onglet matche l'allowlist, l'app monte une
   WebView2 native (Windows) à la place de la
   frame moteur et court-circuite `navigate`
   (REST et FFI — ADR-0003, ADR-0018). Le moteur
   n'est jamais contacté pour ces domaines.
2. **Allowlist explicite, minimale, configurée** :
   liste de domaines en configuration (règle
   §4.13 — aucune valeur en dur), pas de
   détection heuristique — un échec `play()` ou
   une erreur EME n'est pas un signal fiable.
   Chaque entrée est un arbitrage logué.
3. **Chrome Explorer conservé** : onglets,
   omnibox, tokens, historique identiques ; les
   événements de navigation WebView2
   (`NavigationStarted`, `SourceChanged`,
   `DocumentTitle`, pile arrière/avant)
   resynchronisent `BrowserTab` — la délégation
   est transparente à l'usage et signalée par un
   indicateur dans le chrome.
4. **Exception d'égress nommée** : le trafic
   WebView2 ne traverse pas `explorer-net` —
   politique ADR-0022 (anti-fuites), mandataires
   `http_proxy`/`no_proxy`, jar à cookies et
   profil ne s'appliquent pas aux domaines
   délégués. L'allowlist est aussi une liste
   d'exceptions réseau : minimale pour cette
   raison autant que pour le périmètre de
   confiance.
5. **Confiance déléguée à Microsoft** : la page
   vit dans les processus WebView2 (Chromium
   sandboxé) — modèle de menace différent du
   nôtre (ADR-0011, ADR-0026), ni meilleur ni
   pire : autre. Les données de session de ces
   sites vivent dans le user-data-folder
   WebView2, séparé du profil Explorer.
6. **Repli honnête hors-Windows** : pas de
   WebView2 → voie 3 par défaut — le document se
   charge normalement par le moteur, le DRM
   échoue, le message de la plateforme parle.
   Jamais de faux succès (même règle
   qu'ADR-0023).
7. **Zéro dépendance au workspace Rust** : le
   plugin Flutter retenu (`webview_windows` ou
   équivalent, code natif côté app) est justifié
   par cette ADR ; `#![forbid(unsafe_code)]`,
   ADR-0001 et le sens de dépendance restent
   intacts — rien n'entre dans `crates/`.

## Séquencement (déclencheur)

- **Garée par condition** : la dépense (plugin
  natif + modèle d'onglet hybride + resync de
  navigation) ne se justifie qu'à usage réel — un
  produit dont personne ne visite les plateformes
  concernées n'a pas besoin du chemin.
  Déclencheur : premier usage réel documenté
  confronté à un domaine DRM, ou jalonnement
  d'une release grand public.
- **Ouverture = étape NAVIGATEUR** (piste
  produit) — chantier `app/` pur, aucune couture
  PIPELINE.
- **Les murs restent nommés** : ni MSE ni EME
  dans le moteur — cette ADR ne les rapproche
  pas ; ADR-0024 reste la seule couture média du
  domaine.

## Justification

- **Le verrou est contractuel** : payer le
  moindre coût moteur pour une classe de sites
  inaccessible par construction serait du
  sur-dimensionnement — même argument
  qu'ADR-0024 (couverture interne bornée et
  coûteuse).
- **La seam est déjà « hôte »** : l'app composite
  déjà un lecteur natif au-dessus de la frame
  (étape 71, `MediaSurface`) ; remplacer la
  frame entière par une surface native est le
  même mouvement, un cran plus large.
- **La mission reste intacte** : le moteur
  from-scratch ne lit pas ces sites — par
  délégation revendiquée et bornée, pas par
  import. ADR-0001 porte sur le pipeline ;
  rien ici n'importe de moteur dans le
  workspace.
- **La voie 2 (FFI Widevine) reste documentée** —
  L3/480p + mois + fragilité : si WebView2
  devenait indisponible, le repli est
  l'exclusion assumée (voie 3), pas le pont FFI.

## Conséquences

- `BrowserTab` côté app gagnera une notion
  d'onglet délégué (miroir d'état, source de
  navigation WebView2 au lieu du daemon) —
  extension de modèle `app/` uniquement, hors
  périmètre des crates.
- L'allowlist devient un fichier de configuration
  à part entière — chaque ajout = arbitrage
  explicite (périmètre de confiance + exception
  egress).
- Hors-Windows, les sites DRM restent non lus —
  l'écart est nommé, pas découvert au terrain.
- À l'implémentation : vérifier la licence du
  plugin retenu comme pour `fvp` (ADR-0024
  §Conséquences — vigilance licence).
- Aucun code modifié par cette ADR ;
  `forbid(unsafe_code)` et la règle pur Rust
  inchangés.

## Amendement 2026-10-25 — plugin retenu (136a)

Contrainte technique découverte à l'évaluation :
**les flux DRM ne sont pas capturables**. Une vidéo
protégée (Widevine L1, PlayReady matériel) sort par
des surfaces protégées que les API de capture
(`Windows.Graphics.Capture`) restituent en noir — un
plugin qui rend la webview « offscreen → texture
Flutter » afficherait du noir exactement sur le
contenu visé. Seul un **HWND natif** composé par le
système rend le contenu protégé.

Matrice pesée :

| Plugin | Licence | Rendu | DRM | Verdict |
|---|---|---|---|---|
| `webview_windows` 0.4.0 | BSD-3 | offscreen WGC → texture | noir (surfaces protégées) | écarté |
| `flutter_inappwebview` 6.1.5 | Apache-2 | embed, API lourde fédérée | incertain | écarté (poids) |
| `desktop_webview_window` | — | fenêtre top-level séparée | rendu OK | écarté (casse le chrome) |
| **`webview_win_floating` 3.0.3** | **BSD-3** | **HWND natif overlay** | **rendu OK** | **retenu** |

`webview_win_floating` pose la WebView2 en HWND
au-dessus de la fenêtre Flutter (pas de capture) :
le contenu protégé s'affiche, la vidéo est fluide,
`userDataFolder`/`profileName` configurables
(sessions séparées du profil Explorer, §5),
délégués de navigation complets (pageStarted /
pageFinished / urlChange / titre / historique).
Limites assumées et documentées par l'upstream :
aucun widget Flutter ne peut se composer **au-
dessus** de la zone webview ni route poussée
par-dessus — le chrome Explorer vit hors de cette
zone, `setVisibility` couvre l'occlusion (onglet
inactif, panneaux). Mode standalone (`WinWebView*`)
retenu — pas de dépendance `webview_flutter`.
Équivalent Linux proposé par le plugin
(webkit2gtk) hors périmètre — L3 seulement
(§Extensions multi-OS inchangé).

PoC : `app/lib/poc_delegated.dart` (cible `main`
séparée, aucun wiring) — `flutter build windows`
vert, nuget/WebView2 SDK auto-téléchargés.
