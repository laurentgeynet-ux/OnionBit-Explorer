// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:explorer_app/core/design/design_tokens.dart';
import 'package:explorer_app/main.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('app démarre et affiche la coquille navigateur',
      (WidgetTester tester) async {
    await tester.pumpWidget(const ProviderScope(child: ExplorerApp()));
    await tester.pump();
    // Le champ omnibox est présent (tokens de repli utilisés hors bundle).
    expect(find.byType(TextField), findsOneWidget);
    expect(find.byIcon(Icons.search), findsOneWidget);
  });

  test('tokens de repli exposent les sections clés', () {
    final t = DesignTokens.fallback;
    expect(t.color('surface', 'base'), isA<Color>());
    expect(t.dim('spacing', 'md'), greaterThan(0));
    expect(t.dim('radius', 'md'), greaterThan(0));
    expect(t.str('typography', 'fontFamily'), isNotEmpty);
  });
}
