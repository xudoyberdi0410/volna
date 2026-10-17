/// Cover art through the daemon (`volna://img/`), with a tinted placeholder
/// while loading and when there is no image.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/models.dart';
import '../../ipc/proto_image.dart';
import '../../state/connection.dart';

/// One image cache per transport.
final protoImageCacheProvider =
    Provider<ProtoImageCache>((ref) => ProtoImageCache(ref.watch(transportProvider)));

/// Pixel size buckets asked from the daemon, so one cover is cached in few sizes.
int _bucket(double px) {
  for (final b in const [96, 192, 320, 544, 800]) {
    if (px <= b) return b;
  }
  return 1080;
}

enum CoverShape { rounded, circle }

class CoverImage extends ConsumerWidget {
  /// A catalog URL (`https://lh3…`) or a `volna://` URI; null shows the placeholder.
  final String? url;
  final double size;
  final CoverShape shape;
  final double radius;

  /// Placeholder icon.
  final IconData icon;

  /// Seeds the placeholder tint (an id or title), so rows don't look all the same.
  final String? seed;

  const CoverImage({
    super.key,
    required this.url,
    this.size = 48,
    this.shape = CoverShape.rounded,
    this.radius = 6,
    this.icon = Icons.music_note_rounded,
    this.seed,
  });

  CoverImage.thumbs(
    List<Thumbnail> thumbnails, {
    Key? key,
    double size = 48,
    CoverShape shape = CoverShape.rounded,
    double radius = 6,
    IconData icon = Icons.music_note_rounded,
    String? seed,
  }) : this(
          key: key,
          url: bestThumbnail(thumbnails)?.url,
          size: size,
          shape: shape,
          radius: radius,
          icon: icon,
          seed: seed,
        );

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final placeholder = CoverPlaceholder(icon: icon, seed: seed ?? url, size: size);
    Widget child = placeholder;
    final u = url;
    if (u != null && u.isNotEmpty) {
      final dpr = MediaQuery.maybeDevicePixelRatioOf(context) ?? 1;
      child = Image(
        image: VolnaImage(u, cache: ref.watch(protoImageCacheProvider), size: _bucket(size * dpr)),
        width: size,
        height: size,
        fit: BoxFit.cover,
        gaplessPlayback: true,
        frameBuilder: (context, img, frame, sync) => sync
            ? img
            : Stack(fit: StackFit.passthrough, children: [
                placeholder,
                AnimatedOpacity(
                  opacity: frame == null ? 0 : 1,
                  duration: const Duration(milliseconds: 220),
                  curve: Curves.easeOut,
                  child: img,
                ),
              ]),
        errorBuilder: (_, _, _) => placeholder,
      );
    }
    final clipped = shape == CoverShape.circle
        ? ClipOval(child: child)
        : ClipRRect(borderRadius: BorderRadius.circular(radius), child: child);
    return SizedBox.square(dimension: size, child: clipped);
  }
}

/// A soft gradient tile with an icon, tinted from [seed].
class CoverPlaceholder extends StatelessWidget {
  final IconData icon;
  final String? seed;
  final double size;
  const CoverPlaceholder({super.key, required this.icon, this.seed, required this.size});

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    final hue = ((seed?.hashCode ?? 0) % 360).abs().toDouble();
    final dark = Theme.of(context).brightness == Brightness.dark;
    final a = HSLColor.fromAHSL(1, hue, 0.35, dark ? 0.22 : 0.84).toColor();
    final b = HSLColor.fromAHSL(1, (hue + 40) % 360, 0.30, dark ? 0.15 : 0.76).toColor();
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        gradient: LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [a, b],
        ),
      ),
      alignment: Alignment.center,
      child: Icon(icon, size: size * 0.38, color: cs.onSurface.withValues(alpha: 0.35)),
    );
  }
}

/// Up to four covers in a 2×2 grid (user playlists without a custom cover).
class CoverCollage extends StatelessWidget {
  final List<String> urls;
  final double size;
  final double radius;
  final String? seed;
  const CoverCollage({super.key, required this.urls, this.size = 48, this.radius = 6, this.seed});

  @override
  Widget build(BuildContext context) {
    if (urls.length < 4) {
      return CoverImage(
        url: urls.firstOrNull,
        size: size,
        radius: radius,
        icon: Icons.queue_music_rounded,
        seed: seed,
      );
    }
    final half = size / 2;
    return ClipRRect(
      borderRadius: BorderRadius.circular(radius),
      child: SizedBox.square(
        dimension: size,
        child: Wrap(children: [
          for (final u in urls.take(4)) CoverImage(url: u, size: half, radius: 0),
        ]),
      ),
    );
  }
}
