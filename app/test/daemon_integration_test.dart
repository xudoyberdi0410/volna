/// Against a running daemon (`cargo run -p volna-daemon -- --tray`), only with
/// `VOLNA_LIVE=1` and its socket present: search → album → one cover through
/// `volna://img`.
library;

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:volna/api/catalog.dart';
import 'package:volna/api/models.dart';
import 'package:volna/ipc/proto_image.dart';
import 'package:volna/ipc/socket_transport.dart';
import 'package:volna/ipc/transport.dart';

void main() {
  final path = socketPath();
  // Opt-in, so a plain `flutter test` never talks to the user's own daemon.
  final live = Platform.environment['VOLNA_LIVE'] == '1';
  final running = live &&
      (Platform.isWindows
          ? File(portFilePath(path)).existsSync()
          : FileSystemEntity.typeSync(path) == FileSystemEntityType.unixDomainSock);

  test('search → album → cover', () async {
    final t = SocketTransport()..start();
    addTearDown(t.close);
    final catalog = CatalogApi(t);

    final SearchPage page;
    try {
      page = await catalog.search('Daft Punk Discovery', filter: SearchFilter.albums);
    } on DaemonException catch (e) {
      if (e.message.contains('unknown command')) {
        // An older daemon (installed package) without the native API.
        markTestSkipped('the running daemon has no catalog_* commands');
        return;
      }
      rethrow;
    }
    final summary = page.items.whereType<AlbumItem>().first.album;
    expect(summary.browseId, startsWith('MPRE'));

    final album = await catalog.album(summary.browseId);
    expect(album.title, isNotEmpty);
    expect(album.tracks, isNotEmpty);

    final thumb = bestThumbnail(album.thumbnails)!;
    final images = ProtoImageCache(t, cacheDir: () async => null);
    final bytes = await images.load(volnaImageUri(thumb.url, size: 256));
    expect(bytes.length, greaterThan(100));
    // JPEG, PNG or WebP.
    expect(
      [0xff, 0x89, 0x52].contains(bytes[0]),
      isTrue,
      reason: 'not an image: ${bytes.take(8).toList()}',
    );
  }, skip: running ? false : 'set VOLNA_LIVE=1 with a daemon socket at $path', timeout: const Timeout(Duration(minutes: 5)));
}
