/// `color_palette{imageUri}` (`crates/daemon/src/api/color.rs`): dominant and
/// accent colours of a cover, computed in the daemon and cached per URL.
library;

import 'dart:ui' show Color;

import '../ipc/transport.dart';
import '_json.dart';

/// `#rrggbb` → [Color]; null when malformed.
Color? parseHexColor(Object? s) {
  if (s is! String) return null;
  final h = s.startsWith('#') ? s.substring(1) : s;
  if (h.length != 6) return null;
  final v = int.tryParse(h, radix: 16);
  return v == null ? null : Color(0xff000000 | v);
}

class Swatch {
  final Color color;

  /// 0..1; all swatches sum to 1.
  final double share;
  const Swatch(this.color, this.share);
}

class Palette {
  final Color dominant;
  final Color accent;
  final Color onDominant;
  final Color onAccent;

  /// Whether [dominant] reads as dark (white text on it).
  final bool dark;

  /// By share, largest first.
  final List<Swatch> swatches;

  const Palette({
    required this.dominant,
    required this.accent,
    required this.onDominant,
    required this.onAccent,
    required this.dark,
    this.swatches = const [],
  });

  factory Palette.fromJson(Json j) {
    const black = Color(0xff000000), white = Color(0xffffffff);
    return Palette(
      dominant: parseHexColor(j['dominant']) ?? black,
      accent: parseHexColor(j['accent']) ?? white,
      onDominant: parseHexColor(j['onDominant']) ?? white,
      onAccent: parseHexColor(j['onAccent']) ?? black,
      dark: j['dark'] as bool? ?? true,
      swatches: [
        for (final s in (j['swatches'] as List? ?? const []))
          if (parseHexColor((s as Json)['color']) case final c?)
            Swatch(c, asDouble(s['share']) ?? 0),
      ],
    );
  }
}

class ColorApi {
  final Transport _t;
  const ColorApi(this._t);

  /// [imageUri]: a `volna://img/?u=…` URI or the plain cover URL.
  Future<Palette> palette(String imageUri) async =>
      Palette.fromJson((await _t.call('color_palette', {'imageUri': imageUri})) as Json);
}
