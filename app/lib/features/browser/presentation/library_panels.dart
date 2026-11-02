// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/design/design_tokens.dart';
import '../../../core/l10n/app_localizations.dart';
import '../domain/bookmark.dart';
import '../domain/history_entry.dart';
import '../domain/active_download.dart';
import 'browser_controller.dart';

/// Panneau bibliothèque (étape 57) — historique de navigation
/// ou signets du store moteur, en dialog modal. Un tap navigue
/// l'onglet actif vers l'entrée ; les signets offrent une
/// suppression en ligne.
class LibraryPanel extends ConsumerWidget {
  const LibraryPanel._({required this.tokens, required this.history});

  final DesignTokens tokens;

  /// `true` → historique, `false` → signets.
  final bool history;

  static Future<void> showHistory(BuildContext context, DesignTokens tokens) =>
      _show(context, tokens, history: true);

  static Future<void> showBookmarks(
    BuildContext context,
    DesignTokens tokens,
  ) => _show(context, tokens, history: false);

  /// Panneau téléchargements (étape 176) : fiches en vol
  /// (progression SSE + annulation) puis le journal profil
  /// des terminés — `[]` en mode FFI (pas d'ABI lecture).
  static Future<void> showDownloads(
    BuildContext context,
    DesignTokens tokens,
  ) => showDialog<void>(
    context: context,
    builder: (_) => _DownloadsDialog(tokens: tokens),
  );

  static Future<void> _show(
    BuildContext context,
    DesignTokens tokens, {
    required bool history,
  }) => showDialog<void>(
    context: context,
    builder: (_) => LibraryPanel._(tokens: tokens, history: history),
  );

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = tokens;
    final ctrl = ref.read(browserProvider.notifier);

    void open(String url) {
      Navigator.of(context).pop();
      ctrl.navigate(url);
    }

    return AlertDialog(
      backgroundColor: t.color('surface', 'raised'),
      title: Text(history ? AppLocalizations.of(context).history : AppLocalizations.of(context).bookmarks),
      content: SizedBox(
        width: 460,
        height: 380,
        child: history
            ? ref
                  .watch(historyProvider)
                  .when(
                    loading: () =>
                        const Center(child: CircularProgressIndicator()),
                    error: (e, _) => _Message(
                      tokens: t,
                      text: AppLocalizations.of(context).historyUnavailable('$e'),
                    ),
                    data: (entries) =>
                        _HistoryList(tokens: t, entries: entries, onOpen: open),
                  )
            : ref
                  .watch(bookmarksProvider)
                  .when(
                    loading: () =>
                        const Center(child: CircularProgressIndicator()),
                    error: (e, _) =>
                        _Message(tokens: t, text: AppLocalizations.of(context).bookmarksUnavailable('$e')),
                    data: (marks) => _BookmarkList(
                      tokens: t,
                      marks: marks,
                      onOpen: open,
                      onRemove: (url) =>
                          ref.read(bookmarksProvider.notifier).remove(url),
                    ),
                  ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(AppLocalizations.of(context).close),
        ),
      ],
    );
  }
}

/// Dialogue téléchargements — la liste « en vol » vit dans
/// `state.downloads` (rebuild à chaque event SSE/NDJSON),
/// le journal est tiré une fois par `listDownloads`.
class _DownloadsDialog extends ConsumerWidget {
  const _DownloadsDialog({required this.tokens});

  final DesignTokens tokens;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = tokens;
    final l10n = AppLocalizations.of(context);
    final ctrl = ref.read(browserProvider.notifier);
    final actives = ref.watch(
      browserProvider.select((s) => s.downloads),
    );
    return AlertDialog(
      backgroundColor: t.color('surface', 'raised'),
      title: Text(l10n.downloads),
      content: SizedBox(
        width: 460,
        height: 380,
        child: FutureBuilder<List<Map<String, dynamic>>>(
          future: ctrl.listDownloads(),
          builder: (_, snap) {
            final journal = snap.data ?? const <Map<String, dynamic>>[];
            final live = actives.values.toList()
              ..sort((a, b) => a.id.compareTo(b.id));
            if (live.isEmpty && journal.isEmpty) {
              return _Message(tokens: t, text: l10n.emptyDownloads);
            }
            return ListView(
              children: [
                for (final d in live)
                  _ActiveTile(
                    tokens: t,
                    dl: d,
                    onCancel: d.finished
                        ? null
                        : () => ctrl.cancelDownload(d.id),
                  ),
                for (final j in journal.reversed)
                  _EntryTile(
                    tokens: t,
                    title: '${j['filename'] ?? ''}',
                    url: '${j['path'] ?? ''}',
                    onTap: () {},
                  ),
              ],
            );
          },
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l10n.close),
        ),
      ],
    );
  }
}

