// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/api/api_client.dart';
import '../../../core/api/events_client.dart';
import '../../../core/di/providers.dart';
import '../data/browser_repository.dart';
import '../domain/browser_tab.dart';

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
  });

  final List<BrowserTab> tabs;
  final String? activeId;

  /// Frames PNG par onglet (octets bruts) — tirées sur
  /// `frame_invalidated` (ADR-0004 : pull, pas push).
  final Map<String, List<int>> frames;

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
  }) =>
      BrowserState(
        tabs: tabs ?? this.tabs,
        activeId: activeId ?? this.activeId,
        frames: frames ?? this.frames,
        daemonOnline: daemonOnline ?? this.daemonOnline,
        lastError: lastError != null ? lastError() : this.lastError,
        frameTick: frameTick ?? this.frameTick,
      );
}

/// Dépôt injectable (tests : `Provider.overrideWith`).
final browserRepositoryProvider = Provider<BrowserRepository>(
  (ref) => BrowserRepository(ref.watch(apiClientProvider)),
);

/// Contrôleur navigateur — pilote les actions REST et applique
/// les événements SSE au `BrowserState` (ADR-0003/0004).
class BrowserController extends Notifier<BrowserState> {
  StreamSubscription<ServerEvent>? _sub;

  /// Dépôt capturé dans `build` — `ref` est invalide après un
  /// `await`, les lectures post-build passent par ce champ.
  late final BrowserRepository _repo;

  @override
  BrowserState build() {
    final events = ref.watch(eventsClientProvider);
    _repo = ref.watch(browserRepositoryProvider);
    _sub?.cancel();
    _sub = events.events().listen(_onEvent);
    ref.onDispose(() => _sub?.cancel());
    // Microtask : le bootstrap touche `state` après le retour
    // de `build` (un dépôt qui jette synchronement ne doit
    // pas lire un provider non initialisé).
    unawaited(Future.microtask(_bootstrap));
    return const BrowserState();
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
      state = state.copyWith(
        tabs: tabs,
        activeId: state.activeId ?? tabs.first.id,
        daemonOnline: true,
      );
    } on ApiException {
      if (ref.mounted) state = state.copyWith(daemonOnline: false);
    }
  }

  /// Réessaie la connexion (bouton « daemon absent »).
  Future<void> retry() => _bootstrap();

  /// `navigated`/`title_changed`/`frame_invalidated`/`tab_*` →
  /// mise à jour de l'état d'affichage.
  void _onEvent(ServerEvent ev) {
    final tabId = '${ev.data['tab']}';
    switch (ev.name) {
      case 'navigated':
        _patch(tabId, (t) => t.copyWith(
              url: ev.data['url'] as String? ?? t.url,
              title: ev.data['title'] as String? ?? t.title,
              loading: false,
            ));
      case 'title_changed':
        _patch(tabId, (t) =>
            t.copyWith(title: ev.data['title'] as String? ?? t.title));
      case 'frame_invalidated':
        if (tabId == state.activeId) {
          state = state.copyWith(frameTick: state.frameTick + 1);
        }
        unawaited(_pullFrame(tabId));
      case 'pipeline_error':
        _patch(tabId, (t) => t.copyWith(loading: false));
        state = state.copyWith(
          lastError: () => ev.data['message'] as String?,
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
    if (!state.frames.containsKey(id)) unawaited(_pullFrame(id));
  }

  /// Tire le PNG de l'onglet (`GET …/frame`) — le push SSE ne
  /// fait que signaler, le contenu voyage en pull (ADR-0004).
  Future<void> _pullFrame(String tabId) async {
    try {
      final bytes = await _repo.fetchFrame(tabId);
      if (bytes == null || !ref.mounted) return;
      state = state.copyWith(
        frames: {...state.frames, tabId: bytes},
      );
    } on Object {
      // Daemon coupé → la prochaine invalidation retentera.
    }
  }

  /// Scroll molette → `POST …/input {scroll,dy}` — le moteur
  /// borne et repeint, `frame_invalidated` suit.
  Future<void> scroll(double dy) => _sendInput({'type': 'scroll', 'dy': dy});

  /// Survol → `mousemove` (coalescé : dernier état suffit).
  Future<void> hover(double x, double y) =>
      _sendInput({'type': 'mousemove', 'x': x, 'y': y});

  /// Clic → `input click` — le moteur hit-teste et navigue
  /// si c'est un lien (étape 31 côté UI : curseur).
  Future<void> click(double x, double y) =>
      _sendInput({'type': 'click', 'x': x, 'y': y});

  Future<void> _sendInput(Map<String, dynamic> ev) async {
    final id = state.activeId;
    if (id == null) return;
    try {
      await _repo.sendInput(id, ev);
    } on ApiException catch (e) {
      state = state.copyWith(lastError: () => e.message);
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
      state = state.copyWith(lastError: () => e.message);
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
    }
  }
}

/// Provider du contrôleur — l'UI lit `browserProvider`.
final browserProvider =
    NotifierProvider<BrowserController, BrowserState>(BrowserController.new);
