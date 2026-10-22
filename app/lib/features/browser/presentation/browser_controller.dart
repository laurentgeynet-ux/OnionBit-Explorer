// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/api/api_client.dart';
import '../../../core/api/events_client.dart';
import '../../../core/api/ffi_engine.dart';
import '../../../core/config/app_config.dart';
import '../../../core/di/providers.dart';
import '../../../core/platform/env.dart';
import '../../../core/settings/app_settings.dart';
import '../data/browser_repository.dart';
import '../data/media_player.dart';
import '../domain/bookmark.dart';
import '../domain/browser_tab.dart';
import '../domain/download_offer.dart';
import '../domain/history_entry.dart';
import '../domain/media_offer.dart';

/// État UI du navigateur : onglets, onglet actif, disponibilité
/// du daemon, dernière erreur métier.
class BrowserState {
  const BrowserState({
    this.tabs = const [],
    this.activeId,
    this.daemonOnline = false,
    this.lastError,
    this.frameTick = 0,
    this.hoverUrl,
    this.hoverEditable = false,
    this.fieldFocused = false,
    this.pageErrors = const {},
    this.downloadOffer,
    this.media = const {},
    this.scrollY = const {},
    this.resubmit,
  });

  final List<BrowserTab> tabs;
  final String? activeId;

  /// `false` tant que le daemon n'a pas répondu — l'UI affiche
  /// l'état dégradé (omnibox désactivée, bouton réessayer).
  final bool daemonOnline;

  /// Dernier message d'erreur métier (`pipeline_error`, échec
  /// d'action) — affiché en snackbar/bandeau.
  final String? lastError;

  /// Compteur incrémenté à chaque `frame_invalidated` de
  /// l'onglet actif — l'étape 30 s'en sert pour retirer le
  /// PNG (ici il rend simplement le rebuild observable).
  final int frameTick;

  /// `href` sous le curseur (résultat `hover` du moteur) —
  /// pilote le curseur lien + la barre de statut.
  final String? hoverUrl;

  /// Le point survolé est un champ éditable → curseur texte.
  final bool hoverEditable;

  /// Un champ de formulaire est focalisé côté moteur — les
  /// frappes clavier sont relayées en `input key`.
  final bool fieldFocused;

  /// Erreur de chargement par onglet (`pipeline_error` SSE ou
  /// `navigate` en échec) — affichée en page d'erreur stylée
  /// à la place de la frame (étape 33). Effacée au prochain
  /// `navigated` de l'onglet.
  final Map<String, String> pageErrors;

  /// Dernier téléchargement proposé en attente de décision
  /// (étape 70, SSE `download_offered`) — l'UI affiche la
  /// boîte « Enregistrer / Ouvrir comme texte ». `null`
  /// dès qu'une décision est prise.
  final DownloadOffer? downloadOffer;

  /// Surfaces média live par onglet (seam `MediaSurface`,
  /// étape 71) : `{tabId: {mediaId: offer}}` — alimenté par
  /// `media_offered`/`moved`/`gone` + `GET …/media` au
  /// bootstrap. `rect` en px CSS document.
  final Map<String, Map<int, MediaOffer>> media;

  /// Scroll vertical courant par onglet, porté par
  /// `frame_invalidated.scroll_y` (champ additif 71d) —
  /// l'overlay média positionne à `rect.y − scroll_y`.
  /// `scroll_x` n'existe pas : le moteur est vertical-seul.
  final Map<String, double> scrollY;

  /// Resoumission en attente d'accord (étape 93) : le reload
  /// a répondu `resubmit_confirm` — l'UI affiche le dialogue
  /// « renvoyer le formulaire ? », `reloadConfirmed` rejoue
  /// avec `{"confirm":true}`. `null` dès décision prise.
  final ({String tabId, String method, String url})? resubmit;

  /// Onglet actif, ou `null`.
  BrowserTab? get active {
    for (final t in tabs) {
      if (t.id == activeId) return t;
    }
    return tabs.isEmpty ? null : tabs.first;
  }

  BrowserState copyWith({
    List<BrowserTab>? tabs,
    String? activeId,
    bool? daemonOnline,
    String? Function()? lastError,
    int? frameTick,
    String? Function()? hoverUrl,
    bool? hoverEditable,
    bool? fieldFocused,
    Map<String, String>? pageErrors,
    DownloadOffer? Function()? downloadOffer,
    Map<String, Map<int, MediaOffer>>? media,
    Map<String, double>? scrollY,
    ({String tabId, String method, String url})? Function()? resubmit,
  }) => BrowserState(
    tabs: tabs ?? this.tabs,
    activeId: activeId ?? this.activeId,
    daemonOnline: daemonOnline ?? this.daemonOnline,
    lastError: lastError != null ? lastError() : this.lastError,
    frameTick: frameTick ?? this.frameTick,
    hoverUrl: hoverUrl != null ? hoverUrl() : this.hoverUrl,
    hoverEditable: hoverEditable ?? this.hoverEditable,
    fieldFocused: fieldFocused ?? this.fieldFocused,
    pageErrors: pageErrors ?? this.pageErrors,
    downloadOffer: downloadOffer != null ? downloadOffer() : this.downloadOffer,
    media: media ?? this.media,
    scrollY: scrollY ?? this.scrollY,
    resubmit: resubmit != null ? resubmit() : this.resubmit,
  );
}

/// Fabrique de lecteurs natifs pour la seam `MediaSurface` —
/// `VideoMediaPlayer` (MDK/`fvp`) en prod, fake en test.
final mediaPlayerFactoryProvider = Provider<MediaPlayerFactory>(
  (ref) => VideoMediaPlayer.create,
);

/// Dépôt injectable (tests : `Provider.overrideWith`).
/// `auto` : `FfiEngine` (moteur in-process, ADR-0018) si la
/// bibliothèque dynamique charge, sinon le repo REST daemon.
/// Le réglage `AppSettings.engine` l'emporte sur l'env
/// (`effectiveEngineModeProvider`) ; `profileDir` est exporté
/// vers `EXPLORER_PROFILE` **avant** le chargement FFI —
/// changement de mode = re-création du provider (l'ancien
/// moteur est disposé via `onDispose`).
final browserRepositoryProvider = Provider<BrowserRepository>((ref) {
  final cfg = ref.watch(configProvider);
  final mode = ref.watch(effectiveEngineModeProvider);
  // `.select` : la re-création du moteur n'a lieu que si
  // `profileDir` (ou `engine` en amont) change — un changement
  // de thème ne doit pas rebâtir le dépôt.
  final profile = ref.watch(settingsProvider.select((s) => s.profileDir));
  if (profile.isNotEmpty) setProcessEnv('EXPLORER_PROFILE', profile);
  final ffi = mode == 'rest' ? null : FfiEngine.tryLoad(cfg);
  ref.onDispose(() => ffi?.dispose());
  return ffi ?? BrowserRepository(ref.watch(apiClientProvider));
});