/// Fiche d'un téléchargement en vol — barre de progression
/// bornée (`total` connu) ou indéterminée, annulation tant
/// que le job n'a pas terminé.
class _ActiveTile extends StatelessWidget {
  const _ActiveTile({
    required this.tokens,
    required this.dl,
    this.onCancel,
  });

  final DesignTokens tokens;
  final ActiveDownload dl;
  final VoidCallback? onCancel;

  @override
  Widget build(BuildContext context) {
    final muted = tokens.color('onSurface', 'muted');
    final err = dl.error;
    return ListTile(
      dense: true,
      title: Text(dl.filename, maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            err ?? dl.url,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              color: err != null
                  ? tokens.color('danger', 'base', fallback: Colors.red)
                  : muted,
              fontSize: tokens.dim2('typography', 'size', 'sm', fallback: 12),
            ),
          ),
          if (!dl.finished)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: LinearProgressIndicator(value: dl.fraction),
            ),
        ],
      ),
      trailing: onCancel == null
          ? null
          : IconButton(
              icon: const Icon(Icons.close),
              tooltip: AppLocalizations.of(context).cancel,
              onPressed: onCancel,
            ),
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({required this.tokens, required this.text});

  final DesignTokens tokens;
  final String text;

  @override
  Widget build(BuildContext context) => Center(
    child: Text(
      text,
      style: TextStyle(color: tokens.color('onSurface', 'muted')),
    ),
  );
}

class _HistoryList extends StatelessWidget {
  const _HistoryList({
    required this.tokens,
    required this.entries,
    required this.onOpen,
  });

  final DesignTokens tokens;
  final List<HistoryEntry> entries;
  final void Function(String url) onOpen;

  @override
  Widget build(BuildContext context) {
    if (entries.isEmpty) {
      return _Message(tokens: tokens, text: AppLocalizations.of(context).emptyHistory);
    }
    return ListView.builder(
      itemCount: entries.length,
      itemBuilder: (_, i) {
        final e = entries[i];
        return _EntryTile(
          tokens: tokens,
          title: e.title.isEmpty ? e.url : e.title,
          url: e.url,
          onTap: () => onOpen(e.url),
        );
      },
    );
  }
}

class _BookmarkList extends StatelessWidget {
  const _BookmarkList({
    required this.tokens,
    required this.marks,
    required this.onOpen,
    required this.onRemove,
  });

  final DesignTokens tokens;
  final List<Bookmark> marks;
  final void Function(String url) onOpen;
  final void Function(String url) onRemove;

  @override
  Widget build(BuildContext context) {
    if (marks.isEmpty) {
      return _Message(
        tokens: tokens,
        text: AppLocalizations.of(context).emptyBookmarks,
      );
    }
    return ListView.builder(
      itemCount: marks.length,
      itemBuilder: (_, i) {
        final b = marks[i];
        return _EntryTile(
          tokens: tokens,
          title: b.title.isEmpty ? b.url : b.title,
          url: b.url,
          onTap: () => onOpen(b.url),
          trailing: IconButton(
            icon: const Icon(Icons.delete_outline),
            tooltip: AppLocalizations.of(context).removeBookmark,
            onPressed: () => onRemove(b.url),
          ),
        );
      },
    );
  }
}

class _EntryTile extends StatelessWidget {
  const _EntryTile({
    required this.tokens,
    required this.title,
    required this.url,
    required this.onTap,
    this.trailing,
  });

  final DesignTokens tokens;
  final String title;
  final String url;
  final VoidCallback onTap;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    final muted = tokens.color('onSurface', 'muted');
    return ListTile(
      dense: true,
      title: Text(title, maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Text(
        url,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: TextStyle(
          color: muted,
          fontSize: tokens.dim2('typography', 'size', 'sm', fallback: 12),
        ),
      ),
      trailing: trailing,
      onTap: onTap,
    );
  }
}
