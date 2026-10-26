// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:explorer_app/core/config/app_config.dart';
import 'package:explorer_app/features/browser/domain/drm_allowlist.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final allow = DrmAllowlist(const [
    'netflix.com',
    'canalplus.com',
    'primevideo.com',
  ]);

  test('host exact délégué', () {
    expect(allow.delegatesUrl('https://netflix.com/browse'), isTrue);
    expect(allow.delegatesUrl('https://primevideo.com'), isTrue);
  });

  test('sous-domaines couverts', () {
    expect(allow.delegatesUrl('https://www.netflix.com/'), isTrue);
    expect(allow.delegatesUrl('https://devices.netflix.com/x'), isTrue);
    expect(allow.delegatesUrl('https://a.b.canalplus.com'), isTrue);
  });

  test('domaines proches non listés refusés', () {
    expect(allow.delegatesUrl('https://evilnetflix.com'), isFalse);
    expect(allow.delegatesUrl('https://netflix.com.evil.fr'), isFalse);
    expect(allow.delegatesUrl('https://amazon.fr/gp/video'), isFalse);
    expect(allow.delegatesUrl('https://example.com'), isFalse);
  });

  test('schémas et entrées hors web refusés', () {
    expect(allow.delegatesUrl('about:blank'), isFalse);
    expect(allow.delegatesUrl('ftp://netflix.com'), isFalse);
    expect(allow.delegatesUrl('data:text/html,x'), isFalse);
    expect(allow.delegatesUrl(''), isFalse);
    expect(allow.delegatesUrl('not a url'), isFalse);
  });

  test('normalisation : casse, point final, blancs, vide', () {
    final a = DrmAllowlist(const ['NETFLIX.com ', ' canap.fr.', '']);
    expect(a.delegatesUrl('https://www.netflix.com'), isTrue);
    expect(a.delegatesUrl('https://canap.fr'), isTrue);
    expect(a.hosts, isNot(contains('')));
  });

  test('seed produit : YouTube (MSE hors moteur + DRM) délégué', () {
    final a = DrmAllowlist(const AppConfig().drmDelegateHosts);
    expect(a.delegatesUrl('https://www.youtube.com/watch?v=x'), isTrue);
    expect(a.delegatesUrl('https://m.youtube.com'), isTrue);
    expect(a.delegatesUrl('https://music.youtube.com'), isTrue);
    expect(a.delegatesUrl('https://youtu.be/abc'), isTrue);
    // Google lui-même reste sur le moteur.
    expect(a.delegatesUrl('https://www.google.com'), isFalse);
  });

  test('seed produit : catalogue streaming étendu', () {
    final a = DrmAllowlist(const AppConfig().drmDelegateHosts);
    for (final url in [
      'https://www.hulu.com/watch/x',
      'https://play.max.com',
      'https://tv.apple.com/fr/show/x',
      'https://www.tf1plus.fr/direct',
      'https://www.dazn.com/fr-FR/home',
      'https://www.twitch.tv/chaine',
      'https://tv.sfr.fr/direct',
      'https://www.bbc.co.uk/iplayer',
    ]) {
      expect(a.delegatesUrl(url), isTrue, reason: url);
    }
    // La boutique Amazon reste sur le moteur.
    expect(a.delegatesUrl('https://www.amazon.fr/dp/x'), isFalse);
  });

  test('host avec port et credentials matche sur le host seul', () {
    expect(allow.delegatesUrl('https://netflix.com:443/browse'), isTrue);
    expect(allow.delegatesUrl('https://u:p@netflix.com/'), isTrue);
  });
}