/// Signets du store moteur — `AsyncNotifier` rechargé après
/// chaque mutation (`toggle`/`remove`). En mode FFI la liste
/// reste vide (dégradation documentée, ADR-0018).
class BookmarksController extends AsyncNotifier<List<Bookmark>> {
  @override
  Future<List<Bookmark>> build() =>
      ref.watch(browserRepositoryProvider).bookmarks();

  /// Ajoute le signet s'il est absent, le retire sinon —
  /// `true` si l'URL est signetée après l'appel.
  Future<bool> toggle(String url, {String title = ''}) async {
    if (url.isEmpty || url == 'about:blank') return false;
    final repo = ref.read(browserRepositoryProvider);
    final list = state.value ?? const <Bookmark>[];
    final marked = list.any((b) => b.url == url);
    try {
      if (marked) {
        await repo.removeBookmark(url);
      } else {
        await repo.addBookmark(url, title: title);
      }
    } on Object {
      return marked;
    }
    state = AsyncData(await repo.bookmarks());
    return !marked;
  }

  /// Retire un signet du store puis recharge la liste.
  Future<void> remove(String url) async {
    final repo = ref.read(browserRepositoryProvider);
    try {
      await repo.removeBookmark(url);
    } on Object {
      // Daemon injoignable → la liste visible reste telle quelle.
      return;
    }
    state = AsyncData(await repo.bookmarks());
  }
}

final bookmarksProvider =
    AsyncNotifierProvider<BookmarksController, List<Bookmark>>(
      BookmarksController.new,
    );

/// Historique de navigation du store moteur — rechargé à
/// chaque ouverture du panneau (`autoDispose`).
final historyProvider = FutureProvider.autoDispose<List<HistoryEntry>>(
  (ref) => ref.watch(browserRepositoryProvider).history(),
);

/// Facteur de zoom d'affichage (étape 57) — implémenté
/// côté UI via le viewport moteur : `setViewport(w/z, h/z,
/// z×dpr)` refond le layout à `w/z` unités CSS puis rasterise
/// net à `w×dpr` px physiques (ADR-0021 — aucune API moteur).
/// Borné pour rester dans les budgets de frame.
class ZoomController extends Notifier<double> {
  static const min = 0.25;
  static const max = 4.0;
  static const step = 1.25;

  @override
  double build() => 1.0;

  void zoomIn() => state = (state * step).clamp(min, max);
  void zoomOut() => state = (state / step).clamp(min, max);
  void reset() => state = 1.0;
}

final zoomProvider = NotifierProvider<ZoomController, double>(
  ZoomController.new,
);

/// État de la barre find-in-page (étape 57bis) — visibilité,
/// requête et position courante. Le surlignage est peint
/// côté moteur ; l'app ne garde que compte + indice.
class FindBarState {
  const FindBarState({
    this.visible = false,
    this.query = '',
    this.index = 0,
    this.count = 0,
    this.busy = false,
  });

  final bool visible;
  final String query;
  final int index;
  final int count;
  final bool busy;

  FindBarState copyWith({
    bool? visible,
    String? query,
    int? index,
    int? count,
    bool? busy,
  }) => FindBarState(
    visible: visible ?? this.visible,
    query: query ?? this.query,
    index: index ?? this.index,
    count: count ?? this.count,
    busy: busy ?? this.busy,
  );
}

/// Pilote `POST /api/tabs/{id}/find` — nouveau terme
/// (`dir 0`), occurrence suivante/précédente (±1), fermeture
/// (requête vide → le moteur efface le surlignage).
class FindController extends Notifier<FindBarState> {
  @override
  FindBarState build() => const FindBarState();

  String? get _tabId => ref.read(browserProvider).activeId;

  void open() => state = state.copyWith(visible: true);

  Future<void> close() async {
    if (state.query.isNotEmpty) {
      final id = _tabId;
      if (id != null) {
        try {
          await ref.read(browserRepositoryProvider).find(id, '');
        } on Object {
          // fermeture locale tolérée — l'overlay se peint
          // encore côté moteur jusqu'à la prochaine requête.
        }
      }
    }
    state = const FindBarState();
  }

  Future<void> submit(String query) async {
    final id = _tabId;
    if (id == null || state.busy) return;
    state = state.copyWith(query: query, busy: true);
    try {
      final r = await ref
          .read(browserRepositoryProvider)
          .find(id, query, dir: 0);
      state = state.copyWith(
        index: r.index,
        count: r.count,
        busy: false,
        query: r.query.isEmpty ? query : r.query,
      );
    } on Object catch (e) {
      state = state.copyWith(busy: false);
      ref.read(browserProvider.notifier).setError('$e');
    }
  }

  Future<void> step(int dir) async {
    final id = _tabId;
    if (id == null || state.busy || state.count == 0) return;
    state = state.copyWith(busy: true);
    try {
      final r = await ref
          .read(browserRepositoryProvider)
          .find(id, state.query, dir: dir);
      state = state.copyWith(index: r.index, count: r.count, busy: false);
    } on Object catch (e) {
      state = state.copyWith(busy: false);
      ref.read(browserProvider.notifier).setError('$e');
    }
  }
}

final findProvider = NotifierProvider<FindController, FindBarState>(
  FindController.new,
);

/// Contrôleur navigateur — pilote les actions REST et applique
/// les événements SSE au `BrowserState` (ADR-0003/0004).
class BrowserController extends Notifier<BrowserState> {
  StreamSubscription<ServerEvent>? _sub;

  /// Timer de sondage des animations en mode FFI — remplace
  /// le SSE `frame_invalidated` absent du canal (ADR-0018).
  Timer? _animTimer;

  /// Re-bootstrap borné tant que le daemon est injoignable
  /// (étape 81) — annulé au premier succès. Sans lui,
  /// `daemonOnline:false` n'était relâché que par le bouton
  /// « Réessayer » : le SSE reconnectait en silence pendant
  /// que la bannière affirmait « daemon absent ».
  Timer? _daemonRetry;

  /// Callback brightness OS pour `theme: 'system'` (étape 97)
  /// — désenregistré au dispose.
  VoidCallback? _brightnessListener;

  /// Dépôt capturé dans `build` — `ref` est invalide après un
  /// `await`, les lectures post-build passent par ce champ.
  /// Pas `final` : `build()` se ré-exécute quand
  /// `browserRepositoryProvider` est recréé (changement de mode
  /// moteur / `profileDir`) — un `late final` jetterait
  /// `LateInitializationError` à la réassignation.
  late BrowserRepository _repo;

  /// Config capturée dans `build` (cadence de re-bootstrap).
  late AppConfig _cfg;

  /// Fabrique de lecteurs natifs — injectable en test.
  late MediaPlayerFactory _playerFactory;

  /// Lecteurs natifs par surface claimée : `{tab: {media: p}}`
  /// — hors `BrowserState` (objets à ressources, pas de l'état).
  final Map<String, Map<int, MediaPlayer>> _players = {};

