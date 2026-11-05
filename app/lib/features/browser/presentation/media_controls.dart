// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../../../core/design/design_tokens.dart';
import '../../../core/l10n/app_localizations.dart';
import '../data/media_player.dart';
import '../domain/media_offer.dart';
import 'browser_controller.dart';

/// Surface média claimée — contrôles natifs (étape 183,
/// ADR-0024) : la vidéo reste **transparente tant que la
/// première frame n'existe pas** — le poster peint par le
/// moteur sous le même rect fait office d'affiche, jamais de
/// flash noir. `<video controls>` reçoit la barre native
/// (lecture, seek, volume) auto-masquée en lecture ; hors
/// `controls` la surface n'a aucun recognizer et l'appelant
/// l'enveloppe d'`IgnorePointer` — les gestes de la page
/// atteignent le moteur (hit-test contractuel).
class MediaSurfaceView extends StatefulWidget {
  const MediaSurfaceView({
    super.key,
    required this.ctrl,
    required this.tabId,
    required this.offer,
    required this.tokens,
    required this.cssPerLogical,
    required this.scrollY,
    required this.onActive,
  });

  final BrowserController ctrl;
  final String tabId;
  final MediaOffer offer;

  /// Tokens design (barre, scrim, icônes — rien en dur).
  final DesignTokens tokens;

  /// Échelle affichage → px CSS moteur (`fZoom` de la frame).
  final double cssPerLogical;

  /// `scroll_y` courant de l'onglet — conversion du tap
  /// overlay → coords document pour le hit-test moteur.
  final double scrollY;

  /// Dernier média interagi — cible des raccourcis clavier
  /// (espace, ←/→, ↑/↓, M) définis dans `_FrameView`.
  final ValueChanged<int> onActive;

  @override
  State<MediaSurfaceView> createState() => _MediaSurfaceViewState();
}

class _MediaSurfaceViewState extends State<MediaSurfaceView> {
  /// Barre visible — démarre affichée (l'utilisateur voit les
  /// contrôles apparaître puis s'effacer, convention native).
  bool _visible = true;
  Timer? _hide;

  /// Fraction de seek en cours de drag — affichée tant que le
  /// geste n'est pas lâché (le seek ne part qu'à `onChangeEnd`,
  /// pas à chaque crans de la glissière).
  double? _dragFrac;

  /// Lecteur écouté — posé/retiré par identité : le claim est
  /// asynchrone, le player arrive après le premier build.
  MediaPlayer? _listened;
  MediaSnapshot? _snap;

  /// Champs UI du dernier snapshot lu — le rebuild ne part
  /// que si l'un change (position quantifiée à ~100 ms, le
  /// lecteur notifie à chaque frame décodée).
  int _qPos = -1, _qBuf = -1;
  int? _lastDur;
  bool _lastReady = false, _lastMuted = false;
  double _lastVol = -1;

  void _attach(MediaPlayer? p) {
    if (identical(p, _listened)) return;
    _listened?.removeListener(_onNotify);
    _listened = p;
    p?.addListener(_onNotify);
  }

  void _onNotify() {
    final p = _listened;
    if (p == null || !mounted) return;
    final s = p.snapshot;
    final qp = s.positionMs ~/ 100, qb = s.bufferedMs ~/ 250;
    final changed = _snap?.state != s.state ||
        _lastDur != s.durationMs ||
        _lastReady != p.ready ||
        _lastMuted != p.muted ||
        _lastVol != p.volume ||
        qp != _qPos ||
        qb != _qBuf;
    _qPos = qp;
    _qBuf = qb;
    _lastDur = s.durationMs;
    _lastReady = p.ready;
    _lastMuted = p.muted;
    _lastVol = p.volume;
    _snap = s;
    // Transition → la barre se réaffiche et ré-arme sa
    // disparition (playing) ou reste (paused/ended/buffering).
    if (changed) {
      setState(() {});
      _rearm();
    }
  }

  /// Ré-arme l'auto-masquage : visible immédiatement ; la
  /// barre ne s'efface que pendant `playing` (une pause, un
  /// buffering, une fin la gardent au premier plan).
  void _rearm() {
    _hide?.cancel();
    if (!mounted) return;
    if (!_visible) setState(() => _visible = true);
    final s = _snap;
    if (s != null && s.state == 'playing') {
      _hide = Timer(widget.ctrl.cfg.mediaControlsHideDelay, () {
        if (mounted && _dragFrac == null) {
          setState(() => _visible = false);
        }
      });
    }
  }

