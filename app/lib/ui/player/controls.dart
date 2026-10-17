/// Player controls shared by the mini player, the fullscreen player and the
/// wave screen.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/player_models.dart';
import '../../state/likes.dart';
import '../../state/player.dart';
import '../theme/app_theme.dart';
import 'position.dart';

// ---- seek bar ----------------------------------------------------------------

class SeekBar extends ConsumerStatefulWidget {
  final bool showTimes;

  /// Thickness of the bar at rest; it grows on hover.
  final double thickness;

  const SeekBar({super.key, this.showTimes = true, this.thickness = 4});

  @override
  ConsumerState<SeekBar> createState() => _SeekBarState();
}

class _SeekBarState extends ConsumerState<SeekBar> {
  double? _drag;
  bool _hover = false;

  double _fraction(Offset local, double width) => (local.dx / width).clamp(0.0, 1.0);

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    final timeStyle = t.textTheme.labelMedium?.copyWith(
      color: t.colorScheme.onSurfaceVariant,
      fontFeatures: const [FontFeature.tabularFigures()],
    );
    return PositionBuilder(builder: (context, pos, dur) {
      final known = dur != null && dur > 0;
      final frac = _drag ?? (known ? (pos / dur).clamp(0.0, 1.0) : 0.0);
      final active = _hover || _drag != null;
      return Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          LayoutBuilder(builder: (context, c) {
            final w = c.maxWidth;
            return MouseRegion(
              cursor: known ? SystemMouseCursors.click : MouseCursor.defer,
              onEnter: (_) => setState(() => _hover = true),
              onExit: (_) => setState(() => _hover = false),
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onHorizontalDragStart:
                    known ? (d) => setState(() => _drag = _fraction(d.localPosition, w)) : null,
                onHorizontalDragUpdate:
                    known ? (d) => setState(() => _drag = _fraction(d.localPosition, w)) : null,
                onHorizontalDragEnd: known
                    ? (_) {
                        final f = _drag;
                        setState(() => _drag = null);
                        if (f != null) ref.read(playerViewProvider.notifier).seek(f * dur);
                      }
                    : null,
                onTapUp: known
                    ? (d) => ref
                        .read(playerViewProvider.notifier)
                        .seek(_fraction(d.localPosition, w) * dur)
                    : null,
                child: SizedBox(
                  height: 18,
                  width: w,
                  child: TweenAnimationBuilder<double>(
                    tween: Tween(end: active ? widget.thickness + 2 : widget.thickness),
                    duration: const Duration(milliseconds: 160),
                    builder: (context, h, _) => CustomPaint(
                      painter: _BarPainter(
                        fraction: frac,
                        thickness: h,
                        thumb: active,
                        fill: active ? t.colorScheme.primary : t.colorScheme.onSurface,
                        track: t.colorScheme.onSurface.withValues(alpha: 0.14),
                      ),
                    ),
                  ),
                ),
              ),
            );
          }),
          if (widget.showTimes)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Row(
                children: [
                  Text(formatDuration(_drag != null && known ? _drag! * dur : pos),
                      style: timeStyle),
                  const Spacer(),
                  Text(known ? '-${formatDuration(dur - (_drag ?? frac) * dur)}' : formatDuration(dur),
                      style: timeStyle),
                ],
              ),
            ),
        ],
      );
    });
  }
}

class _BarPainter extends CustomPainter {
  final double fraction;
  final double thickness;
  final bool thumb;
  final Color fill;
  final Color track;