  /// Frames décodées par onglet — **hors `BrowserState`
  /// volontairement** (terrain 22/10) : les `copyWith` donnent
  /// naissance à des états obsolètes qui restent épinglés par
  /// l'arbre de widgets (DebugCreator, listeners d'animations)
  /// — une frame de ~20-250 Mo par état retenu, ~10 Go en
  /// quelques minutes. La map est mutée en place ; `frameTick`
  /// dans l'état pilote les rebuilds d'affichage.
  final Map<String, ui.Image> _frameImages = {};

  /// `scroll_y` (px CSS) **de la frame actuellement affichée**
  /// par onglet — posé à l'installation de l'image. Sert à la
  /// traduction locale (étape 109) : entre le scroll et le
  /// pull suivant, l'écart `scrollY − peint` se rend en
  /// translatant la frame retenue — retour visuel immédiat,
  /// converge à la frame livrée.
  final Map<String, double> _paintedScrollY = {};

  /// Plan de pull par onglet (étape 109) : `rects` dirty
  /// accumulées (px physiques) → pull partiel + composite ;
  /// `full` → pull plein (scroll, `rects` vides/absentes,
  /// volume borné dépassé — la région post-scroll ne se
  /// compose pas sur une frame pré-scroll).
  final Map<String, List<ui.Rect>> _pendingRects = {};
  final Set<String> _pendingFull = {};

  /// Frame décodée courante d'un onglet (`null` jamais rendue)
  /// — affichée par `RawImage` dans le widget frame.
  ui.Image? frameImage(String tabId) => _frameImages[tabId];

  /// Scroll demandé mais pas encore peint, en px CSS —
  /// `scrollY moteur − scroll_y de la frame affichée`. Le
  /// `_FrameView` translate la frame retenue de `-shift` px
  /// logiques·zoom (étape 109) : la molette répond sans
  /// attendre l'aller-retour moteur ; le pull livré remet
  /// l'écart à zéro.
  double frameShiftCss(String tabId) {
    final painted = _paintedScrollY[tabId];
    if (painted == null || _frameImages[tabId] == null) return 0;
    return (state.scrollY[tabId] ?? painted) - painted;
  }

  /// Écouteurs posés sur chaque lecteur — clés `tab/media`
  /// pour `removeListener` au dispose.
  final Map<String, void Function()> _playerListeners = {};

  /// Horodatage + dernier `state` POSTés par surface —
  /// borne « ≤ 4 Hz » du contrat (`_cfg.mediaReportInterval`).
  final Map<String, DateTime> _reportTimes = {};
  final Map<String, String> _reportStates = {};

  static String _mkey(String tab, int media) => '$tab/$media';

  @override
  BrowserState build() {
    _repo = ref.watch(browserRepositoryProvider);
    _cfg = ref.read(configProvider);
    _playerFactory = ref.read(mediaPlayerFactoryProvider);
    ref.onDispose(() => _daemonRetry?.cancel());
    ref.onDispose(_releaseAllPlayers);
    // Étape 97 — `AppSettings.theme` propagé au moteur :
    // `dark`/`light` directs, `system` suit la brightness OS
    // (changements inclus via `onPlatformBrightnessChanged`).
    ref.listen(settingsProvider.select((s) => s.theme), (_, _) {
      unawaited(_pushColorScheme());
    });
    // `PlatformDispatcher.instance` (dart:ui) est accessible
    // sans `WidgetsBinding` — le notifier vit aussi dans les
    // tests unitaires purs (ProviderContainer seul).
    final dispatcher = ui.PlatformDispatcher.instance;
    _brightnessListener = () {
      if (ref.read(settingsProvider).theme == 'system') {
        unawaited(_pushColorScheme());
      }
    };
    dispatcher.onPlatformBrightnessChanged = _brightnessListener;
    ref.onDispose(() {
      if (dispatcher.onPlatformBrightnessChanged == _brightnessListener) {
        dispatcher.onPlatformBrightnessChanged = null;
      }
    });
    if (_repo is FfiEngine) {
      // Pas de SSE in-process : la sondage d'animations porte
      // l'invalidation ; les actions tirent leur frame au
      // retour (navigate/input).
      _animTimer?.cancel(); // rebuild sans dispose (repo recréé)
      _animTimer = Timer.periodic(
        const Duration(milliseconds: 50),
        (_) => unawaited(_tickFfi()),
      );
      ref.onDispose(() => _animTimer?.cancel());
    } else {
      final events = ref.watch(eventsClientProvider);
      _sub?.cancel();
      _sub = events.events().listen(_onEvent);
      ref.onDispose(() => _sub?.cancel());
    }
    // Microtask : le bootstrap touche `state` après le retour
    // de `build` (un dépôt qui jette synchronement ne doit
    // pas lire un provider non initialisé).
    unawaited(Future.microtask(_bootstrap));
    return const BrowserState();
  }

  /// Sondage `explorer_tick` — avance transitions/animations
  /// et retire la frame de l'onglet actif si invalidée.
  /// Asynchrone (étape 107 : le tick tourne sur l'isolate
  /// worker FFI — la boucle 50 ms ne bloque plus l'UI).
  /// Ét.109 : le tick rapporte `scroll_y` + dirty-rects —
  /// même plan de pull que le SSE `frame_invalidated`.
  Future<void> _tickFfi() async {
    final engine = _repo;
    final id = state.activeId;
    if (engine is! FfiEngine || id == null) return;
    try {
      final inv = await engine.tick(id);
      if (!ref.mounted) return;
      final y = inv.scrollY;
      if (y != null && state.scrollY[id] != y) {
        state = state.copyWith(scrollY: {...state.scrollY, id: y});
      }
      if (inv.invalidated || (inv.rects?.isNotEmpty ?? false)) {
        _planInvalidation(id, inv.rects, y);
        unawaited(_pullFrame(id));
      }
    } on Object {
      // Moteur in-process indisponible → prochain tick.
    }
  }

  /// Premier contact : liste les onglets existants ou en crée
  /// un — prouve aussi que le daemon répond.
  Future<void> _bootstrap() async {
    try {
      var tabs = await _repo.listTabs();
      if (tabs.isEmpty) {
        tabs = [await _repo.createTab()];
      }
      if (!ref.mounted) return;
      _daemonRetry?.cancel();
      _daemonRetry = null;
      state = state.copyWith(
        tabs: tabs,
        activeId: state.activeId ?? tabs.first.id,
        daemonOnline: true,
        // « daemon injoignable » résolu par la reconnexion —
        // l'erreur persistante est purgée (étape 88).
        lastError: () => null,
      );
      // Réglage courant poussé au moteur neuf (étape 97 —
      // l'env session démarre `light` sinon).
      unawaited(_pushColorScheme());
      // Réconciliation à froid : les offres émises avant la
      // reconnexion SSE sont récupérées par `GET …/media`.
      for (final t in tabs) {
        unawaited(_syncMedia(t.id));
      }
    } on Object {
      // Pas de rebuild si déjà offline — la boucle de retry
      // n'émet un frame qu'au changement d'état réel.
      if (ref.mounted && state.daemonOnline) {
        state = state.copyWith(daemonOnline: false);
      }
      _scheduleDaemonRetry();
    }
  }

