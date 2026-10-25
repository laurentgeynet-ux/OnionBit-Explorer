// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Allowlist des domaines délégués à la WebView2 — étape 136b,
/// ADR-0027 §2.
///
/// Le matching est **déclaratif, jamais heuristique** : un
/// échec `play()` ou une erreur EME n'est pas un signal — on
/// délègue uniquement sur host explicite. Chaque entrée est un
/// arbitrage (périmètre de confiance + exception d'égress
/// ADR-0022 : le trafic WebView2 ne traverse pas
/// `explorer-net`).
///
/// Règle : host exact ou sous-domaine — `netflix.com` couvre
/// `www.netflix.com` et `devices.netflix.com` ; `evilnetflix.com`
/// ne matche pas.
///
/// Limite assumée : le matching est au **host entier** —
/// `amazon.tld` n'entre PAS dans la liste (déléguer tout
/// Amazon enverrait la boutique hors moteur ; la délégation
/// VOD d'Amazon passe par `primevideo.com`, et un futur
/// matching par chemin serait un arbitrage séparé).
class DrmAllowlist {
  DrmAllowlist(Iterable<String> hosts)
      : _hosts = {
          for (final h in hosts)
            h.trim().toLowerCase().replaceAll(RegExp(r'\.$'), ''),
        }..remove('');

  final Set<String> _hosts;

  /// Les entrées normalisées (minuscules, point final retiré).
  Iterable<String> get hosts => _hosts;

  /// `true` si l'URI doit ouvrir un onglet délégué WebView2.
  bool delegates(Uri uri) {
    if (uri.scheme != 'http' && uri.scheme != 'https') return false;
    final host = uri.host.toLowerCase();
    if (host.isEmpty) return false;
    return _hosts.any((e) => host == e || host.endsWith('.$e'));
  }

  /// Variante chaîne — `false` si l'URL est malformée (on ne
  /// délègue jamais ce qu'on ne sait pas parser).
  bool delegatesUrl(String url) {
    final uri = Uri.tryParse(url);
    return uri != null && delegates(uri);
  }
}
