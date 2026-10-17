/// «Моя волна»: the default screen. Idle, it invites to start the wave; while
/// the wave plays, it shows the track big with dislike / play / skip / like.
library;

import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/player_models.dart';
import '../../state/player.dart';
import '../nav.dart';
import '../player/controls.dart';
import '../player/cover.dart';
import '../player/track_title.dart';
import '../shell/background.dart';

class WaveScreen extends ConsumerStatefulWidget {
  const WaveScreen({super.key});

  @override
  ConsumerState<WaveScreen> createState() => _WaveScreenState();
}

class _WaveScreenState extends ConsumerState<WaveScreen> {
  bool _starting = false;

  Future<void> _start() async {
    setState(() => _starting = true);
    await ref.read(playerViewProvider.notifier).waveStart();
    if (mounted) setState(() => _starting = false);
  }

  @override
  Widget build(BuildContext context) {
    final active = ref.watch(playerViewProvider.select((v) => v.waveActive));
    final track = ref.watch(playerViewProvider.select((v) => v.current));
    final showTrack = active && track != null;
    return AnimatedSwitcher(
      duration: const Duration(milliseconds: 500),
      switchInCurve: Curves.easeOutCubic,
      switchOutCurve: Curves.easeInCubic,
      transitionBuilder: (child, a) => FadeTransition(
        opacity: a,
        child: ScaleTransition(scale: Tween(begin: 0.97, end: 1.0).animate(a), child: child),
      ),
      child: showTrack
          ? _WavePlaying(key: const ValueKey('playing'), track: track)
          : _WaveIdle(
              key: const ValueKey('idle'),
              starting: _starting || (active && track == null),
              onStart: _start,
            ),
    );
  }
}

class _WaveIdle extends ConsumerWidget {
  final bool starting;
  final VoidCallback onStart;
  const _WaveIdle({super.key, required this.starting, required this.onStart});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = Theme.of(context);
    final other = ref.watch(playerViewProvider.select((v) => v.current != null));
    return LayoutBuilder(builder: (context, c) {
      return Stack(
        children: [
          Positioned.fill(
            child: IgnorePointer(child: WaveLines(color: t.colorScheme.primary)),
          ),
          Center(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 32),
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 520),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text('Моя волна',
                        textAlign: TextAlign.center,
                        style: t.textTheme.displayMedium),
                    const SizedBox(height: 14),
                    Text(
                      'Бесконечный поток музыки под ваш вкус. '
                      'Лайкайте то, что нравится, и пропускайте лишнее: волна подстроится.',
                      textAlign: TextAlign.center,
                      style: t.textTheme.bodyLarge?.copyWith(color: t.colorScheme.onSurfaceVariant),
                    ),
                    const SizedBox(height: 32),
                    SizedBox(
                      height: 52,
                      child: FilledButton.icon(
                        onPressed: starting ? null : onStart,
                        style: FilledButton.styleFrom(
                          backgroundColor: t.colorScheme.onSurface,
                          foregroundColor: t.colorScheme.surface,
                          disabledBackgroundColor: t.colorScheme.onSurface.withValues(alpha: 0.5),
                          disabledForegroundColor: t.colorScheme.surface,
                          padding: const EdgeInsets.symmetric(horizontal: 28),
                          textStyle: t.textTheme.titleMedium,
                        ),
                        icon: starting
                            ? SizedBox(
                                width: 18,
                                height: 18,
                                child: CircularProgressIndicator(
                                    strokeWidth: 2, color: t.colorScheme.surface),
                              )
                            : const Icon(Icons.play_arrow_rounded, size: 24),
                        label: Text(starting ? 'Подбираю треки…' : 'Запустить волну'),
                      ),
                    ),
                    const SizedBox(height: 12),
                    TextButton.icon(
                      onPressed: () => openSettings(context, 'wave'),
                      icon: const Icon(Icons.tune_rounded, size: 18),
                      label: const Text('Настроить волну'),
                    ),
                    if (other) ...[
                      const SizedBox(height: 18),
                      Text('Сейчас играет очередь. Волна заменит её.',
                          style: t.textTheme.bodySmall
                              ?.copyWith(color: t.colorScheme.onSurfaceVariant)),
                    ],
                  ],
                ),
              ),
            ),
          ),
        ],
      );
    });
  }
}