  /// Arme le re-bootstrap — idempotent : un timer déjà armé
  /// couvre les échecs suivants (bootstrap et actions REST
  /// échouées en « injoignable » partagent la même file).
  void _scheduleDaemonRetry() {
    if (!ref.mounted || _daemonRetry?.isActive == true) return;
    _daemonRetry = Timer(_cfg.daemonRetryDelay, () {
      _daemonRetry = null;
      unawaited(_bootstrap());
    });
  }

  /// Réessaie la connexion (bouton « daemon absent »).
  Future<void> retry() => _bootstrap();

  /// `navigated`/`title_changed`/`frame_invalidated`/`tab_*` →
  /// mise à jour de l'état d'affichage.
  void _onEvent(ServerEvent ev) {
    final tabId = '${ev.data['tab']}';
    switch (ev.name) {
      case 'navigated':
        _patch(
          tabId,
          (t) => t.copyWith(
            url: ev.data['url'] as String? ?? t.url,
            title: ev.data['title'] as String? ?? t.title,
            loading: false,
          ),
        );
        // Page neuve : la frame retenue n'a plus de `scroll_y`
        // valide — la traduction locale se désarme jusqu'à la
        // première peinture (étape 109 : translaterait une
        // page différente pendant le chargement).
        _paintedScrollY.remove(tabId);
        // La nouvelle page n'a aucun champ focalisé — le
        // flag global repart à zéro si c'est l'onglet actif.
        if (tabId == state.activeId && state.fieldFocused) {
          state = state.copyWith(fieldFocused: false);
        }
        if (state.pageErrors.containsKey(tabId)) {
          state = state.copyWith(
            pageErrors: {...state.pageErrors}..remove(tabId),
          );
        }
        // Filet : `MediaGone` précède `navigated` au contrat —
        // la purge couvre tout ordre inattendu côté daemon.
        if (state.media.containsKey(tabId) && state.media[tabId]!.isNotEmpty) {
          _dropAllMedia(tabId);
        }
      case 'title_changed':
        _patch(
          tabId,
          (t) => t.copyWith(title: ev.data['title'] as String? ?? t.title),
        );
      case 'frame_invalidated':
        // Scroll atomique avec l'invalidation (champ additif
        // 71d) — n'écrire qu'au changement : un rebuild par
        // frame pour rien coûte plus que le test.
        final y = (ev.data['scroll_y'] as num?)?.toDouble();
        if (y != null && state.scrollY[tabId] != y) {
          state = state.copyWith(scrollY: {...state.scrollY, tabId: y});
        }
        // Ét.109 — `rects` dirty (px physiques) consommées :
        // plan partiel quand la frame retenue reste à la même
        // position, plein sinon.
        _planInvalidation(tabId, _parseRects(ev.data['rects']), y);
        if (tabId == state.activeId) {
          state = state.copyWith(frameTick: state.frameTick + 1);
        }
        unawaited(_pullFrame(tabId));
      case 'media_offered':
        final offer = MediaOffer.fromJson(ev.data);
        _putMedia(tabId, offer);
        unawaited(_claimMedia(tabId, offer));
      case 'media_moved':
        {
          final mid = (ev.data['media'] as num?)?.toInt();
          final r = ev.data['rect'] as Map<String, dynamic>?;
          if (mid != null && r != null) {
            _patchMedia(
              tabId,
              mid,
              (m) => m.copyWith(
                rect: ui.Rect.fromLTWH(
                  (r['x'] as num?)?.toDouble() ?? m.rect.left,
                  (r['y'] as num?)?.toDouble() ?? m.rect.top,
                  (r['w'] as num?)?.toDouble() ?? m.rect.width,
                  (r['h'] as num?)?.toDouble() ?? m.rect.height,
                ),
              ),
            );
          }
        }
      case 'media_gone':
        {
          final mid = (ev.data['media'] as num?)?.toInt();
          if (mid != null) _dropMedia(tabId, mid);
        }
      case 'media_command':
        _onMediaCommand(tabId, ev.data);
      case 'pipeline_error':
        _patch(tabId, (t) => t.copyWith(loading: false));
        final msg = ev.data['message'] as String?;
        state = state.copyWith(
          lastError: () => msg,
          pageErrors: msg == null ? null : {...state.pageErrors, tabId: msg},
        );
      case 'download_offered':
        // Étape 70 : corps non affichable bufferisé —
        // l'UI propose « Enregistrer / Ouvrir comme texte ».
        state = state.copyWith(
          downloadOffer: () => DownloadOffer.fromJson(ev.data),
        );
      case 'download_done':
        state = state.copyWith(
          lastError: () => 'Téléchargement enregistré : ${ev.data['path']}',
        );
      case 'tab_closed':
        final tabs = state.tabs.where((t) => t.id != tabId).toList();
        _dropAllMedia(tabId);
        state = state.copyWith(
          tabs: tabs,
          activeId: state.activeId == tabId
              ? (tabs.isEmpty ? null : tabs.first.id)
              : state.activeId,
          scrollY: {...state.scrollY}..remove(tabId),
          media: {...state.media}..remove(tabId),
        );
        // Raster de l'onglet mort — `dispose()` décrémente la
        // référence ; le rasteriseur libère les pixels quand sa
        // frame en cours finit de peindre.
        _frameImages.remove(tabId)?.dispose();
        _paintedScrollY.remove(tabId);
        _pendingRects.remove(tabId);
        _pendingFull.remove(tabId);
    }
  }

  /// Accepte le téléchargement proposé (étape 70) : écrit
  /// sous le répertoire du daemon — `DownloadDone` SSE
  /// rapporte le chemin (snackbar via `lastError`).
  Future<void> acceptDownload(DownloadOffer offer) async {
    state = state.copyWith(downloadOffer: () => null);
    try {
      await _repo.acceptDownload(offer.tab, offer.id);
    } on Exception catch (e) {
      state = state.copyWith(lastError: () => 'Téléchargement : $e');
    }
  }

  /// Rejette le téléchargement : le corps est rejoué comme
  /// document (« dump texte » — l'onglet navigue dessus).
  Future<void> declineDownload(DownloadOffer offer) async {
    state = state.copyWith(downloadOffer: () => null);
    try {
      await _repo.declineDownload(offer.tab, offer.id);
    } on Exception catch (e) {
      state = state.copyWith(lastError: () => 'Téléchargement : $e');
    }
  }

  // ----------------------------------------------------------------
  // Seam MediaSurface (étape 71d) — offres → claim → lecteur
  // natif ; commandes JS → lecteur ; état lecteur → moteur
  // (≤ 4 Hz). `rect` document − `scroll_y` côté overlay.
  // ----------------------------------------------------------------

