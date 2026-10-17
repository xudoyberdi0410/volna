/// Cover colours: [paletteProvider] per cover URL (the daemon computes and
/// caches them), [currentPaletteProvider] for the playing track. The current
/// palette keeps the previous one until the next has arrived, so the
/// background never flashes to grey between tracks.
library;

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/color.dart';
import '../ipc/proto_image.dart';
import '../ipc/transport.dart';
import 'connection.dart';
import 'player.dart';
import 'theme.dart';

final colorApiProvider = Provider((ref) => ColorApi(ref.watch(transportProvider)));

/// Image bytes through `volna://img/` (proxy applies), memory + disk LRU.
final imageCacheProvider = Provider((ref) => ProtoImageCache(ref.watch(transportProvider)));

final paletteProvider = FutureProvider.family<Palette?, String>((ref, cover) async {
  try {
    return await ref.watch(colorApiProvider).palette(cover);
  } on DaemonException {
    return null;
  }
});

class _Last {
  Palette? value;
}

// Survives rebuilds of [currentPaletteProvider] (notifiers may be recreated).
final _lastPalette = Provider((ref) => _Last());

final currentPaletteProvider = Provider<Palette?>((ref) {
  final last = ref.read(_lastPalette);
  if (!ref.watch(coverColorsProvider)) return last.value = null;
  final cover = ref.watch(playerViewProvider.select((v) => v.current?.cover));
  if (cover == null || cover.isEmpty) return last.value;
  final p = ref.watch(paletteProvider(cover)).value;
  if (p != null) last.value = p;
  return last.value;
});
