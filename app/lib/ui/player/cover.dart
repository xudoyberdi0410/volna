/// Cover art through the daemon (`volna://img/`), with a quiet placeholder
/// and a crossfade when the URL changes.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../ipc/proto_image.dart';
import '../../state/palette.dart';

/// Thumbnail sizes asked from the daemon; few buckets keep the cache warm.
int coverBucket(double px) {
  for (final b in const [96, 226, 544, 800]) {
    if (px <= b) return b;
  }
  return 1200;
}

class CoverArt extends ConsumerWidget {
  final String? url;
  final double size;
  final double radius;

  /// Shadow under the cover (big covers).
  final bool elevated;
  final Duration fade;

  const CoverArt({
    super.key,
    required this.url,
    required this.size,
    this.radius = 10,
    this.elevated = false,
    this.fade = const Duration(milliseconds: 450),
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final dpr = MediaQuery.maybeDevicePixelRatioOf(context) ?? 1;
    final u = url;
    final scheme = Theme.of(context).colorScheme;
    final placeholder = _Placeholder(size: size);
    Widget image = u == null || u.isEmpty
        ? placeholder
        : Image(
            key: ValueKey(u),
            image: VolnaImage(u, cache: ref.watch(imageCacheProvider), size: coverBucket(size * dpr)),
            width: size,
            height: size,
            fit: BoxFit.cover,
            gaplessPlayback: true,
            filterQuality: FilterQuality.medium,
            frameBuilder: (context, child, frame, sync) => sync
                ? child
                : AnimatedOpacity(
                    opacity: frame == null ? 0 : 1,
                    duration: const Duration(milliseconds: 260),
                    curve: Curves.easeOut,
                    child: child,
                  ),
            errorBuilder: (_, _, _) => placeholder,
          );
    image = AnimatedSwitcher(
      duration: fade,
      switchInCurve: Curves.easeOut,
      switchOutCurve: Curves.easeIn,
      layoutBuilder: (current, previous) => Stack(
        alignment: Alignment.center,
        children: [...previous, ?current],
      ),
      child: KeyedSubtree(key: ValueKey(u), child: image),
    );
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(radius),
        color: scheme.surfaceContainerHigh,
        boxShadow: elevated
            ? [
                BoxShadow(
                  color: Colors.black.withValues(alpha: 0.35),
                  blurRadius: size * 0.12,
                  offset: Offset(0, size * 0.05),
                ),
              ]
            : null,
      ),
      clipBehavior: Clip.antiAlias,
      child: image,
    );
  }
}

class _Placeholder extends StatelessWidget {
  final double size;
  const _Placeholder({required this.size});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        gradient: LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [c.primary.withValues(alpha: 0.28), c.primary.withValues(alpha: 0.08)],
        ),
      ),
      child: Icon(Icons.music_note_rounded,
          size: size * 0.36, color: c.onSurface.withValues(alpha: 0.35)),
    );
  }
}