  /// Vue vidéo native d'une surface claimée (`null` si le
  /// lecteur n'existe pas encore) — posée dans l'overlay
  /// `Positioned` du `_FrameView`.
  Widget? mediaView(String tabId, int mediaId) =>
      _players[tabId]?[mediaId]?.view;

  /// `true` quand le lecteur a une première frame — l'overlay
  /// affiche un fond opaque en attendant (pas de « trou »).
  bool mediaReady(String tabId, int mediaId) =>
      _players[tabId]?[mediaId]?.ready == true;

  /// Toggle play/pause pour les surfaces `controls` (UI hôte
  /// minimale v1 : tap = play/pause).
  void mediaToggle(String tabId, int mediaId) {
    final p = _players[tabId]?[mediaId];
    if (p == null) return;
    unawaited(p.snapshot.state == 'playing' ? p.pause() : p.play());
  }

  /// Réconciliation au bootstrap : les offres déjà présentes
  /// (émises avant la reconnexion SSE) sont listées puis
  /// claimées comme les offres live.
  Future<void> _syncMedia(String tabId) async {
    try {
      for (final m in await _repo.listMedia(tabId)) {
        _putMedia(tabId, m);
        if (!m.claimed) unawaited(_claimMedia(tabId, m));
      }
    } on Object {
      // Daemon sans surfaces — rien à réconcilier.
    }
  }

  /// Pose le claim REST puis instancie le lecteur natif.
  /// Échec → l'offre reste non claimée (les stubs ADR-0023
  /// continuent côté JS — comportement honnête du contrat).
  Future<void> _claimMedia(String tabId, MediaOffer offer) async {
    try {
      final ok = await _repo.claimMedia(tabId, offer.id, _cfg.mediaCodecs);
      if (!ok || !ref.mounted) return;
      _patchMedia(tabId, offer.id, (m) => m.copyWith(claimed: true));
      final player = await _playerFactory(offer.url);
      if (!ref.mounted || state.media[tabId]?[offer.id] == null) {
        unawaited(player.dispose());
        return;
      }
      final key = _mkey(tabId, offer.id);
      void listener() => _reportMedia(tabId, offer.id);
      _playerListeners[key] = listener;
      player.addListener(listener);
      (_players[tabId] ??= {})[offer.id] = player;
      // Premier rapport forcé : le moteur attend un état
      // initial pour refléter `paused`/`readyState`.
      unawaited(_reportMedia(tabId, offer.id, force: true));
      if (offer.autoplay) unawaited(player.play());
    } on Object {
      // Claim refusé ou lecteur en échec — non claimé honnête.
    }
  }

  /// Ordre `media_command` JS → lecteur natif (uniquement
  /// sur surface claimée — le moteur n'émet que là).
  void _onMediaCommand(String tabId, Map<String, dynamic> data) {
    final mid = (data['media'] as num?)?.toInt();
    final p = mid == null ? null : _players[tabId]?[mid];
    final cmd = data['cmd'];
    if (p == null || cmd is! Map<String, dynamic>) return;
    switch (cmd['type']) {
      case 'play':
        unawaited(p.play());
      case 'pause':
        unawaited(p.pause());
      case 'seek':
        unawaited(
          p.seekTo(Duration(milliseconds: (cmd['ms'] as num?)?.toInt() ?? 0)),
        );
      case 'volume':
        unawaited(p.setVolume((cmd['v'] as num?)?.toDouble() ?? 1.0));
      case 'muted':
        unawaited(p.setMuted(cmd['on'] == true));
      case 'rate':
        unawaited(p.setRate((cmd['r'] as num?)?.toDouble() ?? 1.0));
    }
  }

  /// Rapport `POST …/state` borné : saut si < `mediaReportInterval`
  /// sauf changement d'état (playing↔paused↔ended↔error) ou
  /// `force` — le changement d'état vaut plus que la cadence.
  Future<void> _reportMedia(
    String tabId,
    int mediaId, {
    bool force = false,
  }) async {
    final p = _players[tabId]?[mediaId];
    if (p == null) return;
    final key = _mkey(tabId, mediaId);
    final snap = p.snapshot;
    final now = DateTime.now();
    final changed = _reportStates[key] != snap.state;
    final at = _reportTimes[key];
    final due = at == null || now.difference(at) >= _cfg.mediaReportInterval;
    if (!force && !changed && !due) return;
    _reportStates[key] = snap.state;
    _reportTimes[key] = now;
    try {
      await _repo.mediaState(tabId, mediaId, snap.toJson());
    } on Object {
      // Surface morte (410) → `media_gone` nettoiera.
    }
  }

  void _putMedia(String tabId, MediaOffer offer) {
    final perTab = {...?state.media[tabId], offer.id: offer};
    state = state.copyWith(media: {...state.media, tabId: perTab});
  }

  void _patchMedia(
    String tabId,
    int mediaId,
    MediaOffer Function(MediaOffer) f,
  ) {
    final cur = state.media[tabId]?[mediaId];
    if (cur == null) return;
    _putMedia(tabId, f(cur));
  }

  /// Retire une surface : listener → dispose lecteur → offre.
  /// Pas de `release` REST — `MediaGone` signifie que le
  /// moteur a déjà oublié la surface (410 garanti).
  void _dropMedia(String tabId, int mediaId) {
    final key = _mkey(tabId, mediaId);
    final p = _players[tabId]?.remove(mediaId);
    if (p != null) {
      final l = _playerListeners.remove(key);
      if (l != null) p.removeListener(l);
      unawaited(p.dispose());
    }
    if (_players[tabId]?.isEmpty ?? false) _players.remove(tabId);
    _reportTimes.remove(key);
    _reportStates.remove(key);
    final perTab = state.media[tabId];
    if (perTab != null && perTab.containsKey(mediaId)) {
      final next = {...perTab}..remove(mediaId);
      state = state.copyWith(media: {...state.media, tabId: next});
    }
  }

  void _dropAllMedia(String tabId) {
    for (final mid in _players[tabId]?.keys.toList() ?? const <int>[]) {
      _dropMedia(tabId, mid);
    }
  }

  /// Libère les ressources natives **sans toucher `state`** —
  /// réservé à `onDispose` (Riverpod interdit l'écriture
  /// d'état dans le cycle de vie du provider).
  void _releaseAllPlayers() {
    for (final entry in _players.entries.toList()) {
      for (final mid in entry.value.keys.toList()) {
        final key = _mkey(entry.key, mid);
        final p = entry.value.remove(mid);
        if (p != null) {
          final l = _playerListeners.remove(key);
          if (l != null) p.removeListener(l);
          unawaited(p.dispose());
        }
      }
    }
    _players.clear();
    _reportTimes.clear();
    _reportStates.clear();
  }

