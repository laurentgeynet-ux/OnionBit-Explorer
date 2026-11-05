// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:explorer_app/features/browser/data/media_player.dart';
import 'package:flutter_test/flutter_test.dart';

/// Étape 201 — diff structurel `_onNotify` : `==` couvre
/// **tous** les champs du snapshot ; `volatilesFrom` gèle
/// position/tampon/télémétrie (cadence pull) pour que
/// l'égalité ne teste que les champs stables. Verrous :
/// chaque champ stable divergeant est détecté ; les volatils
/// seuls ne diffèrent pas ; le contenu de liste compte
/// (l'ancien diff ne voyait que `.length`).
MediaSnapshot snap({
  int positionMs = 1000,
  int bufferedMs = 5000,
  String state = 'paused',
  int? durationMs = 60000,
  String? error,
  List<(int, int)> bufferedRanges = const [],
  MediaFrameStats? frameStats,
  List<MediaTrack> tracks = const [],
  int audioTrack = -1,
  int subtitleTrack = -1,
  String subtitle = '',
  List<MediaChapter> chapters = const [],
  int chapter = -1,
  int? loopState,
}) =>
    MediaSnapshot(
      positionMs: positionMs,
      bufferedMs: bufferedMs,
      state: state,
      durationMs: durationMs,
      error: error,
      bufferedRanges: bufferedRanges,
      frameStats: frameStats,
      tracks: tracks,
      audioTrack: audioTrack,
      subtitleTrack: subtitleTrack,
      subtitle: subtitle,
      chapters: chapters,
      chapter: chapter,
      loopState: loopState,
    );

void main() {
  test('201 — volatilesFrom gèle position/tampon/télémétrie', () {
    final a = snap(
      bufferedRanges: const [(0, 5000)],
      frameStats: const MediaFrameStats(
        presented: 10,
        dropped: 1,
        totalDelaySecs: 0.2,
      ),
    );
    final b = snap(
      positionMs: 9999,
      bufferedMs: 9000,
      bufferedRanges: const [(7000, 9000)],
      frameStats: const MediaFrameStats(
        presented: 99,
        dropped: 5,
        corrupted: 2,
        totalDelaySecs: 1.5,
      ),
    );
    // Bruts : différents ; volatils alignés : égaux.
    expect(a, isNot(equals(b)));
    final aligned = MediaSnapshot.volatilesFrom(b, a);
    expect(a, equals(aligned));
    expect(a.hashCode, aligned.hashCode);
  });

  test('201 — chaque champ stable divergeant est détecté', () {
    final base = snap();
    final variants = <String, MediaSnapshot>{
      'state': snap(state: 'playing'),
      'durationMs': snap(durationMs: 1),
      'error': snap(error: 'boom'),
      'audioTrack': snap(audioTrack: 0),
      'subtitleTrack': snap(subtitleTrack: 0),
      'subtitle': snap(subtitle: 'cue'),
      'chapter': snap(chapter: 0),
      'loopState': snap(loopState: -1),
      'tracks': snap(tracks: const [MediaTrack(index: 0, kind: 0)]),
      'chapters': snap(
        chapters: const [MediaChapter(index: 0, startMs: 0)],
      ),
    };
    variants.forEach((field, b) {
      expect(
        base,
        isNot(equals(MediaSnapshot.volatilesFrom(b, base))),
        reason: 'champ stable non détecté : $field',
      );
    });
  });

  test('201 — contenu de liste à même longueur diffère', () {
    // L'ancien diff ne comparait que `.length` : un titre de
    // piste modifié à longueur constante restait invisible.
    final en = snap(
      tracks: const [MediaTrack(index: 0, kind: 0, title: 'eng')],
    );
    final fr = snap(
      tracks: const [MediaTrack(index: 0, kind: 0, title: 'fra')],
    );
    expect(en, isNot(equals(fr)));
    // `bufferedRanges` compare par valeur (records), pas
    // par identité de liste.
    final r1 = snap(bufferedRanges: const [(0, 100), (500, 900)]);
    expect(
      r1,
      equals(snap(bufferedRanges: const [(0, 100), (500, 900)])),
    );
    expect(
      r1,
      isNot(equals(snap(bufferedRanges: const [(0, 100), (500, 800)]))),
    );
  });
}