  _BarPainter({
    required this.fraction,
    required this.thickness,
    required this.thumb,
    required this.fill,
    required this.track,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final y = size.height / 2;
    final r = Radius.circular(thickness / 2);
    final full = RRect.fromLTRBR(0, y - thickness / 2, size.width, y + thickness / 2, r);
    canvas.drawRRect(full, Paint()..color = track);
    final x = size.width * fraction;
    if (x > 0) {
      canvas.drawRRect(
        RRect.fromLTRBR(0, y - thickness / 2, x, y + thickness / 2, r),
        Paint()..color = fill,
      );
    }
    if (thumb) {
      canvas.drawCircle(Offset(x, y), thickness + 2.5, Paint()..color = fill);
    }
  }

  @override
  bool shouldRepaint(_BarPainter o) =>
      o.fraction != fraction ||
      o.thickness != thickness ||
      o.thumb != thumb ||
      o.fill != fill ||
      o.track != track;
}

/// A thin, non-interactive progress line (the mini player's top edge).
class ProgressLine extends StatelessWidget {
  const ProgressLine({super.key});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    return PositionBuilder(builder: (context, pos, dur) {
      final f = dur != null && dur > 0 ? (pos / dur).clamp(0.0, 1.0) : 0.0;
      return SizedBox(
        height: 2,
        child: Align(
          alignment: Alignment.centerLeft,
          child: FractionallySizedBox(
            widthFactor: f,
            child: ColoredBox(color: c.primary),
          ),
        ),
      );
    });
  }
}

// ---- buttons -------------------------------------------------------------------

/// Round play/pause; a ring spins while loading.
class PlayPauseButton extends ConsumerWidget {
  final double size;

  /// Filled circle (big players) or a bare icon (mini player).
  final bool filled;