  /// `rects` du `frame_invalidated` REST → `ui.Rect` (px
  /// physiques, même unité que `frame?x&y&w&h`).
  /// `null` = champ absent (daemon antérieur — pull plein,
  /// comme avant le contrat 55).
  List<ui.Rect>? _parseRects(Object? raw) {
    if (raw is! List) return null;
    return [
      for (final r in raw)
        if (r is Map)
          ui.Rect.fromLTWH(
            (r['x'] as num?)?.toDouble() ?? 0,
            (r['y'] as num?)?.toDouble() ?? 0,
            (r['w'] as num?)?.toDouble() ?? 0,
            (r['h'] as num?)?.toDouble() ?? 0,
          ),
    ];
  }

  /// Décide du prochain pull (étape 109) : **plein** si la
  /// frame retenue n'est plus à la même position (scroll —
  /// les `rects` post-scroll vivent dans les coordonnées
  /// raster du *nouveau* window, incompatibles avec l'image
  /// pré-scroll), si `rects` est vide/absent (contrat 55 :
  /// vide = repeindre entièrement), ou si le volume dépasse
  /// les bornes config. Sinon les régions s'accumulent —
  /// le pull n'en cherchera que les morceaux.
  void _planInvalidation(String tabId, List<ui.Rect>? rects, double? scrollY) {
    final painted = _paintedScrollY[tabId];
    final scrolled =
        scrollY != null &&
        painted != null &&
        (scrollY - painted).abs() > _cfg.scrollSyncEpsilon;
    if (painted == null ||
        scrolled ||
        rects == null ||
        rects.isEmpty) {
      _pendingFull.add(tabId);
      _pendingRects.remove(tabId);
      return;
    }
    final list = (_pendingRects[tabId] ??= [])..addAll(rects);
    var area = 0.0;
    for (final r in list) {
      area += r.width * r.height;
    }
    final img = _frameImages[tabId];
    final frameArea = (img?.width ?? 0) * (img?.height ?? 0);
    if (list.length > _cfg.frameRegionMaxRects ||
        area > frameArea * _cfg.frameRegionMaxArea) {
      _pendingFull.add(tabId);
      _pendingRects.remove(tabId);
    }
  }

  void _patch(String tabId, BrowserTab Function(BrowserTab) f) {
    state = state.copyWith(
      tabs: [for (final t in state.tabs) t.id == tabId ? f(t) : t],
    );
  }

  /// Nouvel onglet (devient actif).
  Future<void> newTab() async {
    try {
      final t = await _repo.createTab();
      state = state.copyWith(
        tabs: [...state.tabs, t],
        activeId: t.id,
        daemonOnline: true,
      );
    } on ApiException catch (e) {
      state = state.copyWith(lastError: () => e.message);
    }
  }

  /// Remonte une erreur d'un contrôleur satellite (find bar,
  /// étape 57bis) dans `lastError` — même affichage que les
  /// erreurs d'action.
  void setError(String message) =>
      state = state.copyWith(lastError: () => message);

  /// Ferme un onglet (côté daemon + localement — l'événement
  /// `tab_closed` confirme).
  Future<void> closeTab(String id) async {
    try {
      await _repo.closeTab(id);
      _onEvent(ServerEvent('tab_closed', {'tab': int.tryParse(id) ?? id}));
    } on ApiException catch (e) {
      state = state.copyWith(lastError: () => e.message);
    }
  }

  /// Sélectionne l'onglet affiché — tire sa frame si absente.
  /// `fieldFocused` décrit l'onglet actif : flag global, il
  /// repart à zéro au changement d'onglet (le focus réel de
  /// l'onglet entrant sera re-appris au prochain clic).
  void selectTab(String id) {
    state = state.copyWith(activeId: id, fieldFocused: false);
    if (!_frameImages.containsKey(id)) {
      unawaited(_pullFrame(id));
    }
  }

  /// Ctrl+Tab / Ctrl+Shift+Tab — cyclage circulaire dans la
  /// liste d'onglets (étape 85). `dir` = ±1 ; no-op sur liste
  /// vide ou onglet actif introuvable (sélection du premier /
  /// dernier, bord le plus proche).
  void cycleTab(int dir) {
    final n = state.tabs.length;
    if (n == 0) return;
    final i = state.tabs.indexWhere((t) => t.id == state.activeId);
    final next = i < 0 ? (dir >= 0 ? 0 : n - 1) : (((i + dir) % n) + n) % n;
    selectTab(state.tabs[next].id);
  }

  /// Ctrl+1..8 — sélection directe de l'onglet `index`
  /// (étape 85 ; Ctrl+9 = dernier, résolu par l'appelant).
  void selectTabAt(int index) {
    if (index < 0 || index >= state.tabs.length) return;
    selectTab(state.tabs[index].id);
  }

  /// Clic-molette sur un lien → ouverture dans un nouvel
  /// onglet **en arrière-plan** (étape 85 — sémantique
  /// Chrome/Firefox) : l'onglet courant garde le premier
  /// plan pendant que le neuf charge derrière.
  Future<void> openInNewTab(String url) async {
    final prev = state.activeId;
    await newTab();
    final fresh = state.activeId;
    if (fresh == null || fresh == prev) return; // échec de création
    unawaited(navigate(url));
    if (prev != null) selectTab(prev);
  }

  /// Pulls en vol / demandés par onglet (étape 89) : un
  /// `frame_invalidated` pendant un pull marque juste
  /// « à refaire » — au plus 1 fetch en vol + 1 en attente,
  /// les rafales (scroll, hover, caret) se replient en
  /// « dernière frame gagne ».
  final Set<String> _pullInflight = {};
  final Set<String> _pullPending = {};

  /// Tire la frame de l'onglet — `ui.Image` brute dans les
  /// deux modes (étape 89 : le REST décode une fois au lieu
  /// de recréer une `MemoryImage` par frame — la clé de cache
  /// par identité des bytes y accumulait des images décodées,
  /// et `Uint8List.fromList` allouait de la mémoire externe
  /// à chaque rebuild). Les PNG octets restent dans `frames`
  /// (marqueur de présence pour `selectTab`).
  Future<void> _pullFrame(String tabId) async {
    if (_pullInflight.contains(tabId)) {
      _pullPending.add(tabId);
      return;
    }
    _pullInflight.add(tabId);
    try {
      do {
        _pullPending.remove(tabId);
        await _pullFrameOnce(tabId);
        if (!ref.mounted) return;
      } while (_pullPending.contains(tabId));
    } finally {
      _pullInflight.remove(tabId);
    }
  }

  /// Une itération de pull : vide le plan accumulé puis
  /// pull **partiel** (régions → composite sur la frame
  /// retenue) ou **plein** en repli (étape 109). Le plan
  /// est drainé en tête — les invalidations arrivées pendant
  /// l'itération s'accumulent pour le tour suivant.
  Future<void> _pullFrameOnce(String tabId) async {
    final rects = _pendingRects.remove(tabId);
    final full =
        _pendingFull.remove(tabId) || rects == null || rects.isEmpty;
    if (!full && _frameImages[tabId] != null) {
      if (await _pullRegions(tabId, rects)) return;
      // Région indisponible/décodage → plein honnête.
    }
    await _pullFullFrame(tabId);
  }

