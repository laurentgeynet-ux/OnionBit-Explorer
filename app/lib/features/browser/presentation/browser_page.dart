// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../core/design/design_tokens.dart';

/// Coquille navigateur — phase 0 : omnibox + état « daemon
/// non connecté ». Le contenu des onglets arrivera avec l'API
/// (phases 6–7 : frame PNG + hit-test — ADR-0004).
class BrowserPage extends ConsumerWidget {
  const BrowserPage({super.key, required this.tokens});

  final DesignTokens tokens;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final surface = tokens.color('surface', 'base');
    final muted = tokens.color('onSurface', 'muted');
    final pad = tokens.dim('spacing', 'md', fallback: 16);

    return Scaffold(
      backgroundColor: surface,
      body: SafeArea(
        child: Column(
          children: [
            Padding(
              padding: EdgeInsets.all(pad),
              child: TextField(
                decoration: const InputDecoration(
                  hintText: 'Rechercher ou saisir une URL',
                  prefixIcon: Icon(Icons.search),
                ),
                textInputAction: TextInputAction.go,
                onSubmitted: (_) {
                  // Le branchement navigate arrive avec la phase 6
                  // (POST /api/tabs/{id}/navigate) — avant le daemon,
                  // la soumission est ignorée sans effet de bord.
                },
              ),
            ),
            Expanded(
              child: Center(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(Icons.public, size: tokens.dim('spacing', 'xl', fallback: 40), color: muted),
                    SizedBox(height: pad),
                    Text(
                      'Explorer — moteur en cours de démarrage',
                      style: TextStyle(color: muted),
                    ),
                    SizedBox(height: tokens.dim('spacing', 'xs', fallback: 4)),
                    Text(
                      'Phase 0 : squelette UI — le daemon arrive en phase 6.',
                      style: TextStyle(
                        color: muted,
                        fontSize: tokens.dim2('typography', 'size', 'sm', fallback: 12),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
