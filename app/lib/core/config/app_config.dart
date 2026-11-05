// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Configuration de l'app — toutes les valeurs « en dur » vivent ici
/// (AGENTS.md §4.13 : aucune valeur en dur ailleurs).
/// Réglages `onionmedia_set_framedrop_auto` (P-63) — escalade
/// automatique de saut de trames quand le décodage est moins
/// rapide que le temps réel (le vrai levier « mur 4K » —
/// `set_max_pixels` ne borne que la géométrie des
/// représentations, pas le débit décodé).
class MediaFramedropConfig {
  const MediaFramedropConfig({
    this.window = const Duration(seconds: 2),
    this.dwell = const Duration(seconds: 1),
    this.pin = const Duration(seconds: 10),
    this.dropsPerWindow = 4,
    this.maxLevel = 3,
  });

  /// Fenêtre glissante de comptage des drops (`window_ms`).
  final Duration window;

  /// Délai entre deux montées de palier (`dwell_ms`).
  final Duration dwell;

  /// Pin après un `set_framedrop` explicite (`pin_ms`).
  final Duration pin;

  /// Drops dans `window` déclenchant un palier
  /// (`drops_per_window`).
  final int dropsPerWindow;

  /// Palier max auto — 3 = `NonKey` (jamais d'écran noir
  /// auto ; la doc dépôt ramène 4/`All` à `NonKey`).
  final int maxLevel;
}