  /// Pull plein — `ui.Image` décodée par chemin unique
  /// REST/FFI (FFI livre les pixels bruts, ADR-0018). L'image
  /// vit hors `BrowserState` : les états obsolètes épinglés
  /// par l'arbre de widgets ne retiennent plus de raster
  /// (fuite ~300 Mo/frame mesurée 22/10).
  Future<void> _pullFullFrame(String tabId) async {
    try {
      final ui.Image img;
      final engine = _repo;
      if (engine is FfiEngine) {
        final i = await engine.fetchFrameImage(tabId);
        if (i == null) return;
        img = i;
      } else {
        final bytes = await _repo.fetchFrame(tabId);
        if (bytes == null || bytes.isEmpty) return;
        final codec = await ui.instantiateImageCodec(bytes);
        try {
          img = (await codec.getNextFrame()).image;
        } finally {
          codec.dispose();
        }
      }
      if (!ref.mounted) {
        img.dispose();
        return;
      }
      _installFrame(tabId, img);
    } on Object {
      // Moteur/daemon indisponible ou PNG indécodable → le
      // prochain tick retentera.
    }
  }

  /// Pull partiel (étape 109) : chaque `rect` dirty devient
  /// un fetch `frame?x&y&w&h` (REST) ou `frame_copy_region`
  /// (FFI), puis le tout se **compose** sur l'image retenue
  /// par `PictureRecorder` — transport et décodage ramenés
  /// à la surface réellement repeinte. `false` → l'appelant
  /// bascule en pull plein (région hors frame, dll
  /// antérieure, decode impossible).
  Future<bool> _pullRegions(String tabId, List<ui.Rect> rects) async {
    final base = _frameImages[tabId];
    if (base == null) return false;
    final engine = _repo;
    final patches = <(ui.Rect, ui.Image)>[];
    try {
      for (final r in rects) {
        final x = r.left.round();
        final y = r.top.round();
        final w = r.width.round();
        final h = r.height.round();
        if (w <= 0 || h <= 0) continue;
        final ui.Image? patch = engine is FfiEngine
            ? await engine.fetchFrameRegionImage(tabId, x, y, w, h)
            : await _pullRegionPng(tabId, x, y, w, h);
        if (patch == null) return false;
        // Daemon sans la route région (pré-étape 55) : il
        // répond la frame entière — la taille décodée déborde
        // la région → composer à (x,y) serait faux → plein.
        if (patch.width > w || patch.height > h) {
          patch.dispose();
          return false;
        }
        patches.add((r, patch));
      }
      final rec = ui.PictureRecorder();
      final c = ui.Canvas(rec);
      c.drawImage(base, ui.Offset.zero, ui.Paint());
      for (final (r, img) in patches) {
        c.drawImage(img, ui.Offset(r.left, r.top), ui.Paint());
      }
      final img = await rec.endRecording().toImage(base.width, base.height);
      if (!ref.mounted) {
        img.dispose();
        return false;
      }
      _installFrame(tabId, img);
      return true;
    } on Object {
      return false;
    } finally {
      for (final (_, img) in patches) {
        img.dispose();
      }
    }
  }

  /// PNG d'une région → `ui.Image` décodée (REST étape 55).
  Future<ui.Image?> _pullRegionPng(
    String tabId,
    int x,
    int y,
    int w,
    int h,
  ) async {
    final bytes = await _repo.fetchFrameRegion(tabId, x, y, w, h);
    if (bytes == null || bytes.isEmpty) return null;
    final codec = await ui.instantiateImageCodec(bytes);
    try {
      return (await codec.getNextFrame()).image;
    } finally {
      codec.dispose();
    }
  }

  /// Installe l'image composée/livrée : remplace l'ancienne
  /// (dispose), mémorise son `scroll_y` de peinture — la
  /// traduction locale retombe à zéro jusqu'au prochain
  /// scroll (étape 109) — et réveille l'affichage.
  void _installFrame(String tabId, ui.Image img) {
    _frameImages[tabId]?.dispose();
    _frameImages[tabId] = img;
    _paintedScrollY[tabId] = state.scrollY[tabId] ?? 0.0;
    state = state.copyWith(frameTick: state.frameTick + 1);
  }

  /// Scroll molette → `POST …/input {scroll,dy}` — le moteur
  /// borne et repeint, `frame_invalidated` suit. La réponse
  /// porte `scroll_y` **autoritaire** (REST JSON + FFI joint
  /// par le worker) : posé tout de suite, il déclenche la
  /// traduction locale de la frame retenue (étape 109) avant
  /// même l'événement — retour visuel sans attente réseau.
  Future<void> scroll(double dy) async {
    final res = await _sendInput({'type': 'scroll', 'dy': dy});
    final y = (res?['scroll_y'] as num?)?.toDouble();
    if (y == null || !ref.mounted) return;
    final id = state.activeId;
    if (id != null && state.scrollY[id] != y) {
      state = state.copyWith(scrollY: {...state.scrollY, id: y});
    }
  }

  /// Survol → `mousemove` (coalescé : dernier état suffit) —
  /// le résultat `hover` pilote le curseur + la barre de
  /// statut (`url`, `editable`).
  Future<void> hover(double x, double y) async {
    final res = await _sendInput({'type': 'mousemove', 'x': x, 'y': y});
    if (res == null || !ref.mounted) return;
    final url = res['url'];
    state = state.copyWith(
      hoverUrl: () => url is String ? url : null,
      hoverEditable: res['editable'] == true,
    );
  }

  /// Clic → `input click` — le moteur hit-teste : lien →
  /// navigation (SSE), champ → `focused` (les frappes sont
  /// ensuite relayées en `input key`).
  Future<void> click(double x, double y) async {
    final res = await _sendInput({'type': 'click', 'x': x, 'y': y});
    if (res == null || !ref.mounted) return;
    state = state.copyWith(fieldFocused: res['result'] == 'focused');
  }

  /// Frappe clavier → `input key` — n'a de sens que si un
  /// champ est focalisé (`fieldFocused`). `unfocused`
  /// resynchronise le flag : le moteur a perdu le focus
  /// (Enter/Escape, nœud supprimé) — sans ça chaque frappe
  /// suivante était envoyée dans le vide (`ignored`) avec
  /// un champ qui semblait focalisé.
  Future<void> key(
    String key, {
    bool ctrl = false,
    bool shift = false,
    bool alt = false,
    bool meta = false,
  }) async {
    final res = await _sendInput({
      'type': 'key',
      'key': key,
      // Ét.105 — état des modifieurs → props `ctrlKey`…
      // du `KeyboardEvent` côté moteur.
      if (ctrl) 'ctrl': true,
      if (shift) 'shift': true,
      if (alt) 'alt': true,
      if (meta) 'meta': true,
    });
    if (res == null || !ref.mounted) return;
    if (res['result'] == 'unfocused' || res['result'] == 'navigated') {
      state = state.copyWith(fieldFocused: false);
    }
  }

