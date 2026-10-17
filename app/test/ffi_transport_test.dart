/// [FfiTransport] against a host build of `libvolna_core.so`
/// (`cargo build -p volna-core`). Found as `$VOLNA_CORE_LIB`, else under
/// `$CARGO_TARGET_DIR` or `../target`; skipped when there is none.
library;

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:volna/api/player.dart';
import 'package:volna/ipc/ffi_transport.dart';
import 'package:volna/ipc/transport.dart';

String? _findLib() {
  final env = Platform.environment;
  final candidates = [
    env['VOLNA_CORE_LIB'],
    if (env['CARGO_TARGET_DIR'] case final t?) '$t/debug/libvolna_core.so',
    '../target/debug/libvolna_core.so',
  ];
  for (final c in candidates) {
    if (c != null && c.isNotEmpty && File(c).existsSync()) return c;
  }
  return null;
}

void main() {
  final lib = Platform.isLinux ? _findLib() : null;

  test('calls, errors, events and proto over the C ABI', () async {
    final dir = await Directory.systemTemp.createTemp('volna-ffi-');
    addTearDown(() => dir.delete(recursive: true));
    final t = FfiTransport.start(
        VolnaCoreLib.open(lib!), {'dataDir': dir.path, 'audioServer': false});
    var closed = false;
    addTearDown(() async {
      if (!closed) await t.close();
    });
    expect(t.status, TransportStatus.connected);

    // The typed API on top works unchanged.
    final snap = await PlayerApi(t).snapshot();
    expect(snap.protocol, 1);

    await expectLater(
      t.call('no_such_command'),
      throwsA(isA<DaemonException>().having((e) => e.message, 'message', contains('unknown command'))),
    );
    await expectLater(
      t.call('net_apply', {
        'config': {'mode': 'url', 'value': 'socks5h://127.0.0.1:9'}
      }),
      throwsA(isA<DaemonException>().having((e) => e.code, 'code', 'unsupported')),
    );

    // An event: no stream backend in this build, so playing fails loudly.
    final error = t.events.firstWhere((e) => e.name == 'player:error');
    await t.call('player_set_queue', {
      'tracks': [
        {'id': 'dQw4w9WgXcQ', 'title': 'A'}
      ],
      'startIndex': 0,
      'play': true,
    });
    expect((await error.timeout(const Duration(seconds: 20))).payload, isA<Map>());

    // proto: status, headers and body (a bad image request is a JSON 400).
    final r = await t.proto('GET', 'volna://img/?s=80');
    expect(r.status, 400);
    expect(r.header('content-type'), contains('json'));
    expect((jsonDecode(utf8.decode(r.body)) as Map)['status'], 400);
    expect((await t.proto('POST', 'volna://img/')).status, 405);

    await t.close();
    closed = true;
    expect(t.status, TransportStatus.closed);
    await expectLater(t.call('player_snapshot'), throwsA(isA<DaemonException>()));
  }, skip: lib == null ? 'no host libvolna_core.so (cargo build -p volna-core)' : false);
}