class _WavePlaying extends ConsumerWidget {
  final PlayerTrack track;
  const _WavePlaying({super.key, required this.track});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = Theme.of(context);
    final ctl = ref.read(playerViewProvider.notifier);
    final next = ref.watch(playerViewProvider.select((v) => v.upNext.take(4).toList()));
    return LayoutBuilder(builder: (context, c) {
      // Title, seek bar, buttons and margins take ~340 px; "next up" ~160.
      final room = c.maxHeight - 340;
      final withNext = next.isNotEmpty && room - 160 >= 300 && c.maxWidth >= 560;
      final upNext = withNext ? next : const <PlayerTrack>[];
      final cover =
          math.min(math.min(c.maxWidth - 80, withNext ? room - 160 : room), 420.0).clamp(160.0, 420.0);
      return Stack(
        children: [
          Positioned(
            top: 16,
            right: 20,
            child: Row(
              children: [
                TextButton.icon(
                  onPressed: () => openSettings(context, 'wave'),
                  icon: const Icon(Icons.tune_rounded, size: 18),
                  label: const Text('Настроить'),
                ),
                const SizedBox(width: 4),
                TextButton.icon(
                  onPressed: ctl.waveStop,
                  icon: const Icon(Icons.stop_rounded, size: 18),
                  label: const Text('Остановить'),
                ),
              ],
            ),
          ),
          Positioned(
            top: 26,
            left: 32,
            child: Row(
              children: [
                Icon(Icons.waves_rounded, size: 18, color: t.colorScheme.primary),
                const SizedBox(width: 8),
                Text('МОЯ ВОЛНА',
                    style: t.textTheme.labelSmall?.copyWith(
                        color: t.colorScheme.onSurfaceVariant, letterSpacing: 1.6)),
              ],
            ),
          ),
          Center(
            child: SingleChildScrollView(
              padding: const EdgeInsets.only(top: 64, bottom: 24),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  GestureDetector(
                    onTap: () => openPlayer(context),
                    child: MouseRegion(
                      cursor: SystemMouseCursors.click,
                      child: CoverArt(url: track.cover, size: cover, radius: 20, elevated: true),
                    ),
                  ),
                  const SizedBox(height: 30),
                  SizedBox(
                    width: math.max(cover + 120, 360),
                    child: TrackTitle(
                      track: track,
                      align: CrossAxisAlignment.center,
                      titleStyle: t.textTheme.headlineMedium,
                      artistStyle: t.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.w500),
                    ),
                  ),
                  const SizedBox(height: 18),
                  SizedBox(width: cover, child: const SeekBar()),
                  const SizedBox(height: 14),
                  Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      _RoundAction(
                        icon: Icons.thumb_down_alt_outlined,
                        tooltip: 'Не нравится — пропустить',
                        onPressed: ctl.waveDislike,
                      ),
                      const SizedBox(width: 20),
                      const PlayPauseButton(size: 72),
                      const SizedBox(width: 20),
                      _RoundAction(
                        icon: Icons.skip_next_rounded,
                        tooltip: 'Следующий',
                        onPressed: ctl.next,
                      ),
                      const SizedBox(width: 8),
                      LikeButton(track: track, size: 28),
                    ],
                  ),
                  if (upNext.isNotEmpty) ...[
                    const SizedBox(height: 36),
                    _UpNext(tracks: upNext),
                  ],
                ],
              ),
            ),
          ),
        ],
      );
    });
  }
}

class _RoundAction extends StatelessWidget {
  final IconData icon;
  final String tooltip;
  final VoidCallback onPressed;
  const _RoundAction({required this.icon, required this.tooltip, required this.onPressed});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    return IconButton(
      tooltip: tooltip,
      onPressed: onPressed,
      style: IconButton.styleFrom(
        backgroundColor: c.onSurface.withValues(alpha: 0.08),
        fixedSize: const Size(52, 52),
      ),
      icon: Icon(icon, size: 24),
    );
  }
}

class _UpNext extends StatelessWidget {
  final List<PlayerTrack> tracks;
  const _UpNext({required this.tracks});

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    return Column(
      children: [
        Text('ДАЛЬШЕ',
            style: t.textTheme.labelSmall
                ?.copyWith(color: t.colorScheme.onSurfaceVariant, letterSpacing: 1.6)),
        const SizedBox(height: 12),
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final tr in tracks)
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8),
                child: Tooltip(
                  message: '${tr.title} — ${tr.artistNames}',
                  child: SizedBox(
                    width: 92,
                    child: Column(
                      children: [
                        CoverArt(url: tr.cover, size: 92, radius: 10),
                        const SizedBox(height: 8),
                        Text(tr.title,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            textAlign: TextAlign.center,
                            style: t.textTheme.bodySmall),
                      ],
                    ),
                  ),
                ),
              ),
          ],
        ),
      ],
    );
  }
}

/// Layered sine waves drifting across the idle wave screen.
class WaveLines extends ConsumerStatefulWidget {
  final Color color;
  const WaveLines({super.key, required this.color});

  @override
  ConsumerState<WaveLines> createState() => _WaveLinesState();
}

class _WaveLinesState extends ConsumerState<WaveLines> with SingleTickerProviderStateMixin {
  late final AnimationController _c =
      AnimationController(vsync: this, duration: const Duration(seconds: 18));

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final motion = ref.watch(ambientMotionProvider) && !MediaQuery.of(context).disableAnimations;
    if (motion && !_c.isAnimating) {
      _c.repeat();
    } else if (!motion && _c.isAnimating) {
      _c.stop();
    }
    return AnimatedBuilder(
      animation: _c,
      builder: (context, _) => CustomPaint(
        painter: _WavesPainter(phase: _c.value, color: widget.color),
      ),
    );
  }
}

class _WavesPainter extends CustomPainter {
  final double phase;
  final Color color;
  _WavesPainter({required this.phase, required this.color});

  @override
  void paint(Canvas canvas, Size size) {
    final w = size.width, h = size.height;
    const layers = 5;
    for (var l = 0; l < layers; l++) {
      final k = l / (layers - 1);
      final amp = h * (0.05 + 0.03 * k);
      final y0 = h * (0.72 + 0.045 * l);
      final freq = 1.3 + 0.35 * l;
      final shift = phase * 2 * math.pi * (l.isEven ? 1 : -1) + l * 0.9;
      final path = Path()..moveTo(0, y0);
      for (double x = 0; x <= w; x += 6) {
        final u = x / w;
        final y = y0 +
            amp * math.sin(u * freq * 2 * math.pi + shift) +
            amp * 0.35 * math.sin(u * freq * 5.1 * math.pi - shift * 1.3);
        path.lineTo(x, y);
      }
      path
        ..lineTo(w, h)
        ..lineTo(0, h)
        ..close();
      canvas.drawPath(
        path,
        Paint()
          ..shader = LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: [
              color.withValues(alpha: 0.16 - 0.02 * l),
              color.withValues(alpha: 0.0),
            ],
          ).createShader(Rect.fromLTWH(0, y0 - amp, w, h - y0 + amp)),
      );
    }
  }

  @override
  bool shouldRepaint(_WavesPainter o) => o.phase != phase || o.color != color;
}
