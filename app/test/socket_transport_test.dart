import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:volna/api/catalog.dart';
import 'package:volna/api/models.dart';
import 'package:volna/ipc/frame.dart';
import 'package:volna/ipc/socket_transport.dart';
import 'package:volna/ipc/transport.dart';

/// A fake daemon on a Unix socket: answers hello, then hands frames to [onFrame].
class FakeDaemon {
  final ServerSocket server;
  final String path;
  final void Function(Frame f, void Function(Frame) send) onFrame;
  final sockets = <Socket>[];
  final hellos = <Hello>[];

  FakeDaemon._(this.server, this.path, this.onFrame) {
    server.listen((s) {
      sockets.add(s);
      final d = FrameDecoder();
      var greeted = false;
      void send(Frame f) => s.add(encodeFrame(f));
      s.listen((chunk) {
        for (final f in d.add(chunk)) {
          if (!greeted) {
            hellos.add(f.msg as Hello);
            greeted = true;
            send(Frame(const Welcome('http://127.0.0.1:9/')));
            continue;
          }
          onFrame(f, send);
        }
      }, onError: (_) {});
    });
  }

  static Future<FakeDaemon> start(String path, void Function(Frame, void Function(Frame)) onFrame) async {
    final s = await ServerSocket.bind(InternetAddress(path, type: InternetAddressType.unix), 0);
    return FakeDaemon._(s, path, onFrame);
  }

  Future<void> close() async {
    for (final s in sockets) {
      s.destroy();
    }
    await server.close();
  }
}

void main() {
  group('endpoint', () {
    test('VOLNA_SOCKET wins', () {
      expect(socketPath(env: {'VOLNA_SOCKET': '/x/y.sock', 'XDG_RUNTIME_DIR': '/run/user/1'}, windows: false),
          '/x/y.sock');
    });
    test('XDG_RUNTIME_DIR', () {
      expect(socketPath(env: {'XDG_RUNTIME_DIR': '/run/user/1000'}, windows: false),
          '/run/user/1000/volna/daemon.sock');
    });
    test('relative XDG_RUNTIME_DIR is ignored, temp dir + user', () {
      expect(socketPath(env: {'XDG_RUNTIME_DIR': 'rel', 'USER': 'k'}, windows: false, tempDir: '/tmp'),
          '/tmp/volna-k/volna/daemon.sock');
    });
    test('windows: temp dir, USERNAME, port file', () {
      final p = socketPath(env: {'USERNAME': 'k'}, windows: true, tempDir: r'C:\Temp\');
      expect(p, r'C:\Temp\volna-k\volna\daemon.sock');
      expect(portFilePath(p), r'C:\Temp\volna-k\volna\daemon.port');
      final ep = parsePortFile('51234 abcdef\n');
      expect((ep.port, ep.token), (51234, 'abcdef'));
      expect(() => parsePortFile('nope'), throwsFormatException);
    });
  });

  group('SocketTransport', () {
    late Directory dir;
    late String path;
    setUp(() async {
      dir = await Directory.systemTemp.createTemp('volna-ipc-test');
      path = '${dir.path}/daemon.sock';
    });
    tearDown(() => dir.delete(recursive: true));

    SocketTransport transport() => SocketTransport(
          resolve: () async => UnixEndpoint(path),
          minBackoff: const Duration(milliseconds: 20),
          maxBackoff: const Duration(milliseconds: 50),
          connectWait: const Duration(seconds: 2),
        );

    test('hello, call, error, proto, events, show', () async {
      final daemon = await FakeDaemon.start(path, (f, send) {
        switch (f.msg) {
          case Call(:final id, cmd: 'catalog_album', :final args):
            send(Frame(Reply(id, ok: {
              'browse_id': (args as Map)['id'],
              'playlist_id': null,
              'title': 'A',
              'kind': 'ep',
              'artists': [
                {'id': 'UC1', 'name': 'X'}
              ],
              'year': 2020,
              'description': null,
              'thumbnails': [
                {'url': 'https://lh3/a', 'width': 60, 'height': 60}
              ],
              'explicit': false,
              'track_count': 1,
              'duration_s': 100,
              'tracks': [
                {
                  'video_id': 'dQw4w9WgXcQ',
                  'title': 't',
                  'artists': [],
                  'album': {'id': null, 'name': 'A'},
                  'duration_s': 100,
                  'thumbnails': [],
                  'explicit': true,
                  'kind': 'official_video',
                  'year': null,
                  'views': null,
                  'playable': true
                }
              ],
              'related': [],
            })));
          case Call(:final id):
            send(Frame(Reply(id, err: 'bad_args: nope')));
          case Proto(:final id, :final uri):
            send(Frame(ProtoReply(id, 200, [('Content-Type', 'image/png')]),
                Uint8List.fromList(uri.codeUnits)));
            send(Frame(const Event('player:wave', {'active': true})));
            send(Frame(const Show()));
          default:
        }
      });
      final t = transport()..start();
      final api = CatalogApi(t);
      final album = await api.album('MPREb_x');
      expect(daemon.hellos.single.role, Role.window);
      expect(t.streamBase, 'http://127.0.0.1:9/');
      expect(t.status, TransportStatus.connected);
      expect((album.browseId, album.kind, album.year, album.durationS), ('MPREb_x', AlbumKind.ep, 2020, 100));
      expect(album.tracks.single.kind, TrackKind.officialVideo);
      expect(album.tracks.single.album!.name, 'A');

      await expectLater(
          t.call('nope'),
          throwsA(isA<DaemonException>()
              .having((e) => e.code, 'code', 'bad_args')
              .having((e) => e.message, 'message', 'nope')));

      final ev = t.events.first;
      final show = t.show.first;
      final r = await t.proto('GET', 'volna://img/?u=a');
      expect(r.status, 200);
      expect(r.header('content-type'), 'image/png');
      expect(String.fromCharCodes(r.body), 'volna://img/?u=a');
      expect((await ev).name, 'player:wave');
      await show;

      await t.close();
      await daemon.close();
    });

    test('pending calls fail when the daemon goes away, then it reconnects', () async {
      final first = Completer<void>();
      var daemon = await FakeDaemon.start(path, (f, send) {
        if (!first.isCompleted) first.complete(); // never answers
      });
      final t = transport()..start();
      final call = t.call('player_snapshot');
      final failed = expectLater(
          call, throwsA(isA<DaemonException>().having((e) => e.code, 'code', DaemonException.unavailable)));
      await first.future;
      await daemon.close();
      await failed;

      await File(path).delete().catchError((_) => File(path));
      daemon = await FakeDaemon.start(path, (f, send) {
        if (f.msg case Call(:final id)) send(Frame(Reply(id, ok: 42)));
      });
      expect(await t.call('anything'), 42);
      await t.close();
      await daemon.close();
    });

    test('quit stops the transport', () async {
      final daemon = await FakeDaemon.start(path, (f, send) => send(Frame(const Quit())));
      final t = transport()..start();
      final quit = t.quit.first;
      unawaited(t.call('x').then((_) {}, onError: (_) {}));
      await quit;
      await t.statusChanges.firstWhere((s) => s == TransportStatus.closed);
      await expectLater(t.call('y'), throwsA(isA<DaemonException>()));
      await daemon.close();
    });
  });
}
