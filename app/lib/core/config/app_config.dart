// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Configuration de l'app — toutes les valeurs « en dur » vivent ici
/// (AGENTS.md §4.13 : aucune valeur en dur ailleurs).
class AppConfig {
  const AppConfig({
    this.daemonBaseUrl = 'http://127.0.0.1:9377',
    this.apiKey = '',
    this.connectTimeout = const Duration(seconds: 5),
    this.requestTimeout = const Duration(seconds: 10),
    this.sseReconnectDelay = const Duration(seconds: 2),
    this.daemonRetryDelay = const Duration(seconds: 3),
    this.mediaReportInterval = const Duration(milliseconds: 250),
    this.frameRegionMaxRects = 16,
    this.frameRegionMaxArea = 0.5,
    this.scrollSyncEpsilon = 0.5,
    this.mediaCodecs = const [
      'video/mp4',
      'video/webm',
      'video/x-matroska',
      'video/avi',
      'video/mp2t',
      'audio/mpeg',
      'audio/mp4',
      'audio/aac',
      'audio/ogg',
      'audio/wav',
      'audio/webm',
      'application/vnd.apple.mpegurl',
    ],
    this.drmDelegateHosts = const [
      // Chaque entrée = un arbitrage (ADR-0027 §2 : périmètre
      // de confiance + exception d'égress ADR-0022 — le trafic
      // délégué ne traverse ni explorer-net ni le profil).
      'netflix.com', // VOD — Widevine L1
      'disneyplus.com', // VOD — Widevine
      'canalplus.com', // myCANAL (portail canal+)
      'mycanal.fr', // domaine historique myCANAL
      'primevideo.com', // Prime Video standalone
      'youtube.com', // YouTube — MSE hors portée du moteur
      // (ADR-0024) + DRM films/Premium : onglet délégué
      // entier (page + lecteur natifs WebView2).
      'youtu.be', // liens courts — la 301 part déjà en WebView2
      // amazon.tld volontairement ABSENT : déléguer le host
      // entier enverrait la boutique hors moteur — voir
      // DrmAllowlist (limite host-entier documentée).
    ],
    this.engineMode = const String.fromEnvironment(
      'EXPLORER_ENGINE',
      defaultValue: 'auto',
    ),
    this.engineLibPath = const String.fromEnvironment(
      'EXPLORER_ENGINE_LIB',
      defaultValue: '',
    ),
  });

  /// URL de base du daemon `explorer-daemon` (loopback — ADR-0003).
  final String daemonBaseUrl;

  /// Backend moteur : `auto` (FFI si la DLL est présente, REST
  /// sinon), `rest` (daemon seul), `ffi` (in-process seul —
  /// échoue si la bibliothèque ne charge pas). ADR-0018.
  final String engineMode;

  /// Chemin explicite vers `explorer_ffi.dll` — vide = noms
  /// standards dans le répertoire de l'exécutable / `PATH`.
  final String engineLibPath;

  /// Clé d'API optionnelle (`x-api-key` / `?key=` pour le SSE).
  final String apiKey;

  /// Timeout de connexion et de requête REST.
  final Duration connectTimeout;
  final Duration requestTimeout;

  /// Délai avant reconnexion du canal SSE `GET /api/events`.
  final Duration sseReconnectDelay;

  /// Cadence de re-bootstrap tant que le daemon est injoignable —
  /// `daemonOnline` repasse à `true` dès le premier `listTabs`
  /// réussi (étape 81 : récupération automatique, pas seulement
  /// au bouton « Réessayer »).
  final Duration daemonRetryDelay;

  /// Borne basse entre deux `POST …/media/{id}/state` par
  /// surface (contrat : battement ≤ 4 Hz, étape 71d).
  final Duration mediaReportInterval;

  /// Pull partiel (étape 109) : nombre maximal de `rects`
  /// cumulées avant de basculer en pull plein — au-delà,
  /// N requêtes + N composites coûtent plus qu'une frame.
  final int frameRegionMaxRects;

  /// Pull partiel (étape 109) : fraction de l'aire de frame
  /// couverte par les `rects` au-delà de laquelle le pull
  /// plein gagne (post-scroll, dirty = fenêtre entière).
  final double frameRegionMaxArea;

  /// Tolérance px CSS sur `scroll_y` moteur vs frame peinte
  /// (étape 109) : en dessous, l'invalidation est traitée
  /// comme non-scrollée (les `rects` restent composables).
  final double scrollSyncEpsilon;

  /// Types média déclarés dans `POST …/media/{id}/claim` —
  /// containers/codecs que le backend MDK (`fvp`) sait jouer ;
  /// `canPlayType` côté JS se reflète dessus.
  final List<String> mediaCodecs;

  /// Domaines délégués à la WebView2 (étape 136b, ADR-0027) —
  /// allowlist explicite et minimale ; matching host exact +
  /// sous-domaines (`DrmAllowlist`). Chaque entrée est aussi
  /// une exception d'égress : le trafic de ces domaines ne
  /// traverse pas `explorer-net`.
  final List<String> drmDelegateHosts;
}
