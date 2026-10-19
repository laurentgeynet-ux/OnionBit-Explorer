// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/design/design_tokens.dart';
import '../../../core/settings/app_settings.dart';

/// Panneau de réglages (étape 57) : thème (clair/sombre/système),
/// moteur (auto/REST/FFI), répertoire de profil
/// (`EXPLORER_PROFILE`). Persisté à chaque changement via
/// [settingsProvider] — le mode moteur recrée le dépôt, le
/// profil prend effet au prochain chargement du moteur.
class SettingsDialog extends ConsumerStatefulWidget {
  const SettingsDialog({super.key, required this.tokens});

  final DesignTokens tokens;

  static Future<void> show(BuildContext context, DesignTokens tokens) =>
      showDialog<void>(
        context: context,
        builder: (_) => SettingsDialog(tokens: tokens),
      );

  @override
  ConsumerState<SettingsDialog> createState() => _SettingsDialogState();
}

class _SettingsDialogState extends ConsumerState<SettingsDialog> {
  late final TextEditingController _profile;

  @override
  void initState() {
    super.initState();
    _profile = TextEditingController(
      text: ref.read(settingsProvider).profileDir,
    );
  }

  @override
  void dispose() {
    _profile.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.tokens;
    final settings = ref.watch(settingsProvider);
    final ctrl = ref.read(settingsProvider.notifier);
    final muted = t.color('onSurface', 'muted');
    final pad = t.dim('spacing', 'md', fallback: 16);

    Widget row(String label, Widget child) => Padding(
      padding: EdgeInsets.symmetric(vertical: t.dim('spacing', 'xs')),
      child: Row(
        children: [
          SizedBox(width: 110, child: Text(label)),
          Expanded(child: child),
        ],
      ),
    );

    return AlertDialog(
      backgroundColor: t.color('surface', 'raised'),
      title: const Text('Réglages'),
      content: SizedBox(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            row(
              'Thème',
              DropdownButton<String>(
                value: settings.theme,
                isExpanded: true,
                items: const [
                  DropdownMenuItem(value: 'system', child: Text('Système')),
                  DropdownMenuItem(value: 'dark', child: Text('Sombre')),
                  DropdownMenuItem(value: 'light', child: Text('Clair')),
                ],
                onChanged: (v) {
                  if (v != null) ctrl.setTheme(v);
                },
              ),
            ),
            row(
              'Moteur',
              DropdownButton<String>(
                value: settings.engine,
                isExpanded: true,
                items: const [
                  DropdownMenuItem(
                    value: 'auto',
                    child: Text('Auto (FFI si présent, REST sinon)'),
                  ),
                  DropdownMenuItem(value: 'rest', child: Text('REST (daemon)')),
                  DropdownMenuItem(
                    value: 'ffi',
                    child: Text('FFI (moteur in-process)'),
                  ),
                ],
                onChanged: (v) {
                  if (v != null) ctrl.setEngine(v);
                },
              ),
            ),
            row(
              'Profil',
              TextField(
                controller: _profile,
                decoration: const InputDecoration(
                  hintText: 'Répertoire de profil (défaut : OS)',
                  isDense: true,
                ),
                onSubmitted: ctrl.setProfileDir,
              ),
            ),
            Align(
              alignment: Alignment.centerLeft,
              child: Padding(
                padding: EdgeInsets.only(top: t.dim('spacing', 'xs')),
                child: Text(
                  'Profil : exporté vers EXPLORER_PROFILE — effet au '
                  'prochain démarrage du moteur (FFI) ou du daemon '
                  '(REST). Valider avec Entrée.',
                  style: TextStyle(
                    color: muted,
                    fontSize: t.dim2('typography', 'size', 'sm', fallback: 12),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
      contentPadding: EdgeInsets.all(pad),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Fermer'),
        ),
      ],
    );
  }
}