  /// Tap sur la zone vidéo d'une surface `controls` : sonde
  /// `/hit` d'abord (étape 186 — arbitrage pointer M2).
  /// `media == o.id` = le nœud topmost vit dans l'élément
  /// (vidéo nue) → forward + toggle, statu quo navigateur.
  /// `media == null` sur sonde sûre = un contrôle DOM du
  /// site est peint sous le point → forward **seul** : la
  /// double détente déclencherait le geste du site *et* le
  /// nôtre. Sonde en échec ou sans la clé (daemon/dll
  /// antérieurs) → repli M0 forward+toggle. Le forward est
  /// toujours émis (fire-and-forget `POST /input`) : le
  /// handler JS du site reçoit son clic dans tous les cas.
  Future<void> _onVideoTap(TapUpDetails d) async {
    final z = widget.cssPerLogical;
    final o = widget.offer;
    widget.onActive(o.id);
    // Point **viewport** — servi à la sonde et au clic.
    final vx = o.rect.left + d.localPosition.dx / z;
    final vy = o.rect.top - widget.scrollY + d.localPosition.dy / z;
    final h = await widget.ctrl.hitTarget(widget.tabId, vx, vy);
    if (!mounted) return;
    widget.ctrl.click(vx, vy);
    if (h == null || !h.mediaKnown || h.media == o.id) {
      widget.ctrl.mediaToggle(widget.tabId, o.id);
    }
    _rearm();
  }

