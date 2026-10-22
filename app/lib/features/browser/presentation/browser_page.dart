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
import '../domain/media_offer.dart';
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
  final _omniboxFocus = FocusNode();

  @override
  void dispose() {
    _omnibox.dispose();
    _omniboxFocus.dispose();
    super.dispose();
  }

  /// Ctrl+L — focus omnibox + sélection intégrale (étape 85,
  /// geste chrome) : la frappe suivante remplace l'URL.
  void _focusOmnibox() {
    _omniboxFocus.requestFocus();
    _omnibox.selection = TextSelection(
      baseOffset: 0,
      extentOffset: _omnibox.text.length,
    );
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
      if (!mounted) return;
      final messenger = ScaffoldMessenger.of(context);
      if (msg != null) {
        messenger.showSnackBar(SnackBar(content: Text(msg)));
      } else {
        // Purge (étape 88) : l'erreur est résolue (ex.
        // reconnexion réussie) — le snackbar encore visible
        // disparaît au lieu de survivre à son sujet.
        messenger.hideCurrentSnackBar();
      }
    });
    // Téléchargement proposé (étape 70) : « Enregistrer »
    // écrit sous le répertoire du daemon, « Ouvrir comme
    // texte » rejoue le corps comme document.
    ref.listen(browserProvider.select((s) => s.downloadOffer), (_, offer) {
      if (offer == null || !mounted) return;
      final size = offer.size == null
          ? ''
          : ' — ${(offer.size! / 1024).toStringAsFixed(1)} Kio';
      showDialog<void>(
        context: context,
        builder: (ctx) => AlertDialog(
          title: const Text('Téléchargement'),
          content: Text(
            '${offer.filename}$size\n${offer.url}\n\n'
            'Ce fichier n’est pas affichable dans le navigateur.',
          ),
          actions: [
            TextButton(
              onPressed: () {
                Navigator.of(ctx).pop();
                ctrl.declineDownload(offer);
              },
              child: const Text('Ouvrir comme texte'),
            ),
            FilledButton(
              onPressed: () {
                Navigator.of(ctx).pop();
                ctrl.acceptDownload(offer);
              },
              child: const Text('Enregistrer'),
            ),
          ],
        ),
      );
    });
    // Resoumission POST (étape 93) : le reload demande
    // l'accord explicite — « Renvoyer » rejoue la requête
    // d'origine, « Annuler » n'émet rien.
    ref.listen(browserProvider.select((s) => s.resubmit), (_, pending) {
      if (pending == null || !mounted) return;
      showDialog<void>(
        context: context,
        builder: (ctx) => AlertDialog(
          title: const Text('Renvoyer le formulaire ?'),
          content: Text(
            'La page a été obtenue par ${pending.method}.\n'
            '${pending.url}\n\n'
            'Recharger renverra les données — un effet de bord\n'
            '(commande, paiement) peut être répété.',
          ),
          actions: [
            TextButton(
              onPressed: () {
                Navigator.of(ctx).pop();
                ctrl.dismissResubmit();
              },
              child: const Text('Annuler'),
            ),
            FilledButton(
              onPressed: () {
                Navigator.of(ctx).pop();
                ctrl.reloadConfirmed();
              },
              child: const Text('Renvoyer'),
            ),
          ],
        ),
      );
    });

    final active = st.active;
    final muted = t.color('onSurface', 'muted');

    return Scaffold(
      backgroundColor: t.color('surface', 'base'),
      body: SafeArea(
        // Raccourcis chrome au niveau PAGE (étape 85) : ils
        // doivent répondre aussi quand l'omnibox ou la barre
        // de recherche a le focus — un binding dans _FrameView
        // n'existait que sous focus frame. Les combos y étaient
        // migrées (zoom, find, Échap) pour un seul lieu de vérité.
        child: CallbackShortcuts(
          bindings: {
            const SingleActivator(LogicalKeyboardKey.keyL, control: true):
                _focusOmnibox,
            const SingleActivator(LogicalKeyboardKey.keyT, control: true):
                ctrl.newTab,
            const SingleActivator(LogicalKeyboardKey.keyW, control: true): () {
              final id = st.activeId;
              if (id != null) unawaited(ctrl.closeTab(id));
            },
            const SingleActivator(LogicalKeyboardKey.keyR, control: true):
                ctrl.reload,
            const SingleActivator(LogicalKeyboardKey.f5): ctrl.reload,
            const SingleActivator(LogicalKeyboardKey.arrowLeft, alt: true):
                ctrl.goBack,
            const SingleActivator(LogicalKeyboardKey.arrowRight, alt: true):
                ctrl.goForward,
            const SingleActivator(LogicalKeyboardKey.browserBack): ctrl.goBack,
            const SingleActivator(LogicalKeyboardKey.browserForward):
                ctrl.goForward,
            const SingleActivator(LogicalKeyboardKey.tab, control: true): () =>
                ctrl.cycleTab(1),
            const SingleActivator(
              LogicalKeyboardKey.tab,
              control: true,
              shift: true,
            ): () =>
                ctrl.cycleTab(-1),
            // Ctrl+1..8 → l'onglet n ; Ctrl+9 → le dernier.
            for (var i = 0; i < _tabDigitKeys.length; i++)
              SingleActivator(_tabDigitKeys[i], control: true): () =>
                  ctrl.selectTabAt(i),
            const SingleActivator(
              LogicalKeyboardKey.digit9,
              control: true,
            ): () {
              final n = ref.read(browserProvider).tabs.length;
              if (n > 0) ctrl.selectTabAt(n - 1);
            },
            const SingleActivator(
              LogicalKeyboardKey.equal,
              control: true,
            ): () =>
                ref.read(zoomProvider.notifier).zoomIn(),
            const SingleActivator(LogicalKeyboardKey.add, control: true): () =>
                ref.read(zoomProvider.notifier).zoomIn(),
            const SingleActivator(
              LogicalKeyboardKey.numpadAdd,
              control: true,
            ): () =>
                ref.read(zoomProvider.notifier).zoomIn(),
            const SingleActivator(
              LogicalKeyboardKey.minus,
              control: true,
            ): () =>
                ref.read(zoomProvider.notifier).zoomOut(),
            const SingleActivator(
              LogicalKeyboardKey.numpadSubtract,
              control: true,
            ): () =>
                ref.read(zoomProvider.notifier).zoomOut(),
            const SingleActivator(
              LogicalKeyboardKey.digit0,
              control: true,
            ): () =>
                ref.read(zoomProvider.notifier).reset(),
            const SingleActivator(
              LogicalKeyboardKey.numpad0,
              control: true,
            ): () =>
                ref.read(zoomProvider.notifier).reset(),
            const SingleActivator(LogicalKeyboardKey.keyF, control: true): () =>
                ref.read(findProvider.notifier).open(),
            const SingleActivator(LogicalKeyboardKey.escape): () {
              if (ref.read(findProvider).visible) {
                ref.read(findProvider.notifier).close();
              }
            },
          },
          child: Column(
            children: [
              _TabStrip(tokens: t, state: st, ctrl: ctrl),
              _NavBar(
                tokens: t,
                tab: active,
                enabled: st.daemonOnline && active != null,
                omnibox: _omnibox,
                omniboxFocus: _omniboxFocus,
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
      ),
    );
  }
}

/// Déclencheurs Ctrl+1..8 — index d'onglet direct (étape 85).
const _tabDigitKeys = [
  LogicalKeyboardKey.digit1,
  LogicalKeyboardKey.digit2,
  LogicalKeyboardKey.digit3,
  LogicalKeyboardKey.digit4,
  LogicalKeyboardKey.digit5,
  LogicalKeyboardKey.digit6,
  LogicalKeyboardKey.digit7,
  LogicalKeyboardKey.digit8,
];

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
    // Clic-molette sur l'onglet = fermer (étape 85, geste
    // chrome) — le primaire sélectionne toujours via InkWell.
    return Listener(
      onPointerDown: (ev) {
        if (ev.buttons == kMiddleMouseButton) onClose();
      },
      child: InkWell(
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
    required this.omniboxFocus,
    required this.ctrl,
  });

  final DesignTokens tokens;
  final dynamic tab;
  final bool enabled;
  final TextEditingController omnibox;
  final FocusNode omniboxFocus;
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
      // Étroit → les actions trailing se replient dans un
      // menu « ⋮ » plutôt que de déborder hors de la frame
      // (étape 80 — un Row fixe peint l'overflow en silence
      // en release ; Chrome fait pareil sous ~500 px).
      child: LayoutBuilder(
        builder: (context, c) {
          final compact =
              c.maxWidth < tokens.dim('metrics', 'navCompact', fallback: 560);
          return compact
              ? _compactBar(context, ref, url, markable, marked)
              : _wideBar(context, ref, url, markable, marked, zoom);
        },
      ),
    );
  }

  /// Disposition complète — toutes les icônes visibles.
  Widget _wideBar(
    BuildContext context,
    WidgetRef ref,
    String url,
    bool markable,
    bool marked,
    double zoom,
  ) {
    return Row(
      children: [
        IconButton(
          icon: const Icon(Icons.arrow_back),
          tooltip: 'Précédent',
          onPressed: enabled && tab?.canBack == true ? ctrl.goBack : null,
        ),
        IconButton(
          icon: const Icon(Icons.arrow_forward),
          tooltip: 'Suivant',
          onPressed: enabled && tab?.canForward == true ? ctrl.goForward : null,
        ),
        IconButton(
          icon: const Icon(Icons.refresh),
          tooltip: 'Recharger',
          onPressed: enabled ? ctrl.reload : null,
        ),
        // « Home » = page vide `about:blank` dans l'onglet
        // courant (étape 88) — pas d'accueil configurable, la
        // convention est la page blanche du nouvel onglet.
        IconButton(
          icon: const Icon(Icons.home_outlined),
          tooltip: 'Accueil',
          onPressed: enabled ? () => ctrl.navigate('about:blank') : null,
        ),
        Expanded(
          child: TextField(
            controller: omnibox,
            focusNode: omniboxFocus,
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
            const PopupMenuItem(value: 'out', child: Text('Zoom arrière (−)')),
            const PopupMenuItem(value: 'reset', child: Text('Taille réelle')),
          ],
        ),
        IconButton(
          icon: const Icon(Icons.settings_outlined),
          tooltip: 'Réglages',
          onPressed: () => SettingsDialog.show(context, tokens),
        ),
      ],
    );
  }

  /// Disposition étroite — navigation + omnibox + menu « ⋮ »
  /// qui regroupe les actions trailing (étape 80).
  Widget _compactBar(
    BuildContext context,
    WidgetRef ref,
    String url,
    bool markable,
    bool marked,
  ) {
    PopupMenuItem<String> item(
      String value,
      IconData icon,
      String label, {
      bool enabled = true,
    }) => PopupMenuItem(
      value: value,
      enabled: enabled,
      child: Row(
        children: [
          Icon(icon, size: 18),
          const SizedBox(width: 8),
          Expanded(child: Text(label, overflow: TextOverflow.ellipsis)),
        ],
      ),
    );

    return Row(
      children: [
        IconButton(
          icon: const Icon(Icons.arrow_back),
          tooltip: 'Précédent',
          onPressed: enabled && tab?.canBack == true ? ctrl.goBack : null,
        ),
        IconButton(
          icon: const Icon(Icons.arrow_forward),
          tooltip: 'Suivant',
          onPressed: enabled && tab?.canForward == true ? ctrl.goForward : null,
        ),
        IconButton(
          icon: const Icon(Icons.refresh),
          tooltip: 'Recharger',
          onPressed: enabled ? ctrl.reload : null,
        ),
        Expanded(
          child: TextField(
            controller: omnibox,
            focusNode: omniboxFocus,
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
        PopupMenuButton<String>(
          icon: const Icon(Icons.more_vert),
          tooltip: 'Plus d’actions',
          onSelected: (v) {
            switch (v) {
              case 'mark':
                unawaited(
                  ref
                      .read(bookmarksProvider.notifier)
                      .toggle(url, title: tab?.title as String? ?? ''),
                );
              case 'history':
                LibraryPanel.showHistory(context, tokens);
              case 'marks':
                LibraryPanel.showBookmarks(context, tokens);
              case 'home':
                unawaited(ctrl.navigate('about:blank'));
              case 'find':
                ref.read(findProvider.notifier).open();
              case 'zoom-in':
                ref.read(zoomProvider.notifier).zoomIn();
              case 'zoom-out':
                ref.read(zoomProvider.notifier).zoomOut();
              case 'zoom-reset':
                ref.read(zoomProvider.notifier).reset();
              case 'settings':
                SettingsDialog.show(context, tokens);
            }
          },
          itemBuilder: (_) => [
            item('home', Icons.home_outlined, 'Accueil', enabled: enabled),
            item(
              'mark',
              marked ? Icons.star : Icons.star_border,
              marked ? 'Retirer le signet' : 'Ajouter le signet',
              enabled: markable,
            ),
            item('history', Icons.history, 'Historique', enabled: enabled),
            item(
              'marks',
              Icons.bookmarks_outlined,
              'Signets',
              enabled: enabled,
            ),
            item(
              'find',
              Icons.find_in_page_outlined,
              'Rechercher dans la page',
              enabled: enabled,
            ),
            const PopupMenuDivider(),
            item('zoom-in', Icons.zoom_in, 'Zoom avant', enabled: enabled),
            item('zoom-out', Icons.zoom_out, 'Zoom arrière', enabled: enabled),
            item(
              'zoom-reset',
              Icons.zoom_out_map,
              'Taille réelle',
              enabled: enabled,
            ),
            const PopupMenuDivider(),
            item('settings', Icons.settings_outlined, 'Réglages'),
          ],
        ),
      ],
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

  /// Une frappe ne rejoint le champ moteur que **nue** —
  /// sans Ctrl ni Alt : les combos modifiés sont des gestes
  /// chrome (CallbackShortcuts de page, étape 85) ou
  /// d'édition, jamais du texte à taper (Ctrl+V envoyait 'v'
  /// au champ avant ce garde). **Exception `AltGr`** : Windows
  /// le synthétise `Ctrl+Alt` alors que c'est une corde de
  /// texte AZERTY (`@ € { } [ ] ~`) — sans elle, plus aucune
  /// saisie européenne n'atteint le moteur. Échap circule
  /// librement : il ferme la barre find en page et atteint
  /// le moteur.
  bool _chordHeld() {
    final kb = HardwareKeyboard.instance;
    if (kb.isLogicalKeyPressed(LogicalKeyboardKey.altRight)) {
      return false; // AltGr = texte
    }
    return kb.isLogicalKeyPressed(LogicalKeyboardKey.controlLeft) ||
        kb.isLogicalKeyPressed(LogicalKeyboardKey.controlRight) ||
        kb.isLogicalKeyPressed(LogicalKeyboardKey.altLeft);
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.tokens;
    final zoom = ref.watch(zoomProvider);
    final active = widget.state.active;
    // Frame décodée hors `BrowserState` (fuite corrigée 22/10 :
    // les états obsolètes épinglés par l'arbre retenaient les
    // payloads) — `frameTick` dans l'état pilote le rebuild.
    final raw = active == null ? null : widget.ctrl.frameImage(active.id);
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
      child: raw == null
          ? _empty(t, active, muted)
          : LayoutBuilder(
              builder: (context, c) {
                final size = Size(c.maxWidth, c.maxHeight);
                final dpr = MediaQuery.devicePixelRatioOf(context);
                WidgetsBinding.instance.addPostFrameCallback(
                  (_) => _onResize(size, zoom, dpr),
                );
                final st = widget.state;
                // Le raster produit `zoom·dpr` px par px CSS ; l'image
                // est affichée à `scale: dpr` (1 px image = 1 px
                // physique, nette à tout DPR) → 1 px CSS moteur =
                // `zoom` px logiques à l'écran. Les coordonnées
                // d'entrée se divisent par ce facteur.
                final cssPerLogical = zoom;
                return MouseRegion(
                  cursor: st.hoverEditable
                      ? SystemMouseCursors.text
                      : st.hoverUrl != null
                      ? SystemMouseCursors.click
                      : SystemMouseCursors.basic,
                  child: Listener(
                    onPointerSignal: (ev) {
                      if (ev is PointerScrollEvent) {
                        if (HardwareKeyboard.instance.isControlPressed) {
                          // Ctrl+molette = zoom page — la
                          // convention navigateur (étape 88) ;
                          // molette haut = zoom avant.
                          final z = ref.read(zoomProvider.notifier);
                          if (ev.scrollDelta.dy < 0) {
                            z.zoomIn();
                          } else {
                            z.zoomOut();
                          }
                        } else {
                          // dy en px logiques → px CSS moteur.
                          widget.ctrl.scroll(ev.scrollDelta.dy / cssPerLogical);
                        }
                      }
                    },
                    onPointerDown: (ev) {
                      // Boutons souris de navigation (étape 85) :
                      // X1/X2 = précédent/suivant ; clic-molette
                      // sur un lien = nouvel onglet en
                      // arrière-plan (`hoverUrl` tenu par le
                      // dernier mousemove — même pointeur).
                      switch (ev.buttons) {
                        case kBackMouseButton:
                          widget.ctrl.goBack();
                        case kForwardMouseButton:
                          widget.ctrl.goForward();
                        case kMiddleMouseButton:
                          final url = st.hoverUrl;
                          if (url != null) {
                            unawaited(widget.ctrl.openInNewTab(url));
                          }
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
                          if (_chordHeld()) return;
                          // Échap ferme la barre find (binding
                          // page) — il ne doit pas aussi être
                          // tapé dans le champ moteur focalisé.
                          if (e.logicalKey == LogicalKeyboardKey.escape &&
                              ref.read(findProvider).visible) {
                            return;
                          }
                          if (!st.fieldFocused) return;
                          // `character` = le caractère produit
                          // (accents par touches mortes, AltGr,
                          // shift — `keyLabel` ne voit que la
                          // touche nue) ; les touches nommées
                          // n'en ont pas → repli `keyLabel`
                          // (« Enter », « Backspace »).
                          final ch = e.character;
                          // Ét.105 : état réel des modifieurs
                          // (Shift sur une frappe de texte,
                          // Meta non filtrée par _chordHeld)
                          // → props du KeyboardEvent.
                          final kb = HardwareKeyboard.instance;
                          widget.ctrl.key(
                            ch != null && ch.isNotEmpty
                                ? ch
                                : e.logicalKey.keyLabel,
                            ctrl: kb.isControlPressed,
                            shift: kb.isShiftPressed,
                            alt: kb.isAltPressed,
                            meta: kb.isMetaPressed,
                          );
                        },
                        child: Stack(
                          children: [
                            Align(
                              alignment: Alignment.topLeft,
                              // Traduction locale (étape 109) :
                              // scroll demandé pas encore peint →
                              // la frame retenue glisse de
                              // `-shift` px logiques — retour
                              // visuel immédiat, la bande exposée
                              // attend le pull (fond du conteneur
                              // en attendant, clipé hardEdge).
                              child: Transform.translate(
                                offset: Offset(
                                  0,
                                  -widget.ctrl.frameShiftCss(
                                        widget.state.active?.id ?? '',
                                      ) *
                                      zoom,
                                ),
                                // `ui.Image` décodée côté
                                // contrôleur (REST : codec PNG ;
                                // FFI : pixels bruts ADR-0018) —
                                // `RawImage` direct.
                                child: RawImage(
                                  image: raw,
                                  fit: BoxFit.none,
                                  // 1 px image = 1 px physique :
                                  // taille logique = px/dpr.
                                  scale: dpr,
                                ),
                              ),
                            ),
                            // Overlay MediaSurface (71d) : rects
                            // en px CSS document → position =
                            // (x, y − scroll_y) × zoom, la frame
                            // étant affichée à 1 px CSS = `zoom`
                            // px logiques (étape 82).
                            ..._mediaOverlays(st, active!.id, zoom),
                          ],
                        ),
                      ),
                    ),
                  ),
                );
              },
            ),
    );
  }

  /// Surfaces média claimées positionnées en overlay —
  /// `rect` espace document → viewport : `(x, y − scroll_y)`
  /// en px CSS × `zoom` px logiques (même facteur que
  /// l'entrée, fix étape 82). Audio 0×0 : joue sans overlay.
  /// `controls` → tap = play/pause (UI hôte minimale v1).
  List<Widget> _mediaOverlays(BrowserState st, String tabId, double zoom) {
    final offers = st.media[tabId]?.values;
    if (offers == null || offers.isEmpty) return const [];
    final scrollY = st.scrollY[tabId] ?? 0;
    return [
      for (final m in offers)
        if (m.hasOverlay)
          Positioned(
            left: m.rect.left * zoom,
            top: (m.rect.top - scrollY) * zoom,
            width: m.rect.width * zoom,
            height: m.rect.height * zoom,
            child: _mediaSurface(st, tabId, m),
          ),
    ];
  }

  /// La vue native du lecteur, sur fond noir (letterbox) tant
  /// qu'elle n'existe pas — la surface existe dès l'offre, la
  /// texture arrive après `claim` + init asynchrone.
  Widget _mediaSurface(BrowserState st, String tabId, MediaOffer m) {
    final view = widget.ctrl.mediaView(tabId, m.id);
    Widget child = Container(color: Colors.black, child: view);
    if (m.controls) {
      child = GestureDetector(
        onTap: () => widget.ctrl.mediaToggle(tabId, m.id),
        child: child,
      );
    }
    return child;
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
