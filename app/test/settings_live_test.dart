/// The settings wrappers against a running daemon (its socket from
/// `VOLNA_SOCKET` or the default), only with `VOLNA_LIVE=1`. Read-only:
/// nothing is changed in the daemon's settings.
library;

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:volna/api/network.dart';
import 'package:volna/api/scrobble.dart';
import 'package:volna/api/settings.dart';
import 'package:volna/api/yandex.dart';
import 'package:volna/ipc/socket_transport.dart';

void main() {
  final path = socketPath();
  // Opt-in, so a plain `flutter test` never talks to the user's own daemon.
  final live = Platform.environment['VOLNA_LIVE'] == '1';
  final running = live &&
      !Platform.isWindows &&
      FileSystemEntity.typeSync(path) == FileSystemEntityType.unixDomainSock;

  test('settings commands', () async {
    final t = SocketTransport()..start();
    addTearDown(t.close);

    final net = NetworkApi(t);
    final view = await net.settings();
    // ignore: avoid_print
    print('net: ${view.config.mode.name} ${view.status.mode} cookies=${view.cookies.mode.name}');
    final health = await net.health();
    // ignore: avoid_print
    print('health: ok=${health.ok} ip=${health.exitIp} ytm=${health.ytmAvailable} ${health.latencyMs} ms');
    expect(health.latencyMs != null || health.error != null, isTrue);
    final yt = await net.ytdlpVersion();
    expect(yt.version, isNotEmpty);

    final sc = await ScrobbleApi(t).status();
    expect(sc.services.map((s) => s.service), ScrobbleService.values);
    // ignore: avoid_print
    print('scrobble: ${sc.percentage}% ${[for (final s in sc.services) '${s.service.name}:${s.active}']}');

    final ya = await YandexApi(t).status();
    // ignore: avoid_print
    print('yandex: signedIn=${ya.signedIn} importing=${ya.importing}');

    final legacy = await LegacyApi(t).status();
    // ignore: avoid_print
    print('legacy: imported=${legacy.imported}');

    final wave = await WaveSettingsApi(t).status();
    // ignore: avoid_print
    print('wave: cold=${wave.cold} character=${wave.settings.character.wire}');
    final acts = await WaveSettingsApi(t).activities();
    expect(acts.activities, isNotEmpty);
    final artists = await WaveSettingsApi(t).artistSearch('Kino');
    expect(artists, isNotEmpty);
  }, skip: running ? false : 'set VOLNA_LIVE=1 with a daemon socket at $path', timeout: const Timeout(Duration(minutes: 2)));
}