  const PlayPauseButton({super.key, this.size = 56, this.filled = true});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final v = ref.watch(playerViewProvider);
    final c = Theme.of(context).colorScheme;
    final playing = v.playing || v.state.status == PlayStatus.loading;
    final bg = c.onSurface;
    final fg = c.surface;
    final icon = AnimatedSwitcher(
      duration: const Duration(milliseconds: 180),
      transitionBuilder: (child, a) => ScaleTransition(scale: a, child: FadeTransition(opacity: a, child: child)),
      child: Icon(
        playing ? Icons.pause_rounded : Icons.play_arrow_rounded,
        key: ValueKey(playing),
        size: size * (filled ? 0.52 : 0.62),
        color: filled ? fg : c.onSurface,
      ),
    );
    return Semantics(
      button: true,
      label: playing ? 'Пауза' : 'Играть',
      child: Tooltip(
        message: playing ? 'Пауза (пробел)' : 'Играть (пробел)',
        child: SizedBox(
          width: size,
          height: size,
          child: Stack(
            alignment: Alignment.center,
            children: [
              Material(
                color: filled ? bg : Colors.transparent,
                shape: const CircleBorder(),
                clipBehavior: Clip.antiAlias,
                child: InkWell(
                  onTap: v.current == null ? null : () => ref.read(playerViewProvider.notifier).toggle(),
                  child: Center(child: icon),
                ),
              ),
              if (v.busy)
                IgnorePointer(
                  child: SizedBox(
                    width: size + (filled ? 6 : 0),
                    height: size + (filled ? 6 : 0),
                    child: CircularProgressIndicator(
                      strokeWidth: 2,
                      color: c.primary,
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _ToggleIcon extends StatelessWidget {
  final IconData icon;
  final bool on;
  final String tooltip;
  final VoidCallback? onPressed;

  const _ToggleIcon({
    required this.icon,
    required this.on,
    required this.tooltip,
    required this.onPressed,
  });

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    return IconButton(
      tooltip: tooltip,
      onPressed: onPressed,
      icon: Stack(
        clipBehavior: Clip.none,
        alignment: Alignment.center,
        children: [
          Icon(icon, size: 22, color: on ? c.primary : c.onSurfaceVariant),
          if (on)
            Positioned(
              bottom: -7,
              child: Container(
                width: 4,
                height: 4,
                decoration: BoxDecoration(color: c.primary, shape: BoxShape.circle),
              ),
            ),
        ],
      ),
    );
  }
}

/// shuffle · prev · play · next · repeat
class TransportControls extends ConsumerWidget {
  final double playSize;
  final bool showModes;

  const TransportControls({super.key, this.playSize = 64, this.showModes = true});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final v = ref.watch(playerViewProvider);
    final ctl = ref.read(playerViewProvider.notifier);
    final has = v.current != null;
    final skipSize = playSize * 0.5;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (showModes) ...[
          _ToggleIcon(
            icon: Icons.shuffle_rounded,
            on: v.shuffle,
            tooltip: v.shuffle ? 'Перемешивание включено' : 'Перемешать',
            onPressed: has ? () => ctl.setShuffle(!v.shuffle) : null,
          ),
          SizedBox(width: playSize * 0.18),
        ],
        IconButton(
          tooltip: 'Предыдущий',
          iconSize: skipSize,
          onPressed: has ? ctl.prev : null,
          icon: const Icon(Icons.skip_previous_rounded),
        ),
        SizedBox(width: playSize * 0.18),
        PlayPauseButton(size: playSize),
        SizedBox(width: playSize * 0.18),
        IconButton(
          tooltip: 'Следующий',
          iconSize: skipSize,
          onPressed: has ? ctl.next : null,
          icon: const Icon(Icons.skip_next_rounded),
        ),
        if (showModes) ...[
          SizedBox(width: playSize * 0.18),
          _ToggleIcon(
            icon: v.repeat == Repeat.one ? Icons.repeat_one_rounded : Icons.repeat_rounded,
            on: v.repeat != Repeat.off,
            tooltip: switch (v.repeat) {
              Repeat.off => 'Повтор выключен',
              Repeat.all => 'Повтор очереди',
              Repeat.one => 'Повтор трека',
            },
            onPressed: has ? ctl.cycleRepeat : null,
          ),
        ],
      ],
    );
  }
}

/// Heart for [track]; pops when toggled.
class LikeButton extends ConsumerStatefulWidget {
  final PlayerTrack track;
  final double size;
  const LikeButton({super.key, required this.track, this.size = 22});

  @override
  ConsumerState<LikeButton> createState() => _LikeButtonState();
}

class _LikeButtonState extends ConsumerState<LikeButton> {
  int _pops = 0;

  @override
  Widget build(BuildContext context) {
    final liked = ref.watch(trackLikedProvider(widget.track.id)) ?? false;
    final c = Theme.of(context).colorScheme;
    return IconButton(
      tooltip: liked ? 'Убрать из любимых' : 'Нравится',
      onPressed: () {
        setState(() => _pops++);
        ref.read(trackLikedProvider(widget.track.id).notifier).toggle(widget.track);
      },
      icon: TweenAnimationBuilder<double>(
        key: ValueKey(_pops),
        tween: Tween(begin: _pops == 0 ? 1 : 0.6, end: 1),
        duration: const Duration(milliseconds: 420),
        curve: Curves.elasticOut,
        builder: (context, s, child) => Transform.scale(scale: s, child: child),
        child: Icon(
          liked ? Icons.favorite_rounded : Icons.favorite_border_rounded,
          size: widget.size,
          color: liked ? c.primary : c.onSurfaceVariant,
        ),
      ),
    );
  }
}

class VolumeControl extends ConsumerStatefulWidget {
  final double width;
  const VolumeControl({super.key, this.width = 110});

  @override
  ConsumerState<VolumeControl> createState() => _VolumeControlState();
}

class _VolumeControlState extends ConsumerState<VolumeControl> {
  double? _drag;
  DateTime _sent = DateTime(0);

  @override
  Widget build(BuildContext context) {
    final s = ref.watch(playerViewProvider.select((v) => (v.state.volume, v.state.muted)));
    final vol = _drag ?? (s.$2 ? 0.0 : s.$1);
    final ctl = ref.read(playerViewProvider.notifier);
    final c = Theme.of(context).colorScheme;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        IconButton(
          tooltip: s.$2 ? 'Включить звук' : 'Без звука',
          onPressed: ctl.toggleMute,
          icon: Icon(
            vol == 0
                ? Icons.volume_off_rounded
                : vol < 0.5
                    ? Icons.volume_down_rounded
                    : Icons.volume_up_rounded,
            size: 20,
            color: c.onSurfaceVariant,
          ),
        ),
        SizedBox(
          width: widget.width,
          child: Slider(
            value: vol.clamp(0.0, 1.0),
            onChanged: (x) {
              setState(() => _drag = x);
              // Live while dragging, at most ~12 updates a second.
              final now = DateTime.now();
              if (now.difference(_sent) > const Duration(milliseconds: 80)) {
                _sent = now;
                ctl.setVolume(x);
              }
            },
            onChangeEnd: (x) {
              setState(() => _drag = null);
              ctl.setVolume(x);
            },
          ),
        ),
      ],
    );
  }
}
