// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/design/design_tokens.dart';
import '../../settings/presentation/settings_dialog.dart';
import '../domain/browser_tab.dart';
import 'browser_controller.dart';
import 'library_panels.dart';

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
            if (ref.watch(findProvider).visible)
              _FindBar(tokens: t, enabled: st.daemonOnline),
            Expanded(
              child: active == null
                  ? Center(
                      child: Text(
                        'Aucun onglet',
                        style: TextStyle(color: muted),
                      ),
                    )
                  : st.pageErrors[active.id] != null
                  ? _ErrorPage(
                      tokens: t,
                      url: active.url,
                      message: st.pageErrors[active.id]!,
                      onRetry: () => ctrl.navigate(active.url),
                    )
                  : _FrameView(tokens: t, state: st, ctrl: ctrl),
            ),
            _StatusBar(tokens: t, state: st),
          ],
        ),
      ),
    );
  }
}

/// Page d'erreur stylée (étape 33) — échec réseau/TLS :
/// pas de frame moteur, l'UI rend l'échec elle-même avec
/// les tokens (icône danger, message, réessayer).
class _ErrorPage extends StatelessWidget {
  const _ErrorPage({
    required this.tokens,
    required this.url,
    required this.message,
    required this.onRetry,
  });

  final DesignTokens tokens;
  final String url;
  final String message;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final muted = tokens.color('onSurface', 'muted');
    return Container(
      margin: EdgeInsets.all(tokens.dim('spacing', 'lg', fallback: 24)),
      decoration: BoxDecoration(
        color: tokens.color('surface', 'raised'),
        borderRadius: BorderRadius.circular(
          tokens.dim('radius', 'lg', fallback: 14),
        ),
        border: Border.all(color: tokens.color('border', 'base')),
      ),
      child: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              Icons.error_outline,
              size: tokens.dim('metrics', 'iconXl', fallback: 64),
              color: tokens.color('danger', 'base'),
            ),
            SizedBox(height: tokens.dim('spacing', 'md', fallback: 16)),
            Text(
              'Impossible de charger la page',
              style: TextStyle(
                color: tokens.color('onSurface', 'base'),
                fontSize: tokens.dim2('typography', 'size', 'lg', fallback: 18),
              ),
            ),
            SizedBox(height: tokens.dim('spacing', 'sm', fallback: 8)),
            Text(
              url,
              style: TextStyle(
                color: muted,
                fontSize: tokens.dim2('typography', 'size', 'sm', fallback: 12),
              ),
            ),
            SizedBox(height: tokens.dim('spacing', 'xs', fallback: 4)),
            Text(
              message,
              style: TextStyle(
                color: muted,
                fontSize: tokens.dim2('typography', 'size', 'sm', fallback: 12),
              ),
            ),
            SizedBox(height: tokens.dim('spacing', 'md', fallback: 16)),
            TextButton.icon(
              onPressed: onRetry,
              icon: const Icon(Icons.refresh),
              label: const Text('Réessayer'),
            ),
          ],
        ),
      ),
    );
  }
}

/// Barre de statut : `href` survolé (curseur lien) ou
/// indicateur de champ focalisé — hauteur `metrics.
/// statusBarHeight`, invisible quand rien à dire.
class _StatusBar extends StatelessWidget {
  const _StatusBar({required this.tokens, required this.state});

  final DesignTokens tokens;
  final BrowserState state;

