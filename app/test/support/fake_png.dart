/// Deterministic cover images for tests: a two-colour diagonal gradient
/// with a soft circle, tinted by a hash of the URL, encoded as PNG.
library;

import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

final _crcTable = List<int>.generate(256, (n) {
  var c = n;
  for (var k = 0; k < 8; k++) {
    c = (c & 1) != 0 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  }
  return c;
});

int _crc(List<int> bytes) {
  var c = 0xffffffff;
  for (final b in bytes) {
    c = _crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
  }
  return c ^ 0xffffffff;
}

void _chunk(BytesBuilder out, String type, List<int> data) {
  final len = ByteData(4)..setUint32(0, data.length);
  out.add(len.buffer.asUint8List());
  final td = [...type.codeUnits, ...data];
  out.add(td);
  final crc = ByteData(4)..setUint32(0, _crc(td));
  out.add(crc.buffer.asUint8List());
}

(int, int, int) _hsl(double h, double s, double l) {
  final c = (1 - (2 * l - 1).abs()) * s;
  final x = c * (1 - ((h / 60) % 2 - 1).abs());
  final m = l - c / 2;
  final (r, g, b) = switch (h ~/ 60) {
    0 => (c, x, 0.0),
    1 => (x, c, 0.0),
    2 => (0.0, c, x),
    3 => (0.0, x, c),
    4 => (x, 0.0, c),
    _ => (c, 0.0, x),
  };
  int v(double t) => ((t + m) * 255).round().clamp(0, 255);
  return (v(r), v(g), v(b));
}

int _hash(String s) {
  var h = 0x811c9dc5;
  for (final c in s.codeUnits) {
    h = ((h ^ c) * 0x01000193) & 0xffffffff;
  }
  return h;
}

final _cache = <String, Uint8List>{};

/// A [size]² PNG for [seed].
Uint8List fakeCover(String seed, {int size = 64}) => _cache[seed] ??= () {
      final h = _hash(seed);
      final hue = (h % 360).toDouble();
      final a = _hsl(hue, 0.55, 0.42);
      final b = _hsl((hue + 50 + (h >> 9) % 80) % 360, 0.6, 0.22);
      final cx = size * (0.3 + ((h >> 4) % 40) / 100), cy = size * (0.3 + ((h >> 12) % 40) / 100);
      final rad = size * 0.28;
      final raw = BytesBuilder();
      for (var y = 0; y < size; y++) {
        raw.addByte(0);
        for (var x = 0; x < size; x++) {
          final t = (x + y) / (2 * size - 2);
          var r = a.$1 + (b.$1 - a.$1) * t, g = a.$2 + (b.$2 - a.$2) * t, bl = a.$3 + (b.$3 - a.$3) * t;
          final d = sqrt(pow(x - cx, 2) + pow(y - cy, 2));
          final glow = (1 - (d / rad)).clamp(0.0, 1.0) * 0.35;
          r += (255 - r) * glow;
          g += (255 - g) * glow;
          bl += (255 - bl) * glow;
          raw.add([r.round(), g.round(), bl.round()]);
        }
      }
      final out = BytesBuilder()..add([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
      final ihdr = ByteData(13)
        ..setUint32(0, size)
        ..setUint32(4, size)
        ..setUint8(8, 8)
        ..setUint8(9, 2);
      _chunk(out, 'IHDR', ihdr.buffer.asUint8List());
      _chunk(out, 'IDAT', ZLibEncoder().convert(raw.takeBytes()));
      _chunk(out, 'IEND', const []);
      return out.takeBytes();
    }();
