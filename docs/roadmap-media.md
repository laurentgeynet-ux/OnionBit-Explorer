<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Roadmap personnelle — agent « media »

**Périmètre :** piste NAVIGATEUR produit, chantier `app/`
uniquement — délégation DRM WebView2 (ADR-0027, étape 136
NAV). Aucun crate `explorer-*` touché, aucune couture
PIPELINE ; les besoins domaine passent par
`docs/interface-equipes.md`.

**Déclencheur tiré** : décision lead 2026-10-25 — l'ADR
était garée « usage réel » ; ouverture actée et loguée au
journal. Retranscription équipe : étape 136 de
`docs/roadmap-equipe-navigateur.md`.

**Mission** : un site VOD DRM (Netflix, myCANAL, Disney+,
Prime Video…) s'ouvre dans un **onglet délégué** WebView2
natif Windows — le chrome reste Explorer (onglets, omnibox,
tokens, historique), le moteur n'est jamais contacté pour
ces domaines. Toutes les autres pages restent sur le moteur.
Hors-Windows : repli honnête, le site n'est pas lu — jamais
de faux succès.

Légende : `[ ]` à faire · `[~]` en cours · `[x]` terminée.
Une sous-étape = un commit dédié (règle §5) ; gate Flutter
(`flutter analyze` + `flutter test`) avant chaque commit.

---

## Étape 136 — Onglet délégué WebView2 (ADR-0027)

- [ ] **136a — Choix plugin + licence** : évaluer
  `webview_windows` (pub.dev) vs accès direct WebView2 via
  `win32`+COM ; candidat multi-plateforme
  `flutter_inappwebview` (WebView2 sous Windows +
  WKWebView macOS/iOS + WebView Android dans une API
  unique — à peser si l'extension OS est envisageable, cf.
  § Extensions) ; critères : licence compatible
  MIT/Apache (vigilance ADR-0027 §Conséquences, même
  règle que `fvp` en ADR-0024), maintenance, embed
  Flutter Windows, détection du runtime Evergreen
  (`GetAvailableCoreWebView2BrowserVersionString`).
  Livrable : choix justifié dans l'ADR/journal + PoC
  minimal (surface WebView2 affichée dans une page de
  test de l'app, aucun wiring navigate).
- [ ] **136b — Allowlist DRM en config** (§4.13 — aucune
  valeur en dur) : `AppConfig.drmDelegateHosts`
  (`List<String>`, pattern `mediaCodecs` dans
  `app/lib/core/config/app_config.dart`) ; matcher host
  exact + sous-domaines (`netflix.com` couvre
  `www.netflix.com`). Liste seed minimale versionnée —
  chaque entrée = un arbitrage logué : `netflix.com`,
  `canalplus.com` / `mycanal.fr`, `disneyplus.com`,
  `primevideo.com` + `amazon.*` (chemins vidéo)…
- [ ] **136c — Point de décision de délégation** :
  interception unique dans
  `BrowserController.navigate`
  (`app/lib/features/browser/presentation/browser_controller.dart`)
  — **avant** REST (`BrowserRepository`) comme FFI
  (`FfiEngine`) : les deux canaux sont court-circuités
  (ADR-0003, ADR-0018). `BrowserTab` gagne `kind =
  engine | delegated`
  (`app/lib/features/browser/domain/browser_tab.dart`).
- [ ] **136d — Surface WebView2** : widget
  `DelegatedWebView` monté à la place de la frame moteur
  ; user-data-folder dédié (`<profil app>/webview2/` —
  données de session séparées du profil Explorer,
  ADR-0027 §5) ; cycle de vie création/navigate/dispose
  rattaché au cycle de vie de l'onglet.
- [ ] **136e — Resync navigation** : events WebView2 →
  `BrowserTab` (`SourceChanged`/`NavigationCompleted` →
  `url`, `DocumentTitle` → `title`,
  `CanGoBack`/`CanGoForward` → historique, état de
  chargement) ; `back`/`forward`/`reload` de l'app routés
  vers la WebView2 quand l'onglet est délégué.
- [ ] **136f — Indicateur chrome** : badge/icône
  « délégué » visible dans le chrome de l'onglet — la
  délégation est transparente à l'usage **et signalée**
  (ADR-0027 §3) ; omnibox, onglets, tokens inchangés.
- [ ] **136g — Repli hors-Windows** : allowlist matchée
  sans WebView2 (runtime absent ou autre OS) →
  navigation moteur normale, échec DRM honnête porté par
  le site (ADR-0027 §6) — aucun faux succès.
- [ ] **136h — Gate + doc** : tests unitaires matcher
  allowlist + modèle `BrowserTab.kind` ; `flutter
  analyze`/`flutter test` verts ; CHANGELOG ; case 136
  cochée ; statut ADR-0027 → « Acceptée — livrée ».

## Hors périmètre (murs nommés)

- **MSE/EME dans le moteur** — non rapprochés par cette
  étape (ADR-0024 §murs, ADR-0027 §séquencement) ;
  `MediaSurface` (étape 71) reste la voie du non-DRM.
- **Pont FFI Widevine** — voie écartée documentée
  (L3/480p + fragilité, ADR-0027 §contexte voie 2).
- **Égress/profil** — le trafic WebView2 ne traverse pas
  `explorer-net` : proxy, jar à cookies, politique
  ADR-0022 inapplicables aux domaines délégués —
  l'allowlist reste minimale pour cette raison autant que
  pour le périmètre de confiance.

## Extensions multi-plateforme (note — WebView2 = Windows only)

La délégation est un chemin **Windows** ; le modèle
d'onglet délégué (136c) est pensé comme seam neutre —
équivalents si le produit vise d'autres OS :

| OS | Surface | DRM réaliste |
|---|---|---|
| Android | `android.webkit.WebView` | Widevine via MediaDrm — L3 généralisé, L1 selon device |
| iOS/macOS | `WKWebView` | FairPlay — exige un certificat Apple FPS côté app |
| Linux | CEF (`flutter_cef`) / WebKitGTK | Widevine L3 au mieux ; OpenCDM — niche |

Chaque extension = entrée au journal + réévaluation
licence/certification propre à la plateforme. Hors
périmètre de l'étape 136 elle-même (Windows d'abord).