  @override
  void dispose() {
    _hide?.cancel();
    _listened?.removeListener(_onNotify);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.tokens;
    final o = widget.offer;
    final p = widget.ctrl.mediaPlayer(widget.tabId, o.id);
    _attach(p);
    final snap = _snap ?? p?.snapshot;
    final ready = p?.ready == true;
    final playing = snap?.state == 'playing';
    // Audio : la barre est toute la surface — pas de vidéo à
    // découvrir, jamais masquée.
    final barShown = o.controls &&
        (o.hasAudioBar ||
            _visible ||
            _dragFrac != null ||
            !playing);
    return MouseRegion(
      cursor: o.isVideo && playing && !_visible
          ? SystemMouseCursors.none
          : SystemMouseCursors.basic,
      onEnter: (_) {
        widget.onActive(o.id);
        _rearm();
      },
      onHover: (_) => _rearm(),
      onExit: (_) => _rearm(),
      child: Stack(
        fit: StackFit.expand,
        children: [
          // Couche vidéo — transparente avant la première
          // frame : le poster peint par le moteur reste
          // visible (pas de trou noir).
          if (o.isVideo)
            Positioned.fill(
              child: GestureDetector(
                // La vue du lecteur peut être une texture sans
                // hit-test propre — le tap appartient à toute
                // la surface `controls`, pas au widget interne.
                behavior: HitTestBehavior.opaque,
                onTapUp: o.controls ? _onVideoTap : null,
                child: ready ? p!.view : const SizedBox.shrink(),
              ),
            ),
          // Init / buffering — indicateur discret centré.
          if (o.isVideo && (!ready || snap?.state == 'buffering'))
            const Center(
              child: SizedBox(
                width: 26,
                height: 26,
                child: CircularProgressIndicator(
                  strokeWidth: 2.5,
                  color: Colors.white70,
                ),
              ),
            ),
          // `ended`/`error` : indicateurs réservés aux
          // surfaces `controls` — hors `controls` la page
          // possède son UI (la nôtre serait un overlay
          // muet non cliquable sous `IgnorePointer`).
          if (o.controls && snap?.state == 'ended')
            Center(
              child: IconButton(
                iconSize: 42,
                style: IconButton.styleFrom(
                  backgroundColor: Colors.black54,
                  foregroundColor: Colors.white,
                ),
                icon: const Icon(Icons.replay),
                tooltip: AppLocalizations.of(context).mediaReplay,
                onPressed: () {
                  widget.onActive(o.id);
                  widget.ctrl.mediaReplay(widget.tabId, o.id);
                },
              ),
            ),
          if (o.controls && snap?.state == 'error')
            Center(
              child: Icon(
                Icons.error_outline,
                size: t.dim('metrics', 'iconXl', fallback: 48),
                color: t.color('danger', 'base', fallback: Colors.redAccent),
                semanticLabel: AppLocalizations.of(context).mediaError,
              ),
            ),
          // « Enregistrer la source » — la ressource
          // progressive (URL non-MSE) est sauvable via la pile
          // transport ; hors `controls` la surface est
          // transparente au pointeur, pas de bouton (le geste
          // appartient à la page).
          if (o.controls && o.url.isNotEmpty && !o.mse)
            Positioned(
              top: 4,
              right: 4,
              child: IconButton(
                visualDensity: VisualDensity.compact,
                style: IconButton.styleFrom(
                  backgroundColor: Colors.black54,
                  foregroundColor: Colors.white,
                ),
                icon: const Icon(Icons.download, size: 18),
                tooltip: AppLocalizations.of(context).save,
                onPressed: () async {
                  final messenger = ScaffoldMessenger.of(context);
                  final l10n = AppLocalizations.of(context);
                  final id =
                      await widget.ctrl.downloadStart(widget.tabId, o.url);
                  if (!context.mounted) return;
                  messenger.showSnackBar(
                    SnackBar(
                      content: Text(
                        id == null
                            ? l10n.downloadError(o.url)
                            : l10n.downloadStarted,
                      ),
                    ),
                  );
                },
              ),
            ),
          // Barre de contrôles — `controls` vidéo (bas,
          // auto-masquée) ou `controls` audio (remplit le rect,
          // fixe). `IgnorePointer` quand fondue : une barre
          // invisible ne doit pas manger les gestes vidéo.
          if (o.controls)
            Positioned(
              left: 0,
              right: 0,
              top: o.hasAudioBar ? 0 : null,
              bottom: 0,
              child: AnimatedOpacity(
                opacity: barShown ? 1.0 : 0.0,
                duration: widget.ctrl.cfg.mediaControlsFade,
                child: IgnorePointer(
                  ignoring: !barShown,
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    // Les gestes sur le fond de barre ne sont
                    // pas des clics vidéo — absorbés, ils
                    // ré-arment juste l'auto-masquage.
                    onTap: _rearm,
                    child: MediaControlsBar(
                      player: p,
                      snap: snap,
                      tokens: t,
                      audio: !o.isVideo,
                      dragFrac: _dragFrac,
                      onToggle: () {
                        widget.onActive(o.id);
                        widget.ctrl.mediaToggle(widget.tabId, o.id);
                        _rearm();
                      },
                      onSeekFrac: (frac) {
                        widget.onActive(o.id);
                        _rearm();
                        setState(() => _dragFrac = frac);
                      },
                      onSeekEnd: (frac) {
                        final dur = snap?.durationMs;
                        setState(() => _dragFrac = null);
                        if (dur != null && dur > 0) {
                          widget.ctrl.mediaSeekTo(
                            widget.tabId,
                            o.id,
                            Duration(milliseconds: (frac * dur).round()),
                          );
                        }
                      },
                      onVolume: (v) {
                        widget.onActive(o.id);
                        widget.ctrl.mediaSetVolume(widget.tabId, o.id, v);
                      },
                      onVolumeEnd: () => widget.ctrl.mediaPersistAudio(
                        widget.tabId,
                        o.id,
                      ),
                      onToggleMute: () {
                        widget.onActive(o.id);
                        widget.ctrl.mediaToggleMute(widget.tabId, o.id);
                      },
                    ),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// Barre de contrôles native — play/pause, position,
/// progression (piste secondaire = bufferisé), durée ou
/// badge DIRECT, volume + mute. `player == null` (claim en
/// cours) → contrôles désactivés : la surface existe avant
/// le lecteur, jamais de faux boutons actifs.
class MediaControlsBar extends StatelessWidget {
  const MediaControlsBar({
    super.key,
    required this.player,
    required this.snap,
    required this.tokens,
    required this.audio,
    required this.dragFrac,
    required this.onToggle,
    required this.onSeekFrac,
    required this.onSeekEnd,
    required this.onVolume,
    required this.onVolumeEnd,
    required this.onToggleMute,
  });

  final MediaPlayer? player;
  final MediaSnapshot? snap;
  final DesignTokens tokens;

  /// `<audio controls>` : fond plein (pas de scrim vidéo) et
  /// barre centrée dans le rect offert.
  final bool audio;

  /// Fraction `[0..1]` affichée pendant un drag de seek.
  final double? dragFrac;
  final VoidCallback onToggle;
  final ValueChanged<double> onSeekFrac;
  final ValueChanged<double> onSeekEnd;
  final ValueChanged<double> onVolume;

  /// Fin de geste volume (`onChangeEnd` du slider) — le
  /// contrôleur persiste le réglage une fois, pas à chaque
  /// crans de drag.
  final VoidCallback onVolumeEnd;
  final VoidCallback onToggleMute;

  /// `mm:ss` ou `h:mm:ss` — zéro padding sur l'heure.
  static String fmtTime(int ms) {
    final d = Duration(milliseconds: ms < 0 ? 0 : ms);
    final h = d.inHours;
    final m = d.inMinutes.remainder(60);
    final s = d.inSeconds.remainder(60);
    String two(int n) => n.toString().padLeft(2, '0');
    return h > 0 ? '$h:${two(m)}:${two(s)}' : '${d.inMinutes}:${two(s)}';
  }

  @override
  Widget build(BuildContext context) {
    final t = tokens;
    final l10n = AppLocalizations.of(context);
    final s = snap;
    final durMs = s?.durationMs;
    final live = durMs == null;
    final posMs = s?.positionMs ?? 0;
    final playing = s?.state == 'playing';
    final muted = player?.muted == true;
    final vol = muted ? 0.0 : (player?.volume ?? 1.0);
    final onSurface = Colors.white;
    final compact = VisualDensity.compact;

    final bar = Row(
      children: [
        IconButton(
          visualDensity: compact,
          iconSize: 22,
          style: IconButton.styleFrom(foregroundColor: onSurface),
          icon: Icon(
            s?.state == 'ended'
                ? Icons.replay
                : playing
                    ? Icons.pause
                    : Icons.play_arrow,
          ),
          tooltip: playing
              ? l10n.mediaPause
              : s?.state == 'ended'
                  ? l10n.mediaReplay
                  : l10n.mediaPlay,
          onPressed: player == null ? null : onToggle,
        ),
        if (!live)
          Text(
            fmtTime(
              dragFrac != null ? (dragFrac! * durMs).round() : posMs,
            ),
            style: TextStyle(color: onSurface, fontSize: 12),
          ),
        Expanded(
          child: live
              ? const SizedBox.shrink()
              : SliderTheme(
                  data: SliderTheme.of(context).copyWith(
                    trackHeight: 3,
                    thumbShape: const RoundSliderThumbShape(
                      enabledThumbRadius: 6,
                    ),
                    overlayShape: const RoundSliderOverlayShape(
                      overlayRadius: 12,
                    ),
                    activeTrackColor: t.color(
                      'accent',
                      'base',
                      fallback: Colors.white,
                    ),
                    inactiveTrackColor: Colors.white24,
                    secondaryActiveTrackColor: Colors.white38,
                    thumbColor: onSurface,
                    overlayColor: Colors.white24,
                  ),
                  child: Slider(
                    value: durMs <= 0
                        ? 0
                        : ((dragFrac ?? posMs / durMs).clamp(0.0, 1.0)),
                    secondaryTrackValue: durMs <= 0
                        ? 0
                        : ((s?.bufferedAheadEnd(posMs) ?? 0) / durMs)
                            .clamp(0.0, 1.0),
                    onChanged: player == null ? null : onSeekFrac,
                    onChangeEnd: player == null ? null : onSeekEnd,
                  ),
                ),
        ),
        if (live)
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
            decoration: BoxDecoration(
              color: t.color('danger', 'base', fallback: Colors.red),
              borderRadius: BorderRadius.circular(3),
            ),
            child: Text(
              l10n.mediaLive,
              style: const TextStyle(
                color: Colors.white,
                fontSize: 11,
                fontWeight: FontWeight.w700,
                letterSpacing: 0.5,
              ),
            ),
          )
        else
          Text(
            fmtTime(durMs),
            style: TextStyle(color: onSurface, fontSize: 12),
          ),
        IconButton(
          visualDensity: compact,
          iconSize: 20,
          style: IconButton.styleFrom(foregroundColor: onSurface),
          icon: Icon(
            muted || vol <= 0
                ? Icons.volume_off
                : vol < 0.5
                    ? Icons.volume_down
                    : Icons.volume_up,
          ),
          tooltip: muted ? l10n.mediaUnmute : l10n.mediaMute,
          onPressed: player == null ? null : onToggleMute,
        ),
        SizedBox(
          width: 72,
          child: SliderTheme(
            data: SliderTheme.of(context).copyWith(
              trackHeight: 3,
              thumbShape: const RoundSliderThumbShape(
                enabledThumbRadius: 5,
              ),
              overlayShape: const RoundSliderOverlayShape(
                overlayRadius: 10,
              ),
              activeTrackColor: onSurface,
              inactiveTrackColor: Colors.white24,
              thumbColor: onSurface,
              overlayColor: Colors.white24,
            ),
            child: Slider(
              value: vol.clamp(0.0, 1.0),
              onChanged: player == null ? null : onVolume,
              onChangeEnd: player == null ? null : (_) => onVolumeEnd(),
            ),
          ),
        ),
      ],
    );

    final pad = t.dim('spacing', 'xs', fallback: 4);
    if (audio) {
      // `<audio controls>` : barre pleine dans le rect —
      // fond des tokens (pas de scrim dégradé vidéo).
      return Container(
        color: t.color('surface', 'sunken', fallback: Colors.black87),
        padding: EdgeInsets.symmetric(horizontal: pad),
        child: Center(child: bar),
      );
    }
    // Vidéo : scrim dégradé bas — lisible sur tout contenu,
    // disparaît avec la barre (AnimatedOpacity parent).
    return Container(
      decoration: const BoxDecoration(
        gradient: LinearGradient(
          begin: Alignment.bottomCenter,
          end: Alignment.topCenter,
          colors: [Colors.black87, Colors.transparent],
        ),
      ),
      padding: EdgeInsets.fromLTRB(pad, 18, pad, pad),
      child: bar,
    );
  }
}