  /// Envoie un événement d'entrée ; retourne le JSON de
  /// résultat (`null` en cas d'erreur — déjà en `lastError`).
  Future<Map<String, dynamic>?> _sendInput(Map<String, dynamic> ev) async {
    final id = state.activeId;
    if (id == null) return null;
    try {
      final res = await _repo.sendInput(id, ev);
      // Mode FFI : pas de SSE — l'input invalide → pull
      // direct (le hover pur ne repeint pas forcément).
      if (_repo is FfiEngine &&
          res['result'] != 'hover' &&
          res['result'] != 'ignored') {
        unawaited(_pullFrame(id));
      }
      return res;
    } on Object catch (e) {
      state = state.copyWith(lastError: () => '$e');
      return null;
    }
  }

  /// `POST …/viewport` au redimensionnement — debounced par
  /// l'appelant (le widget accumule, on envoie le dernier).
  Future<void> setViewport(double w, double h, double scale) async {
    final id = state.activeId;
    if (id == null) return;
    try {
      // ignore: avoid_print — sonde terrain 22/10 (frame ≠ fenêtre)
      await _repo.setViewport(id, w, h, scale);
    } on ApiException catch (e) {
      state = state.copyWith(lastError: () => e.message);
    }
  }

  /// Schéma de couleurs effectif (`'dark'`/`'light'`) —
  /// `system` résolu sur la brightness OS courante (étape 97).
  String _effectiveScheme() {
    final theme = ref.read(settingsProvider).theme;
    if (theme == 'dark' || theme == 'light') return theme;
    final b = ui.PlatformDispatcher.instance.platformBrightness;
    return b == Brightness.dark ? 'dark' : 'light';
  }

  /// Pousse le schéma effectif au moteur puis retire la
  /// frame de l'onglet actif (le backend re-render déjà,
  /// REST ; en FFI la re-peinture arrive au prochain tick).
  Future<void> _pushColorScheme() async {
    final id = state.activeId;
    if (id == null) return;
    try {
      await _repo.setColorScheme(id, _effectiveScheme());
      unawaited(_pullFrame(id));
    } on Object {
      // Daemon injoignable → prochain appui/nav réessaie.
    }
  }

  /// Soumission omnibox : complète le schéma si absent puis
  /// `POST …/navigate` — `loading` optimiste jusqu'à
  /// `navigated`/`pipeline_error`.
  Future<void> navigate(String input) async {
    final id = state.activeId;
    if (id == null) return;
    var url = input.trim();
    if (url.isEmpty) return;
    // `about:blank` (bouton Home, étape 88) passe tel quel —
    // le moteur sert le document vide canonique sans fetch.
    if (!url.contains('://') && !url.startsWith('about:')) {
      url = 'http://$url';
    }
    _patch(id, (t) => t.copyWith(loading: true, url: url));
    try {
      final t = await _repo.navigate(id, url);
      if (t != null) _patch(id, (_) => t.copyWith(loading: false));
      state = state.copyWith(daemonOnline: true);
      unawaited(_pullFrame(id));
    } on ApiException catch (e) {
      _patch(id, (t) => t.copyWith(loading: false));
      state = state.copyWith(
        lastError: () => e.message,
        pageErrors: {...state.pageErrors, id: e.message},
      );
      _markOfflineIfUnreachable(e);
    }
  }

  /// `back`/`forward`/`reload` sur l'onglet actif.
  Future<void> goBack() => _doAction((r, id) => r.back(id));
  Future<void> goForward() => _doAction((r, id) => r.forward(id));

  /// Recharge l'onglet actif (étape 93) : une page servie
  /// par méthode non sûre répond `resubmit_confirm` — la
  /// resoumission attend l'accord explicite (`reloadConfirmed`
  /// rejoue, `dismissResubmit` abandonne).
  Future<void> reload() async {
    final id = state.activeId;
    if (id == null) return;
    _patch(id, (t) => t.copyWith(loading: true));
    try {
      final r = await _repo.reload(id);
      _patch(id, (t) => t.copyWith(loading: false));
      if (r.needsConfirm) {
        state = state.copyWith(
          resubmit: () =>
              (tabId: id, method: r.resubmitMethod!, url: r.resubmitUrl!),
        );
      } else if (r.tab != null) {
        _patch(id, (_) => r.tab!);
      }
    } on ApiException catch (e) {
      _patch(id, (t) => t.copyWith(loading: false));
      state = state.copyWith(lastError: () => e.message);
      _markOfflineIfUnreachable(e);
    }
  }

  /// L'utilisateur accepte la resoumission → rejoue la
  /// requête non sûre avec `{"confirm":true}`.
  Future<void> reloadConfirmed() async {
    final pending = state.resubmit;
    if (pending == null) return;
    state = state.copyWith(resubmit: () => null);
    final id = pending.tabId;
    _patch(id, (t) => t.copyWith(loading: true));
    try {
      final r = await _repo.reload(id, confirm: true);
      _patch(id, (t) => t.copyWith(loading: false));
      if (r.tab != null) _patch(id, (_) => r.tab!);
    } on ApiException catch (e) {
      _patch(id, (t) => t.copyWith(loading: false));
      state = state.copyWith(lastError: () => e.message);
      _markOfflineIfUnreachable(e);
    }
  }

  /// L'utilisateur refuse la resoumission — rien n'est
  /// rejoué (comportement « Annuler » du dialogue navigateur).
  void dismissResubmit() => state = state.copyWith(resubmit: () => null);

  Future<void> _doAction(
    Future<BrowserTab?> Function(BrowserRepository, String) act,
  ) async {
    final id = state.activeId;
    if (id == null) return;
    _patch(id, (t) => t.copyWith(loading: true));
    try {
      final t = await act(_repo, id);
      if (t != null) _patch(id, (_) => t.copyWith(loading: false));
    } on ApiException catch (e) {
      _patch(id, (t) => t.copyWith(loading: false));
      state = state.copyWith(lastError: () => e.message);
      _markOfflineIfUnreachable(e);
    }
  }

  /// Échec **de connexion** (`statusCode` absent = « daemon
  /// injoignable », pas une erreur HTTP) → la bannière reflète
  /// l'état réel et le re-bootstrap est armé. Une erreur HTTP
  /// prouve au contraire que le daemon répond.
  void _markOfflineIfUnreachable(ApiException e) {
    if (e.statusCode != null) return;
    if (state.daemonOnline) {
      state = state.copyWith(daemonOnline: false);
    }
    _scheduleDaemonRetry();
  }
}

/// Provider du contrôleur — l'UI lit `browserProvider`.
final browserProvider = NotifierProvider<BrowserController, BrowserState>(
  BrowserController.new,
);
