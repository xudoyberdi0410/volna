/// Images through the daemon's `volna://img/` route, so the app proxy
/// applies (`crates/daemon/src/media.rs::image`): `volna://img/?u=<url>&s=<px>`.
///
/// [ProtoImageCache] keeps bytes in a memory LRU and a disk LRU; [VolnaImage]
/// is the [ImageProvider] on top of it. Flutter's own [ImageCache] still
/// holds the decoded images.
library;

import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/painting.dart';
import 'package:path_provider/path_provider.dart';

import 'transport.dart';

/// A `volna://img/` URI for [url]; `volna://…` URIs pass through. [size]
/// asks the daemon for a thumbnail of about that many pixels.
String volnaImageUri(String url, {int? size}) {
  if (url.startsWith('volna://')) return url;
  final s = size == null ? '' : '&s=$size';
  return 'volna://img/?u=${Uri.encodeComponent(url)}$s';
}

/// 64-bit FNV-1a as hex: a stable file name for a URI (native ints wrap).
String _fileKey(String s) {
  var h = 0xcbf29ce484222325;
  for (final b in s.codeUnits) {
    h = (h ^ b) * 0x100000001b3;
  }
  String half(int v) => v.toRadixString(16).padLeft(8, '0');
  return half(h >>> 32) + half(h & 0xffffffff);
}

class ProtoImageCache {
  final Transport transport;
  final int maxMemoryBytes;
  final int maxDiskBytes;
  final Future<Directory?> Function() _dirFactory;

  ProtoImageCache(
    this.transport, {
    this.maxMemoryBytes = 32 << 20,
    this.maxDiskBytes = 256 << 20,
    Future<Directory?> Function()? cacheDir,
  }) : _dirFactory = cacheDir ?? _defaultDir;

  static Future<Directory?> _defaultDir() async {
    try {
      final base = await getApplicationCacheDirectory();
      return Directory('${base.path}${Platform.pathSeparator}images');
    } catch (_) {
      return null; // No disk cache (tests, unsupported platform).
    }
  }

  // Insertion order = recency (oldest first).
  final _memory = <String, Uint8List>{};
  int _memoryBytes = 0;
  final _inflight = <String, Future<Uint8List>>{};
  Future<Directory?>? _dir;
  int _writesSinceTrim = 0;

  Future<Directory?> get _directory => _dir ??= () async {
        final d = await _dirFactory();
        if (d == null) return null;
        try {
          await d.create(recursive: true);
          unawaited(_trimDisk(d));
          return d;
        } catch (_) {
          return null;
        }
      }();

  /// Bytes of [uri] (a `volna://` URI). Throws on a non-2xx answer.
  Future<Uint8List> load(String uri) {
    final hit = _memory.remove(uri);
    if (hit != null) {
      _memory[uri] = hit;
      return SynchronousFuture(hit);
    }
    // A block body: returning the removed future would make it wait on itself.
    return _inflight[uri] ??= _fetch(uri).whenComplete(() {
      _inflight.remove(uri);
    });
  }

  Future<Uint8List> _fetch(String uri) async {
    final dir = await _directory;
    final file = dir == null ? null : File('${dir.path}${Platform.pathSeparator}${_fileKey(uri)}');
    if (file != null) {
      try {
        final bytes = await file.readAsBytes();
        if (bytes.isNotEmpty) {
          unawaited(file.setLastModified(DateTime.now()).catchError((_) {}));
          _remember(uri, bytes);
          return bytes;
        }
      } on FileSystemException {
        // Not cached.
      }
    }
    final r = await transport.proto('GET', uri);
    if (!r.ok) {
      throw HttpException('volna image ${r.status}', uri: Uri.tryParse(uri));
    }
    _remember(uri, r.body);
    if (file != null && dir != null) {
      unawaited(_store(file, dir, r.body));
    }
    return r.body;
  }

  void _remember(String uri, Uint8List bytes) {
    if (bytes.length > maxMemoryBytes) return;
    _memory[uri] = bytes;
    _memoryBytes += bytes.length;
    while (_memoryBytes > maxMemoryBytes && _memory.isNotEmpty) {
      final k = _memory.keys.first;
      _memoryBytes -= _memory.remove(k)!.length;
    }
  }

  Future<void> _store(File file, Directory dir, Uint8List bytes) async {
    try {
      final tmp = File('${file.path}.tmp');
      await tmp.writeAsBytes(bytes, flush: true);
      await tmp.rename(file.path);
      if (++_writesSinceTrim >= 50) {
        _writesSinceTrim = 0;
        await _trimDisk(dir);
      }
    } catch (_) {
      // A cache write failing is not an error worth surfacing.
    }
  }

  /// Deletes the least recently used files until under [maxDiskBytes].
  Future<void> _trimDisk(Directory dir) async {
    try {
      final files = <(File, int, DateTime)>[];
      var total = 0;
      await for (final e in dir.list()) {
        if (e is! File) continue;
        final st = await e.stat();
        files.add((e, st.size, st.modified));
        total += st.size;
      }
      if (total <= maxDiskBytes) return;
      files.sort((a, b) => a.$3.compareTo(b.$3));
      for (final (f, size, _) in files) {
        if (total <= maxDiskBytes) break;
        try {
          await f.delete();
          total -= size;
        } catch (_) {}
      }
    } catch (_) {}
  }

  /// Drops the memory cache (the disk cache stays).
  void clearMemory() {
    _memory.clear();
    _memoryBytes = 0;
  }
}

/// An [ImageProvider] for catalog images (`https://lh3…`, `https://i.ytimg…`)
/// or `volna://` URIs, loaded through [cache].
@immutable
class VolnaImage extends ImageProvider<VolnaImage> {
  final String uri;
  final double scale;
  final ProtoImageCache cache;

  VolnaImage(String url, {required this.cache, int? size, this.scale = 1.0})
      : uri = volnaImageUri(url, size: size);

  @override
  Future<VolnaImage> obtainKey(ImageConfiguration configuration) =>
      SynchronousFuture<VolnaImage>(this);

  @override
  ImageStreamCompleter loadImage(VolnaImage key, ImageDecoderCallback decode) =>
      MultiFrameImageStreamCompleter(
        codec: _decode(key, decode),
        scale: key.scale,
        debugLabel: key.uri,
        informationCollector: () => [DiagnosticsProperty<String>('URI', key.uri)],
      );

  static Future<ui.Codec> _decode(VolnaImage key, ImageDecoderCallback decode) async {
    final bytes = await key.cache.load(key.uri);
    return decode(await ui.ImmutableBuffer.fromUint8List(bytes));
  }

  @override
  bool operator ==(Object other) =>
      other is VolnaImage && other.uri == uri && other.scale == scale;

  @override
  int get hashCode => Object.hash(uri, scale);

  @override
  String toString() => 'VolnaImage("$uri", scale: $scale)';
}
