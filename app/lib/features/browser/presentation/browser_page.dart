// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/design/design_tokens.dart';
import '../domain/browser_tab.dart';
import 'browser_controller.dart';

/// Coquille navigateur — étape 29 : barre d'onglets, boutons
/// back/forward/reload, omnibox, états de chargement poussés
/// par SSE. Le contenu (frame PNG) est l'étape 30.
class BrowserPage extends ConsumerStatefulWidget {
  const BrowserPage({super.key, required this.tokens});

  final DesignTokens tokens;

  @override
  ConsumerState<BrowserPage> createState() => _BrowserPageState();
}

class _BrowserPageState extends ConsumerState<BrowserPage> {
  final _omnibox = TextEditingController();

  @override
  void dispose() {
    _omnibox.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.tokens;
    final st = ref.watch(browserProvider);
    final ctrl = ref.read(browserProvider.notifier);

    // Omnibox synchronisée sur le changement d'onglet/URL —
    // uniquement si l'utilisateur n'est pas en train d'éditer.
    ref.listen(browserProvider.select((s) => s.activeId), (_, id) {
      final active = ref.read(browserProvider).active;
      if (active != null && _omnibox.text != active.url) {
        _omnibox.text = active.url == 'about:blank' ? '' : active.url;
      }
    });
    ref.listen(browserProvider.select((s) => s.lastError), (_, msg) {
      if (msg != null && mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text(msg)));
      }
    });

    final active = st.active;
    final muted = t.color('onSurface', 'muted');

    return Scaffold(
      backgroundColor: t.color('surface', 'base'),
      body: SafeArea(
        child: Column(
          children: [
            _TabStrip(tokens: t, state: st, ctrl: ctrl),
            _NavBar(
              tokens: t,
              tab: active,
              enabled: st.daemonOnline && active != null,
              omnibox: _omnibox,
              ctrl: ctrl,
            ),
            if (!st.daemonOnline)
              _OfflineBanner(tokens: t, onRetry: ctrl.retry),
            Expanded(
              child: active == null
                  ? Center(
                      child: Text(
                        'Aucun onglet',
                        style: TextStyle(color: muted),
                      ),
                    )
                  : _FrameView(
                      tokens: t,
                      state: st,
                      ctrl: ctrl,
                    ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Bandeau « daemon injoignable » + bouton réessayer.
class _OfflineBanner extends StatelessWidget {
  const _OfflineBanner({required this.tokens, required this.onRetry});

  final DesignTokens tokens;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: tokens.color('danger', 'base', fallback: const Color(0xFFB8860B)),
      child: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: tokens.dim('spacing', 'md', fallback: 16),
          vertical: tokens.dim('spacing', 'xs', fallback: 4),
        ),
        child: Row(
          children: [
            const Expanded(
              child: Text('Daemon injoignable — lancez explorer-daemon'),
            ),
            TextButton(onPressed: onRetry, child: const Text('Réessayer')),
          ],
        ),
      ),
    );
  }
}

/// Barre d'onglets horizontale : titre + spinner + fermeture,
/// « + » en fin de liste.
class _TabStrip extends StatelessWidget {
  const _TabStrip({required this.tokens, required this.state, required this.ctrl});

  final DesignTokens tokens;
  final BrowserState state;
  final BrowserController ctrl;

  @override
  Widget build(BuildContext context) {
    final h = tokens.dim('metrics', 'tabHeight', fallback: 36);
    return SizedBox(
      height: h,
      child: ListView(
        scrollDirection: Axis.horizontal,
        children: [
          for (final tab in state.tabs)
            _TabChip(
              tokens: tokens,
              tab: tab,
              active: tab.id == state.activeId,
              onSelect: () => ctrl.selectTab(tab.id),
              onClose: () => ctrl.closeTab(tab.id),
            ),
          IconButton(
            icon: const Icon(Icons.add),
            onPressed: state.daemonOnline ? ctrl.newTab : null,
            tooltip: 'Nouvel onglet',
          ),
        ],
      ),
    );
  }
}

/// Un onglet dans la barre.
class _TabChip extends StatelessWidget {
  const _TabChip({
    required this.tokens,
    required this.tab,
    required this.active,
    required this.onSelect,
    required this.onClose,
  });

  final DesignTokens tokens;
  final dynamic tab;
  final bool active;
  final VoidCallback onSelect;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final label = tab.title.isNotEmpty ? tab.title : tab.url;
    return InkWell(
      onTap: onSelect,
      child: Container(
        constraints: const BoxConstraints(maxWidth: 220),
        padding: EdgeInsets.symmetric(
          horizontal: tokens.dim('spacing', 'sm', fallback: 8),
        ),
        decoration: BoxDecoration(
          color: active ? tokens.color('surface', 'raised') : null,
          border: Border(
            bottom: BorderSide(
              color: active
                  ? tokens.color('accent', 'base')
                  : Colors.transparent,
              width: 2,
            ),
          ),
        ),
        child: Row(
          children: [
            if (tab.loading)
              const SizedBox(
                width: 12,
                height: 12,
                child: CircularProgressIndicator(strokeWidth: 2),
              )
            else
              const Icon(Icons.public, size: 14),
            const SizedBox(width: 6),
            Expanded(
              child: Text(
                label.isEmpty ? 'Nouvel onglet' : label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            InkWell(
              onTap: onClose,
              child: const Icon(Icons.close, size: 14),
            ),
          ],
        ),
      ),
    );
  }
}

/// Rangée de navigation : back/forward/reload + omnibox.
class _NavBar extends StatelessWidget {
  const _NavBar({
    required this.tokens,
    required this.tab,
    required this.enabled,
    required this.omnibox,
    required this.ctrl,
  });

  final DesignTokens tokens;
  final dynamic tab;
  final bool enabled;
  final TextEditingController omnibox;
  final BrowserController ctrl;

  @override
  Widget build(BuildContext context) {
    final pad = tokens.dim('spacing', 'sm', fallback: 8);
    return Padding(
      padding: EdgeInsets.all(pad),
      child: Row(
        children: [
          IconButton(
            icon: const Icon(Icons.arrow_back),
            tooltip: 'Précédent',
            onPressed:
                enabled && tab?.canBack == true ? ctrl.goBack : null,
          ),
          IconButton(
            icon: const Icon(Icons.arrow_forward),
            tooltip: 'Suivant',
            onPressed:
                enabled && tab?.canForward == true ? ctrl.goForward : null,
          ),
          IconButton(
            icon: const Icon(Icons.refresh),
            tooltip: 'Recharger',
            onPressed: enabled ? ctrl.reload : null,
          ),
          Expanded(
            child: TextField(
              controller: omnibox,
              enabled: enabled,
              decoration: const InputDecoration(
                hintText: 'Rechercher ou saisir une URL',
                prefixIcon: Icon(Icons.search),
                isDense: true,
              ),
              textInputAction: TextInputAction.go,
              onSubmitted: enabled ? ctrl.navigate : null,
            ),
          ),
        ],
      ),
    );
  }
}

/// Zone de contenu — placeholder en attendant la frame PNG
/// (étape 30) ; reflète déjà `frameTick` pour observer le
/// cycle d'invalidation.
/// Viewport moteur : `Image.memory` du PNG tiré sur
/// `frame_invalidated` (le SSE ne pousse qu'un signal —
/// ADR-0004). La molette envoie `scroll`, le resize envoie
/// `viewport` (debounced — le drag de fenêtre est continu).
class _FrameView extends ConsumerStatefulWidget {
  const _FrameView({
    required this.tokens,
    required this.state,
    required this.ctrl,
  });

  final DesignTokens tokens;
  final BrowserState state;
  final BrowserController ctrl;

  @override
  ConsumerState<_FrameView> createState() => _FrameViewState();
}

class _FrameViewState extends ConsumerState<_FrameView> {
  Timer? _resizeDebounce;
  Size? _lastSent;

  @override
  void dispose() {
    _resizeDebounce?.cancel();
    super.dispose();
  }

  void _onResize(Size size, double dpr) {
    // Identique → rien (le debounce ne déduppe pas les tailles).
    if (size == _lastSent && _resizeDebounce == null) return;
    _resizeDebounce?.cancel();
    _resizeDebounce = Timer(
      const Duration(milliseconds: 150),
      () {
        _resizeDebounce = null;
        _lastSent = size;
        widget.ctrl.setViewport(size.width, size.height, dpr);
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.tokens;
    final active = widget.state.active;
    final bytes = active == null ? null : widget.state.frames[active.id];
    final muted = t.color('onSurface', 'muted');
    return Container(
      margin: EdgeInsets.all(t.dim('spacing', 'lg', fallback: 24)),
      decoration: BoxDecoration(
        color: t.color('surface', 'raised'),
        borderRadius:
            BorderRadius.circular(t.dim('radius', 'lg', fallback: 14)),
      ),
      clipBehavior: Clip.hardEdge,
      child: bytes == null
          ? _empty(t, active, muted)
          : LayoutBuilder(
              builder: (context, c) {
                final size = Size(c.maxWidth, c.maxHeight);
                final dpr = MediaQuery.devicePixelRatioOf(context);
                WidgetsBinding.instance.addPostFrameCallback(
                  (_) => _onResize(size, dpr),
                );
                return Listener(
                  onPointerSignal: (ev) {
                    if (ev is PointerScrollEvent) {
                      widget.ctrl.scroll(ev.scrollDelta.dy);
                    }
                  },
                  onPointerHover: (ev) => widget.ctrl
                      .hover(ev.localPosition.dx, ev.localPosition.dy),
                  child: GestureDetector(
                    onTapUp: (d) => widget.ctrl
                        .click(d.localPosition.dx, d.localPosition.dy),
                    child: Align(
                      alignment: Alignment.topLeft,
                      child: Image.memory(
                        Uint8List.fromList(bytes),
                        gaplessPlayback: true, // pas de flash entre frames
                        fit: BoxFit.none, // 1 px CSS = 1 pixel logique
                      ),
                    ),
                  ),
                );
              },
            ),
    );
  }

  Widget _empty(DesignTokens t, BrowserTab? active, Color muted) => Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (active?.loading == true)
              const Padding(
                padding: EdgeInsets.only(bottom: 12),
                child: CircularProgressIndicator(),
              ),
            Text(
              active?.url.isNotEmpty == true
                  ? active!.url
                  : 'Naviguez vers une URL',
              style: TextStyle(color: muted),
            ),
          ],
        ),
      );
}
