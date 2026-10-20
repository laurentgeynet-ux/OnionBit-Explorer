// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/api/api_client.dart';
import '../../../core/api/events_client.dart';
import '../../../core/api/ffi_engine.dart';
import '../../../core/config/app_config.dart';
import '../../../core/di/providers.dart';
import '../../../core/platform/env.dart';
import '../../../core/settings/app_settings.dart';
import '../data/browser_repository.dart';
import '../domain/bookmark.dart';
import '../domain/browser_tab.dart';
import '../domain/download_offer.dart';
import '../domain/history_entry.dart';

/// État UI du navigateur : onglets, onglet actif, disponibilité
/// du daemon, dernière erreur métier.
class BrowserState {
  const BrowserState({
    this.tabs = const [],
    this.activeId,
    this.frames = const {},
    this.daemonOnline = false,
    this.lastError,
    this.frameTick = 0,
    this.hoverUrl,
    this.hoverEditable = false,
    this.fieldFocused = false,
    this.pageErrors = const {},
    this.rawFrames = const {},
    this.downloadOffer,
  });

  final List<BrowserTab> tabs;
  final String? activeId;

  /// Frames PNG par onglet (octets bruts) — tirées sur
  /// `frame_invalidated` (ADR-0004 : pull, pas push).
  final Map<String, List<int>> frames;

  /// Frames décodées `ui.Image` par onglet — canal FFI
  /// in-process (ADR-0018) : pixels RGBA bruts, aucun codec
  /// PNG sur le chemin de rendu.
  final Map<String, ui.Image> rawFrames;

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
    Map<String, List<int>>? frames,
    bool? daemonOnline,
    String? Function()? lastError,
    int? frameTick,
    String? Function()? hoverUrl,
    bool? hoverEditable,
    bool? fieldFocused,
    Map<String, String>? pageErrors,
    Map<String, ui.Image>? rawFrames,
    DownloadOffer? Function()? downloadOffer,
  }) => BrowserState(
    tabs: tabs ?? this.tabs,
    activeId: activeId ?? this.activeId,
    frames: frames ?? this.frames,
    rawFrames: rawFrames ?? this.rawFrames,
    daemonOnline: daemonOnline ?? this.daemonOnline,
    lastError: lastError != null ? lastError() : this.lastError,
    frameTick: frameTick ?? this.frameTick,
    hoverUrl: hoverUrl != null ? hoverUrl() : this.hoverUrl,
    hoverEditable: hoverEditable ?? this.hoverEditable,
    fieldFocused: fieldFocused ?? this.fieldFocused,
    pageErrors: pageErrors ?? this.pageErrors,
    downloadOffer: downloadOffer != null ? downloadOffer() : this.downloadOffer,
  );
}

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
  final profile = ref.watch(settingsProvider).profileDir;
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

  /// Dépôt capturé dans `build` — `ref` est invalide après un
  /// `await`, les lectures post-build passent par ce champ.
  late final BrowserRepository _repo;

  /// Config capturée dans `build` (cadence de re-bootstrap).
  late final AppConfig _cfg;

  @override
  BrowserState build() {
    _repo = ref.watch(browserRepositoryProvider);
    _cfg = ref.read(configProvider);
    ref.onDispose(() => _daemonRetry?.cancel());
    if (_repo is FfiEngine) {
      // Pas de SSE in-process : la sondage d'animations porte
      // l'invalidation ; les actions tirent leur frame au
      // retour (navigate/input).
      _animTimer = Timer.periodic(
        const Duration(milliseconds: 50),
        (_) => _tickFfi(),
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
  void _tickFfi() {
    final engine = _repo;
    final id = state.activeId;
    if (engine is! FfiEngine || id == null) return;
    try {
      if (engine.tick(id)) unawaited(_pullFrame(id));
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
      );
    } on Object {
      if (ref.mounted) state = state.copyWith(daemonOnline: false);
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
        if (state.pageErrors.containsKey(tabId)) {
          state = state.copyWith(
            pageErrors: {...state.pageErrors}..remove(tabId),
          );
        }
      case 'title_changed':
        _patch(
          tabId,
          (t) => t.copyWith(title: ev.data['title'] as String? ?? t.title),
        );
      case 'frame_invalidated':
        if (tabId == state.activeId) {
          state = state.copyWith(frameTick: state.frameTick + 1);
        }
        unawaited(_pullFrame(tabId));
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
          lastError: () =>
              'Téléchargement enregistré : ${ev.data['path']}',
        );
      case 'tab_closed':
        final tabs = state.tabs.where((t) => t.id != tabId).toList();
        state = state.copyWith(
          tabs: tabs,
          activeId: state.activeId == tabId
              ? (tabs.isEmpty ? null : tabs.first.id)
              : state.activeId,
        );
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
  void selectTab(String id) {
    state = state.copyWith(activeId: id);
    if (!state.frames.containsKey(id) && !state.rawFrames.containsKey(id)) {
      unawaited(_pullFrame(id));
    }
  }

  /// Tire la frame de l'onglet — `ui.Image` brute en mode
  /// FFI (ADR-0018), PNG en mode REST (ADR-0004 : le push ne
  /// signale que, le contenu voyage en pull).
  Future<void> _pullFrame(String tabId) async {
    final engine = _repo;
    if (engine is FfiEngine) {
      try {
        final img = await engine.fetchFrameImage(tabId);
        if (img == null || !ref.mounted) return;
        final old = state.rawFrames[tabId];
        state = state.copyWith(rawFrames: {...state.rawFrames, tabId: img});
        old?.dispose();
      } on Object {
        // Moteur indisponible → le prochain tick retentera.
      }
      return;
    }
    try {
      final bytes = await _repo.fetchFrame(tabId);
      if (bytes == null || !ref.mounted) return;
      state = state.copyWith(frames: {...state.frames, tabId: bytes});
    } on Object {
      // Daemon coupé → la prochaine invalidation retentera.
    }
  }

  /// Scroll molette → `POST …/input {scroll,dy}` — le moteur
  /// borne et repeint, `frame_invalidated` suit.
  Future<void> scroll(double dy) => _sendInput({'type': 'scroll', 'dy': dy});

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
  /// champ est focalisé (`fieldFocused`).
  Future<void> key(String key) => _sendInput({'type': 'key', 'key': key});

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
      await _repo.setViewport(id, w, h, scale);
    } on ApiException catch (e) {
      state = state.copyWith(lastError: () => e.message);
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
    if (!url.contains('://')) url = 'http://$url';
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
  Future<void> reload() => _doAction((r, id) => r.reload(id));

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
