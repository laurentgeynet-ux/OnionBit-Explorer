// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import '../../../core/design/design_tokens.dart';
import '../data/media_player.dart';
import 'browser_controller.dart';
import 'media_controls.dart';

/// Vignette Picture-in-Picture (PiP) flottante (Étape 204).
///
/// Flotte au premier plan du viewport de navigation, draggable,
/// avec projection de la texture vidéo, commandes compactes au
/// survol (play/pause, pas-à-pas ±1 frame, seek, volume),
/// avance/recul à la molette en pause, et sous-titres incrustés.
class PipOverlay extends StatefulWidget {
  const PipOverlay({
    super.key,
    required this.ctrl,
    required this.tokens,
  });

  final BrowserController ctrl;
  final DesignTokens tokens;

  @override
  State<PipOverlay> createState() => _PipOverlayState();
}

class _PipOverlayState extends State<PipOverlay> {
  Offset? _offset;
  bool _hovered = false;
  MediaPlayer? _player;

  void _onNotify() {
    if (mounted) setState(() {});
  }

  void _attach(MediaPlayer? p) {
    if (identical(p, _player)) return;
    _player?.removeListener(_onNotify);
    _player = p;
    p?.addListener(_onNotify);
  }

  @override
  void dispose() {
    _player?.removeListener(_onNotify);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final pip = widget.ctrl.currentPip;
    if (pip == null) return const SizedBox.shrink();

    final tabId = pip.tabId;
    final mediaId = pip.mediaId;
    final p = widget.ctrl.mediaPlayer(tabId, mediaId);
    _attach(p);

    if (p == null) return const SizedBox.shrink();
    final snap = p.snapshot;
    final playing = snap.state == 'playing';
    final durMs = snap.durationMs;
    final live = durMs == null;
    final posMs = snap.positionMs;
    final muted = p.muted;
    final vol = muted ? 0.0 : p.volume;

    const width = 360.0;
    const height = 202.5; // ratio 16:9

    return LayoutBuilder(
      builder: (context, constraints) {
        final maxW = constraints.maxWidth;
        final maxH = constraints.maxHeight;

        // Position par défaut : coin inférieur droit (avec marge 24px)
        _offset ??= Offset(
          (maxW - width - 24).clamp(0.0, maxW),
          (maxH - height - 24).clamp(0.0, maxH),
        );

        // Clamp pour ne pas déborder hors écran
        final curX = _offset!.dx.clamp(0.0, (maxW - width).clamp(0.0, maxW));
        final curY = _offset!.dy.clamp(0.0, (maxH - height).clamp(0.0, maxH));
        _offset = Offset(curX, curY);

        return Positioned(
          left: curX,
          top: curY,
          width: width,
          height: height,
          child: Material(
            elevation: 12,
            shadowColor: Colors.black54,
            borderRadius: BorderRadius.circular(10),
            clipBehavior: Clip.antiAlias,
            color: Colors.black,
            child: MouseRegion(
              onEnter: (_) => setState(() => _hovered = true),
              onExit: (_) => setState(() => _hovered = false),
              child: Stack(
                fit: StackFit.expand,
                children: [
                  // 1. Couche Texture Vidéo
                  if (p.ready)
                    Positioned.fill(
                      child: p.view,
                    ),

                  // 2. Gestion de la molette en pause (pas-à-pas) + drag de la fenêtre
                  Positioned.fill(
                    child: Listener(
                      onPointerSignal: (signal) {
                        if (signal is PointerScrollEvent && !playing) {
                          final dir = signal.scrollDelta.dy < 0 ? 1 : -1;
                          widget.ctrl.mediaStepFrame(tabId, mediaId, dir);
                        }
                      },
                      child: GestureDetector(
                        behavior: HitTestBehavior.translucent,
                        onPanUpdate: (d) {
                          setState(() {
                            _offset = Offset(
                              (_offset!.dx + d.delta.dx).clamp(0.0, maxW - width),
                              (_offset!.dy + d.delta.dy).clamp(0.0, maxH - height),
                            );
                          });
                        },
                      ),
                    ),
                  ),

                  // 3. Sous-titres incrustés
                  if (snap.subtitle.isNotEmpty)
                    Positioned(
                      left: 12,
                      right: 12,
                      bottom: _hovered ? 44 : 12,
                      child: IgnorePointer(
                        child: Center(
                          child: Container(
                            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                            decoration: BoxDecoration(
                              color: Colors.black.withValues(alpha: 0.8),
                              borderRadius: BorderRadius.circular(4),
                            ),
                            child: Text(
                              snap.subtitle,
                              textAlign: TextAlign.center,
                              style: const TextStyle(
                                color: Colors.white,
                                fontSize: 13,
                                height: 1.2,
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),

                  // 4. Barre de contrôle et en-tête PiP (au survol ou en pause)
                  AnimatedOpacity(
                    opacity: (_hovered || !playing) ? 1.0 : 0.0,
                    duration: const Duration(milliseconds: 150),
                    child: IgnorePointer(
                      ignoring: !_hovered && playing,
                      child: Container(
                        decoration: const BoxDecoration(
                          gradient: LinearGradient(
                            begin: Alignment.topCenter,
                            end: Alignment.bottomCenter,
                            colors: [
                              Colors.black87,
                              Colors.transparent,
                              Colors.transparent,
                              Colors.black87,
                            ],
                            stops: [0.0, 0.28, 0.65, 1.0],
                          ),
                        ),
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.spaceBetween,
                          children: [
                            // Header PiP : badge draggable + restaurer + fermer
                            Padding(
                              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
                              child: Row(
                                children: [
                                  const Icon(Icons.picture_in_picture_alt, size: 16, color: Colors.white70),
                                  const SizedBox(width: 6),
                                  const Text(
                                    'PiP',
                                    style: TextStyle(
                                      color: Colors.white70,
                                      fontSize: 11,
                                      fontWeight: FontWeight.bold,
                                      letterSpacing: 0.5,
                                    ),
                                  ),
                                  const Spacer(),
                                  // Restaurer / Dock back dans la page
                                  IconButton(
                                    visualDensity: VisualDensity.compact,
                                    iconSize: 18,
                                    padding: EdgeInsets.zero,
                                    constraints: const BoxConstraints(minWidth: 26, minHeight: 26),
                                    style: IconButton.styleFrom(foregroundColor: Colors.white),
                                    icon: const Icon(Icons.open_in_new),
                                    tooltip: 'Restaurer dans la page',
                                    onPressed: () => widget.ctrl.mediaClosePip(),
                                  ),
                                  const SizedBox(width: 4),
                                  // Fermer PiP
                                  IconButton(
                                    visualDensity: VisualDensity.compact,
                                    iconSize: 18,
                                    padding: EdgeInsets.zero,
                                    constraints: const BoxConstraints(minWidth: 26, minHeight: 26),
                                    style: IconButton.styleFrom(foregroundColor: Colors.white),
                                    icon: const Icon(Icons.close),
                                    tooltip: 'Fermer',
                                    onPressed: () => widget.ctrl.mediaClosePip(),
                                  ),
                                ],
                              ),
                            ),

                            // Footer PiP : play/pause, [-1f], [+1f], seek, mute
                            Padding(
                              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
                              child: Row(
                                children: [
                                  IconButton(
                                    visualDensity: VisualDensity.compact,
                                    iconSize: 22,
                                    padding: EdgeInsets.zero,
                                    constraints: const BoxConstraints(minWidth: 28, minHeight: 28),
                                    style: IconButton.styleFrom(foregroundColor: Colors.white),
                                    icon: Icon(playing ? Icons.pause : Icons.play_arrow),
                                    tooltip: playing ? 'Pause' : 'Lecture',
                                    onPressed: () => widget.ctrl.mediaToggle(tabId, mediaId),
                                  ),
                                  if (!live) ...[
                                    IconButton(
                                      visualDensity: VisualDensity.compact,
                                      iconSize: 18,
                                      padding: EdgeInsets.zero,
                                      constraints: const BoxConstraints(minWidth: 24, minHeight: 24),
                                      style: IconButton.styleFrom(foregroundColor: Colors.white70),
                                      icon: const Icon(Icons.arrow_left),
                                      tooltip: 'Image précédente (molette bas)',
                                      onPressed: () => widget.ctrl.mediaStepFrame(tabId, mediaId, -1),
                                    ),
                                    IconButton(
                                      visualDensity: VisualDensity.compact,
                                      iconSize: 18,
                                      padding: EdgeInsets.zero,
                                      constraints: const BoxConstraints(minWidth: 24, minHeight: 24),
                                      style: IconButton.styleFrom(foregroundColor: Colors.white70),
                                      icon: const Icon(Icons.arrow_right),
                                      tooltip: 'Image suivante (molette haut)',
                                      onPressed: () => widget.ctrl.mediaStepFrame(tabId, mediaId, 1),
                                    ),
                                    Expanded(
                                      child: SliderTheme(
                                        data: SliderTheme.of(context).copyWith(
                                          trackHeight: 2,
                                          thumbShape: const RoundSliderThumbShape(enabledThumbRadius: 4),
                                          overlayShape: const RoundSliderOverlayShape(overlayRadius: 8),
                                          activeTrackColor: Colors.white,
                                          inactiveTrackColor: Colors.white24,
                                          thumbColor: Colors.white,
                                        ),
                                        child: Slider(
                                          value: durMs <= 0 ? 0 : (posMs / durMs).clamp(0.0, 1.0),
                                          onChanged: (f) {
                                            widget.ctrl.mediaSeekTo(
                                              tabId,
                                              mediaId,
                                              Duration(milliseconds: (f * durMs).round()),
                                            );
                                          },
                                        ),
                                      ),
                                    ),
                                    Text(
                                      MediaControlsBar.fmtTime(posMs),
                                      style: const TextStyle(color: Colors.white70, fontSize: 10),
                                    ),
                                    const SizedBox(width: 4),
                                  ] else
                                    const Spacer(),
                                  IconButton(
                                    visualDensity: VisualDensity.compact,
                                    iconSize: 18,
                                    padding: EdgeInsets.zero,
                                    constraints: const BoxConstraints(minWidth: 26, minHeight: 26),
                                    style: IconButton.styleFrom(foregroundColor: Colors.white),
                                    icon: Icon(muted || vol <= 0 ? Icons.volume_off : Icons.volume_up),
                                    tooltip: muted ? 'Activer le son' : 'Couper le son',
                                    onPressed: () => widget.ctrl.mediaToggleMute(tabId, mediaId),
                                  ),
                                ],
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}