  @override
  Widget build(BuildContext context) {
    final label =
        state.hoverUrl ??
        (state.fieldFocused ? 'Champ actif — saisie envoyée au moteur' : '');
    if (label.isEmpty) return const SizedBox.shrink();
    return Container(
      height: tokens.dim('metrics', 'statusBarHeight', fallback: 22),
      padding: EdgeInsets.symmetric(
        horizontal: tokens.dim('spacing', 'sm', fallback: 8),
      ),
      decoration: BoxDecoration(
        color: tokens.color('surface', 'raised'),
        border: Border(top: BorderSide(color: tokens.color('border', 'base'))),
      ),
      alignment: Alignment.centerLeft,
      child: Text(
        label,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(
          color: tokens.color('onSurface', 'muted'),
          fontSize: tokens.dim2('typography', 'size', 'sm', fallback: 12),
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
  const _TabStrip({
    required this.tokens,
    required this.state,
    required this.ctrl,
  });

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
        constraints: BoxConstraints(
          maxWidth: tokens.dim('metrics', 'tabMaxWidth', fallback: 220),
        ),
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
              width: tokens.dim('metrics', 'tabIndicatorWidth', fallback: 2),
            ),
          ),
        ),
        child: Row(
          children: [
            if (tab.loading)
              SizedBox(
                width: tokens.dim('metrics', 'progressSize', fallback: 12),
                height: tokens.dim('metrics', 'progressSize', fallback: 12),
                child: const CircularProgressIndicator(),
              )
            else
              Icon(
                Icons.public,
                size: tokens.dim('metrics', 'iconSm', fallback: 14),
              ),
            SizedBox(width: tokens.dim('spacing', 'xs', fallback: 4)),
            Expanded(
              child: Text(
                label.isEmpty ? 'Nouvel onglet' : label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            InkWell(
              onTap: onClose,
              child: Icon(
                Icons.close,
                size: tokens.dim('metrics', 'iconSm', fallback: 14),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Rangée de navigation : back/forward/reload + omnibox +
/// étoile signet, bibliothèque (historique/signets), zoom et
/// réglages (étape 57).
/// Barre find-in-page (étape 57bis) — Ctrl+F l'ouvre,
/// Entrée cherche/passe à la suivante, Maj+Entrée la
/// précédente, Échap ferme. Le surlignage est peint côté
/// moteur ; la barre ne transporte que la requête.
class _FindBar extends ConsumerStatefulWidget {
  const _FindBar({required this.tokens, required this.enabled});

  final DesignTokens tokens;
  final bool enabled;

  @override
  ConsumerState<_FindBar> createState() => _FindBarState();
}

class _FindBarState extends ConsumerState<_FindBar> {
  final _field = TextEditingController();
  final _focus = FocusNode();
  String _submitted = '';

  @override
  void initState() {
    super.initState();
    _focus.requestFocus();
  }

  @override
  void dispose() {
    _field.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.tokens;
    final st = ref.watch(findProvider);
    final ctrl = ref.read(findProvider.notifier);
    final muted = t.color('onSurface', 'muted');
    final canStep = widget.enabled && !st.busy && st.count > 0;

    return CallbackShortcuts(
      // Le TextField a son propre focus — Échap le ferme
      // depuis l'intérieur de la barre (étape 57bis).
      bindings: {const SingleActivator(LogicalKeyboardKey.escape): ctrl.close},
      child: Container(
        padding: EdgeInsets.symmetric(
          horizontal: t.dim('spacing', 'md', fallback: 16),
          vertical: t.dim('spacing', 'xs', fallback: 8),
        ),
        color: t.color('surface', 'raised'),
        child: Row(
          children: [
            Icon(Icons.search, size: 18, color: muted),
            SizedBox(width: t.dim('spacing', 'sm', fallback: 8)),
            SizedBox(
              width: 220,
              child: TextField(
                controller: _field,
                focusNode: _focus,
                enabled: widget.enabled,
                style: TextStyle(
                  color: t.color('onSurface', 'base'),
                  fontSize: 13,
                ),
                decoration: InputDecoration(
                  hintText: 'Rechercher dans la page',
                  hintStyle: TextStyle(color: muted, fontSize: 13),
                  isDense: true,
                  border: InputBorder.none,
                ),
                onSubmitted: (q) {
                  // Entrée : nouvelle requête, ou occurrence
                  // suivante si la requête n'a pas changé.
                  if (q == _submitted && st.count > 0) {
                    ctrl.step(1);
                  } else {
                    _submitted = q;
                    ctrl.submit(q);
                  }
                },
              ),
            ),
            Text(
              st.busy
                  ? '…'
                  : st.query.isEmpty
                  ? ''
                  : '${st.index + (st.count > 0 ? 1 : 0)}/${st.count}',
              style: TextStyle(color: muted, fontSize: 12),
            ),
            IconButton(
              icon: const Icon(Icons.keyboard_arrow_up, size: 20),
              color: canStep ? t.color('onSurface', 'base') : muted,
              onPressed: canStep ? () => ctrl.step(-1) : null,
              tooltip: 'Précédent',
              visualDensity: VisualDensity.compact,
            ),
            IconButton(
              icon: const Icon(Icons.keyboard_arrow_down, size: 20),
              color: canStep ? t.color('onSurface', 'base') : muted,
              onPressed: canStep ? () => ctrl.step(1) : null,
              tooltip: 'Suivant',
              visualDensity: VisualDensity.compact,
            ),
            const Spacer(),
            IconButton(
              icon: const Icon(Icons.close, size: 18),
              color: muted,
              onPressed: () => ctrl.close(),
              tooltip: 'Fermer (Échap)',
              visualDensity: VisualDensity.compact,
            ),
          ],
        ),
      ),
    );
  }
}

class _NavBar extends ConsumerWidget {
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
  Widget build(BuildContext context, WidgetRef ref) {
    final pad = tokens.dim('spacing', 'sm', fallback: 8);
    final url = tab?.url as String? ?? '';
    final markable = enabled && url.isNotEmpty && url != 'about:blank';
    final marked =
        ref.watch(bookmarksProvider).value?.any((b) => b.url == url) == true;
    final zoom = ref.watch(zoomProvider);
    return Padding(
      padding: EdgeInsets.all(pad),
      child: Row(
        children: [
          IconButton(
            icon: const Icon(Icons.arrow_back),
            tooltip: 'Précédent',
            onPressed: enabled && tab?.canBack == true ? ctrl.goBack : null,
          ),
          IconButton(
            icon: const Icon(Icons.arrow_forward),
            tooltip: 'Suivant',
            onPressed: enabled && tab?.canForward == true
                ? ctrl.goForward
                : null,
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
          IconButton(
            icon: Icon(marked ? Icons.star : Icons.star_border),
            tooltip: marked ? 'Retirer le signet' : 'Ajouter le signet',
            onPressed: markable
                ? () => ref
                      .read(bookmarksProvider.notifier)
                      .toggle(url, title: tab?.title as String? ?? '')
                : null,
          ),
          IconButton(
            icon: const Icon(Icons.history),
            tooltip: 'Historique',
            onPressed: enabled
                ? () => LibraryPanel.showHistory(context, tokens)
                : null,
          ),
          IconButton(
            icon: const Icon(Icons.bookmarks_outlined),
            tooltip: 'Signets',
            onPressed: enabled
                ? () => LibraryPanel.showBookmarks(context, tokens)
                : null,
          ),
          IconButton(
            icon: const Icon(Icons.find_in_page_outlined),
            tooltip: 'Rechercher dans la page (Ctrl+F)',
            onPressed: enabled
                ? () => ref.read(findProvider.notifier).open()
                : null,
          ),
          PopupMenuButton<String>(
            icon: const Icon(Icons.zoom_in),
            tooltip: 'Zoom ${(zoom * 100).round()} %',
            enabled: enabled,
            onSelected: (v) {
              final z = ref.read(zoomProvider.notifier);
              switch (v) {
                case 'in':
                  z.zoomIn();
                case 'out':
                  z.zoomOut();
                case 'reset':
                  z.reset();
              }
            },
            itemBuilder: (_) => [
              const PopupMenuItem(value: 'in', child: Text('Zoom avant (+)')),
              const PopupMenuItem(
                value: 'out',
                child: Text('Zoom arrière (−)'),
              ),
              const PopupMenuItem(value: 'reset', child: Text('Taille réelle')),
            ],
          ),
          IconButton(
            icon: const Icon(Icons.settings_outlined),
            tooltip: 'Réglages',
            onPressed: () => SettingsDialog.show(context, tokens),
          ),
        ],
      ),
    );
  }
}

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
  double _lastScale = 0;
  final FocusNode _keys = FocusNode();

  @override
  void dispose() {
    _resizeDebounce?.cancel();
    _keys.dispose();
    super.dispose();
  }

  /// Envoie le viewport moteur en unités **CSS** : `w/zoom` —
  /// l'étape 57 implémente le zoom côté UI via `scale = zoom×dpr`
  /// (layout refondu à `w/z`, raster net à `w×dpr` px — ADR-0021).
  void _onResize(Size size, double zoom, double dpr) {
    final scale = zoom * dpr;
    // Identique → rien (le debounce ne déduppe pas les tailles).
    if (size == _lastSent && scale == _lastScale && _resizeDebounce == null) {
      return;
    }
    _resizeDebounce?.cancel();
    _resizeDebounce = Timer(const Duration(milliseconds: 150), () {
      _resizeDebounce = null;
      _lastSent = size;
      _lastScale = scale;
      widget.ctrl.setViewport(size.width / zoom, size.height / zoom, scale);
    });
  }

  /// Raccourcis chrome — zoom Ctrl+`+`/`-`/`0` (étape 57) et
  /// find-in-page Ctrl+F / Échap (étape 57bis) : actifs même
  /// quand un champ moteur a le focus (jamais relayés au moteur).
  bool _chromeKey(LogicalKeyboardKey key) {
    final kb = HardwareKeyboard.instance;
    final ctrl =
        kb.isLogicalKeyPressed(LogicalKeyboardKey.controlLeft) ||
        kb.isLogicalKeyPressed(LogicalKeyboardKey.controlRight);
    if (ctrl) {
      final z = ref.read(zoomProvider.notifier);
      switch (key) {
        case LogicalKeyboardKey.equal ||
            LogicalKeyboardKey.add ||
            LogicalKeyboardKey.numpadAdd:
          z.zoomIn();
        case LogicalKeyboardKey.minus || LogicalKeyboardKey.numpadSubtract:
          z.zoomOut();
        case LogicalKeyboardKey.digit0 || LogicalKeyboardKey.numpad0:
          z.reset();
        case LogicalKeyboardKey.keyF:
          ref.read(findProvider.notifier).open();
        default:
          return false;
      }
      return true;
    }
    // Échap ferme la barre de recherche si elle est ouverte —
    // sinon la touche suit son chemin normal vers le moteur.
    if (key == LogicalKeyboardKey.escape && ref.read(findProvider).visible) {
      ref.read(findProvider.notifier).close();
      return true;
    }
    return false;
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.tokens;
    final zoom = ref.watch(zoomProvider);
    final active = widget.state.active;
    final bytes = active == null ? null : widget.state.frames[active.id];
    final raw = active == null ? null : widget.state.rawFrames[active.id];
    final muted = t.color('onSurface', 'muted');
    return Container(
      margin: EdgeInsets.all(t.dim('spacing', 'lg', fallback: 24)),
      decoration: BoxDecoration(
        color: t.color('surface', 'raised'),
        borderRadius: BorderRadius.circular(
          t.dim('radius', 'lg', fallback: 14),
        ),
      ),
      clipBehavior: Clip.hardEdge,
      child: bytes == null && raw == null
          ? _empty(t, active, muted)
          : LayoutBuilder(
              builder: (context, c) {
                final size = Size(c.maxWidth, c.maxHeight);
                final dpr = MediaQuery.devicePixelRatioOf(context);
                WidgetsBinding.instance.addPostFrameCallback(
                  (_) => _onResize(size, zoom, dpr),
                );
                final st = widget.state;
                // 1 px CSS moteur = `zoom·dpr` px logiques affichés
                // (raster brut `fit:none`) → les coordonnées
                // d'entrée se divisent par ce facteur.
                final cssPerLogical = zoom * dpr;
                return MouseRegion(
                  cursor: st.hoverEditable
                      ? SystemMouseCursors.text
                      : st.hoverUrl != null
                      ? SystemMouseCursors.click
                      : SystemMouseCursors.basic,
                  child: Listener(
                    onPointerSignal: (ev) {
                      if (ev is PointerScrollEvent) {
                        // dy en px logiques → px CSS moteur.
                        widget.ctrl.scroll(ev.scrollDelta.dy / cssPerLogical);
                      }
                    },
                    onPointerHover: (ev) => widget.ctrl.hover(
                      ev.localPosition.dx / cssPerLogical,
                      ev.localPosition.dy / cssPerLogical,
                    ),
                    child: GestureDetector(
                      onTapUp: (d) => widget.ctrl.click(
                        d.localPosition.dx / cssPerLogical,
                        d.localPosition.dy / cssPerLogical,
                      ),
                      child: KeyboardListener(
                        focusNode: _keys,
                        autofocus: true,
                        onKeyEvent: (e) {
                          if (e is! KeyDownEvent) return;
                          if (_chromeKey(e.logicalKey)) return;
                          if (!st.fieldFocused) return;
                          // keyLabel couvre lettres et touches
                          // nommées (« Backspace », « Enter »).
                          widget.ctrl.key(e.logicalKey.keyLabel);
                        },
                        child: Align(
                          alignment: Alignment.topLeft,
                          // Canal FFI (ADR-0018) : `ui.Image`
                          // brute, sinon PNG mémoire (ADR-0004).
                          child: raw != null
                              ? RawImage(image: raw, fit: BoxFit.none)
                              : Image.memory(
                                  Uint8List.fromList(bytes!),
                                  gaplessPlayback:
                                      true, // pas de flash entre frames
                                  fit:
                                      BoxFit.none, // 1 px CSS = 1 pixel logique
                                ),
                        ),
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
          Padding(
            padding: EdgeInsets.only(
              bottom: t.dim('spacing', 'sm', fallback: 8),
            ),
            child: const CircularProgressIndicator(),
          )
        else
          Icon(
            Icons.public,
            size: t.dim('metrics', 'iconXl', fallback: 64),
            color: muted,
          ),
        SizedBox(height: t.dim('spacing', 'sm', fallback: 8)),
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