class AppConfig {
  const AppConfig({
    this.daemonBaseUrl = 'http://127.0.0.1:9377',
    this.apiKey = '',
    this.connectTimeout = const Duration(seconds: 5),
    this.requestTimeout = const Duration(seconds: 10),
    this.sseReconnectDelay = const Duration(seconds: 2),
    this.daemonRetryDelay = const Duration(seconds: 3),
    this.mediaReportInterval = const Duration(milliseconds: 250),
    this.mediaControlsHideDelay = const Duration(seconds: 3),
    this.mediaControlsFade = const Duration(milliseconds: 160),
    this.mediaSeekStep = const Duration(seconds: 5),
    this.mediaVolumeStep = 0.05,
    this.frameRegionMaxRects = 16,
    this.frameRegionMaxArea = 0.5,
    this.scrollSyncEpsilon = 0.5,
    this.mediaCodecs = const [
      'video/mp4',
      'audio/mp4',
      'audio/x-m4a',
      'video/x-m4v',
      'video/quicktime',
      'video/3gpp',
      'video/3gpp2',
      'audio/3gpp',
      'audio/3gpp2',
      'video/webm',
      'audio/webm',
      'video/x-matroska',
      'audio/x-matroska',
      'video/mp2t',
      'video/mpegts',
      'audio/ogg',
      'application/ogg',
      'audio/opus',
      'audio/mpeg',
      'audio/mp3',
      'audio/mpeg3',
      'audio/x-mpeg',
      'audio/x-mp3',
      'audio/aac',
      'audio/x-aac',
      'audio/aacp',
      'audio/adts',
      'audio/ac3',
      'audio/ac-3',
      'audio/wav',
      'audio/x-wav',
      'audio/wave',
      'audio/vnd.wave',
      'audio/flac',
      'audio/x-flac',
      'application/vnd.apple.mpegurl',
      'application/x-mpegURL',
      'application/dash+xml',
      // Préfixes codec (RFC 6381) — le claim les déclare au
      // moteur : `MediaSource.isTypeSupported` matche sur
      // cette forme (`codecs="avc1.42E01E"` → `avc1`) — les
      // entrées = familles décodables par les crates
      // `onioncodec-*` (capacités déclarées, pas promises).
      'avc1',
      'hvc1',
      'hev1',
      'vp09',
      'av01',
      'vp08',
      'vp8',
      'mp4a',
      'mp3',
      'opus',
      'vorbis',
      'flac',
      'ac-3',
      'ipcm',
    ],
    // Miroir du défaut `explorer-net` (`h1.rs`) — même UA sur
    // le HTML et le flux média pour les CDN qui sniffent.
    this.mediaUserAgent =
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) '
        'AppleWebKit/537.36 (KHTML, like Gecko) '
        'Explorer/0.2 Safari/537.36',
    this.mediaFramedrop = const MediaFramedropConfig(),
    this.drmDelegateHosts = const [
      // Chaque entrée = un arbitrage (ADR-0027 §2 : périmètre
      // de confiance + exception d'égress ADR-0022 — le trafic
      // délégué ne traverse ni explorer-net ni le profil).
      'netflix.com', // VOD — Widevine L1
      'disneyplus.com', // VOD — Widevine
      'canalplus.com', // myCANAL (portail canal+)
      'mycanal.fr', // domaine historique myCANAL
      'primevideo.com', // Prime Video standalone
      // ── Majors US ──
      'hulu.com', // Disney US
      'max.com', // HBO/Max
      'hbomax.com', // domaine historique HBO Max
      'paramountplus.com', // Paramount+
      'peacocktv.com', // NBCUniversal
      'appletv.com', // Apple TV+ (FairPlay)
      'tv.apple.com', // lecteur web Apple TV+
      'crunchyroll.com', // anime
      'discoveryplus.com', // standalone selon régions
      'amcplus.com', // AMC Networks
      'shudder.com', // AMC Networks (horreur)
      'starplus.com', // Disney LatAm
      'mubi.com', // VOD art & essai
      'pluto.tv', // FAST Paramount
      'tubitv.com', // FAST Fox
      'plex.tv', // FAST + media server
      'sling.com', // vMVPD Dish
      'fubo.tv', // vMVPD sport
      'philo.com', // vMVPD
      // ── France / francophonie ──
      'tf1plus.fr', // TF1+
      'france.tv', // France Télévisions
      '6play.fr', // M6+ (domaine historique)
      'm6.fr', // M6+ (nouveau portail)
      'arte.tv', // ARTE (contenus DRM partiels)
      'rmcbfmplay.com', // RMC BFM Play
      'molotov.tv', // TV de rattrapage
      'rakuten.tv', // VOD Rakuten
      'orangecine.fr', // OCS/Orange Ciné
      'tv.sfr.fr', // SFR TV
      // ── Sport ──
      'dazn.com', // DAZN — Widevine strict
      'espn.com', // ESPN+/Watch
      'beinsports.com', // beIN Connect (connect.*)
      // ── Europe ──
      'skyshowtime.com', // Sky/Paramount JV
      'sky.com', // Sky UK
      'nowtv.com', // NOW UK
      'nowtv.it', // NOW Italie
      'itv.com', // ITVX (portal inclus /watch)
      'itvx.com', // ITVX dédié
      'channel4.com', // Channel 4 UK
      'bbc.co.uk', // BBC iPlayer (/iplayer)
      'joyn.de', // Joyn DE
      'rtlplus.de', // RTL+ DE
      'zdf.de', // ZDFmediathek
      'ardmediathek.de', // ARD Mediathek
      'rtve.es', // RTVE Play ES
      'atresplayer.com', // Atresplayer ES
      'raiplay.it', // RaiPlay IT
      'viaplay.com', // Viaplay Nordiques
      // ── Asie / autres ──
      'hotstar.com', // Disney+ Hotstar IN
      'iq.com', // iQIYI
      'wetv.vip', // WeTV Tencent
      'viu.com', // Viu
      'viki.com', // Rakuten Viki
      'twitch.tv', // pas de DRM mais MSE massif —
      // même raison que YouTube : le lecteur JS
      // dépasse le moteur, onglet délégué
      // amazon.tld volontairement ABSENT : déléguer le host
      // entier enverrait la boutique hors moteur — voir
      // DrmAllowlist (limite host-entier documentée).
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

  /// Contrôles média natifs (étape 183) — délai avant
  /// auto-masquage de la barre pendant la lecture, durée du
  /// fondu, pas du seek clavier ←/→ et du volume ↑/↓.
  final Duration mediaControlsHideDelay;
  final Duration mediaControlsFade;
  final Duration mediaSeekStep;
  final double mediaVolumeStep;

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
  /// containers/codecs que le backend OnionMedia sait jouer ;
  /// `canPlayType` côté JS se reflète dessus.
  final List<String> mediaCodecs;

  /// `User-Agent` du lecteur média natif sur les ouvertures
  /// progressives (`onionmedia_set_http_headers`, P-66) —
  /// **miroir du défaut `explorer-net`** (`h1.rs` : les
  /// anti-bot/CDN qui UA-sniffent doivent voir le même UA
  /// sur le HTML et sur le flux). À resynchroniser si le
  /// moteur change le sien.
  final String mediaUserAgent;

  /// Politique `set_framedrop_auto` appliquée à chaque
  /// lecteur natif — `null` = pas d'escalade auto.
  final MediaFramedropConfig mediaFramedrop;

  /// Domaines délégués à la WebView2 (étape 136b, ADR-0027) —
  /// allowlist explicite et minimale ; matching host exact +
  /// sous-domaines (`DrmAllowlist`). Chaque entrée est aussi
  /// une exception d'égress : le trafic de ces domaines ne
  /// traverse pas `explorer-net`.
  final List<String> drmDelegateHosts;
}
