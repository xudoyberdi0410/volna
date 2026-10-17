/// The animated background: the canvas plus soft blobs in the cover's
/// colours that drift slowly and crossfade when the track changes.
library;

import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/color.dart';
import '../../state/palette.dart';
import '../theme/app_theme.dart';

/// Slow ambient animations (background drift, idle wave). Tests turn it off
/// so `pumpAndSettle` settles.
final ambientMotionProvider = Provider<bool>((ref) => true);

/// Three blob colours for [p] on [b].
List<Color> blobColors(Palette? p, Brightness b) {
  final dark = b == Brightness.dark;
  if (p == null) {
    return dark
        ? const [Color(0xff3a2f7a), Color(0xff1d3d6b), Color(0xff4a2350)]
        : const [Color(0xffd9d2fb), Color(0xffcfe0f7), Color(0xfff3d7e6)];
  }
  final sw = [for (final s in p.swatches) s.color];
  Color pick(int i, Color fallback) => i < sw.length ? sw[i] : fallback;
  final raw = [p.dominant, p.accent, pick(1, p.dominant)];
  return [
    for (final c in raw)
      () {
        final hsl = HSLColor.fromColor(c);
        return dark
            ? hsl
                .withLightness(hsl.lightness.clamp(0.16, 0.42))
                .withSaturation((hsl.saturation * 1.1).clamp(0.0, 0.8))
                .toColor()
            : hsl
                .withLightness(hsl.lightness.clamp(0.78, 0.9))
                .withSaturation((hsl.saturation * 0.9).clamp(0.0, 0.7))
                .toColor();
      }(),
  ];
}

class AppBackground extends ConsumerWidget {
  final Widget child;

  /// Stronger colours (fullscreen player).
  final bool vivid;

  const AppBackground({super.key, required this.child, this.vivid = false});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final b = Theme.of(context).brightness;
    final palette = ref.watch(currentPaletteProvider);
    final motion = ref.watch(ambientMotionProvider) && !MediaQuery.of(context).disableAnimations;
    final colors = blobColors(palette, b);
    return TweenAnimationBuilder<_Blobs>(
      tween: _BlobsTween(end: _Blobs(colors)),
      duration: const Duration(milliseconds: 1400),
      curve: Curves.easeInOutCubic,
      builder: (context, blobs, child) => _Drift(
        enabled: motion,
        builder: (t) => CustomPaint(
          painter: _BackgroundPainter(
            base: canvasColor(b),
            blobs: blobs.colors,
            t: t,
            strength: vivid ? 1.0 : 0.75,
            dark: b == Brightness.dark,
          ),
          child: child,
        ),
      ),
      child: RepaintBoundary(child: child),
    );
  }
}

class _Blobs {
  final List<Color> colors;
  const _Blobs(this.colors);
}

class _BlobsTween extends Tween<_Blobs> {
  _BlobsTween({required _Blobs end}) : super(begin: end, end: end);

  @override
  _Blobs lerp(double t) {
    final a = begin!.colors, b = end!.colors;
    return _Blobs([
      for (var i = 0; i < b.length; i++) Color.lerp(i < a.length ? a[i] : b[i], b[i], t)!,
    ]);
  }
}

/// Rebuilds with a slowly advancing phase (0..1 over two minutes).
class _Drift extends StatefulWidget {
  final bool enabled;
  final Widget Function(double t) builder;
  const _Drift({required this.enabled, required this.builder});

  @override
  State<_Drift> createState() => _DriftState();
}

class _DriftState extends State<_Drift> with SingleTickerProviderStateMixin {
  // ~15 repaints a second is plenty for a two-minute drift.
  static const _steps = 120 * 15;
  late final AnimationController _c =
      AnimationController(vsync: this, duration: const Duration(minutes: 2));
  final _phase = ValueNotifier<double>(0);

  @override
  void initState() {
    super.initState();
    _c.addListener(() {
      final q = (_c.value * _steps).floor() / _steps;
      if (q != _phase.value) _phase.value = q;
    });
    _sync();
  }

  @override
  void didUpdateWidget(_Drift old) {
    super.didUpdateWidget(old);
    _sync();
  }

  void _sync() {
    if (widget.enabled && !_c.isAnimating) {
      _c.repeat();
    } else if (!widget.enabled && _c.isAnimating) {
      _c.stop();
    }
  }

  @override
  void dispose() {
    _c.dispose();
    _phase.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) =>
      ValueListenableBuilder(valueListenable: _phase, builder: (_, t, _) => widget.builder(t));
}

class _BackgroundPainter extends CustomPainter {
  final Color base;
  final List<Color> blobs;
  final double t;
  final double strength;
  final bool dark;

  _BackgroundPainter({
    required this.base,
    required this.blobs,
    required this.t,
    required this.strength,
    required this.dark,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    canvas.drawRect(rect, Paint()..color = base);
    final w = size.width, h = size.height;
    final r = math.max(w, h);
    final a = t * 2 * math.pi;
    // Anchors drift on slow Lissajous paths.
    final centers = [
      Offset(w * (0.18 + 0.08 * math.sin(a)), h * (0.12 + 0.07 * math.cos(a * 2))),
      Offset(w * (0.86 + 0.06 * math.cos(a)), h * (0.28 + 0.1 * math.sin(a))),
      Offset(w * (0.5 + 0.12 * math.sin(a + 1.7)), h * (1.0 + 0.06 * math.cos(a))),
    ];
    final radii = [r * 0.62, r * 0.5, r * 0.66];
    final alpha = (dark ? 0.85 : 0.9) * strength;
    for (var i = 0; i < blobs.length && i < centers.length; i++) {
      final c = blobs[i];
      final shader = RadialGradient(
        colors: [c.withValues(alpha: alpha), c.withValues(alpha: alpha * 0.35), c.withValues(alpha: 0)],
        stops: const [0, 0.45, 1],
      ).createShader(Rect.fromCircle(center: centers[i], radius: radii[i]));
      canvas.drawRect(rect, Paint()..shader = shader);
    }
    // A soft vignette keeps text readable on bright covers.
    final scrim = LinearGradient(
      begin: Alignment.topCenter,
      end: Alignment.bottomCenter,
      colors: [
        base.withValues(alpha: dark ? 0.0 : 0.0),
        base.withValues(alpha: dark ? 0.35 : 0.25),
      ],
    ).createShader(rect);
    canvas.drawRect(rect, Paint()..shader = scrim);
  }

  @override
  bool shouldRepaint(_BackgroundPainter old) =>
      old.t != t || old.base != base || old.strength != strength || !_same(old.blobs, blobs);

  static bool _same(List<Color> a, List<Color> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }
}
