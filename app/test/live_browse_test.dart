/// End to end through the real screens against a running daemon:
/// search → album → play (muted) → like → the track is in the library.
///
/// Opt-in: `VOLNA_LIVE=1` and `VOLNA_LIVE_SOCKET` (= `VOLNA_SOCKET`) naming
/// the socket of an isolated test daemon, never the user's: it plays (muted)
/// and likes. `VOLNA_LIVE_SHOTS=<dir>` saves screenshots.
@Tags(['live'])
library;

import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:volna/ipc/proto_image.dart';
import 'package:volna/ipc/socket_transport.dart';
import 'package:volna/state/connection.dart';
import 'package:volna/ui/browse/routes.dart';
import 'package:volna/ui/theme/app_theme.dart';
import 'package:volna/ui/widgets/cards.dart';
import 'package:volna/ui/widgets/cover.dart';
import 'package:volna/ui/widgets/track_row.dart';

import 'support/test_app.dart' show loadAppFonts;

void main() {
  final live = Platform.environment['VOLNA_LIVE'] == '1';
  final socket = Platform.environment['VOLNA_LIVE_SOCKET'];
  final shots = Platform.environment['VOLNA_LIVE_SHOTS'];

  testWidgets('search → album → play → like → library', (tester) async {
    await loadAppFonts();
    tester.view.physicalSize = const Size(1280, 860);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    // The transport resolves VOLNA_SOCKET; it must be the same isolated socket.
    expect(socketPath(), socket);
    final t = SocketTransport()..start();
    final router = GoRouter(initialLocation: '/search?q=Daft Punk Discovery', routes: browseRoutes);
    final key = GlobalKey();

    Future<void> wait(bool Function() done, {int seconds = 60, String? what}) async {
      for (var i = 0; i < seconds * 4; i++) {
        await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 250)));
        await tester.pump(const Duration(milliseconds: 50));
        if (done()) return;
      }
      final texts = find.byType(Text).evaluate().map((e) => (e.widget as Text).data).whereType<String>().take(30);
      fail('timed out waiting for ${what ?? 'the screen'}; transport ${t.status}; on screen: ${texts.join(' | ')}');
    }

    Future<void> shot(String name) async {
      if (shots == null) return;
      // Let covers arrive and decode.
      for (var i = 0; i < 12; i++) {
        await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 250)));
        await tester.pump(const Duration(milliseconds: 50));
      }
      final boundary = key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
      final bytes = await tester.runAsync(() async {
        final img = await boundary.toImage();
        final data = await img.toByteData(format: ui.ImageByteFormat.png);
        return data!.buffer.asUint8List();
      });
      await tester.runAsync(() => File('$shots/$name.png').writeAsBytes(bytes!));
    }

    await tester.pumpWidget(RepaintBoundary(
      key: key,
      child: ProviderScope(
        overrides: [
          transportProvider.overrideWithValue(t),
          protoImageCacheProvider.overrideWithValue(ProtoImageCache(t, cacheDir: () async => null)),
        ],
        child: MaterialApp.router(
          debugShowCheckedModeBanner: false,
          theme: buildTheme(Brightness.dark),
          builder: (context, child) => ColoredBox(color: Theme.of(context).colorScheme.surface, child: child),
          routerConfig: router,
        ),
      ),
    ));

    // Search, then open the first album.
    await wait(
        () => [find.text('Лучший результат'), find.byType(TrackRow), find.byType(ItemCard)]
            .any((f) => f.evaluate().isNotEmpty),
        what: 'search results');
    await shot('live_search');
    await tester.tap(find.widgetWithText(ChoiceChip, 'Альбомы'));
    await wait(() => find.byType(ItemCard).evaluate().isNotEmpty, what: 'album results');
    await tester.tap(find.byType(ItemCard).first);
    await wait(() => find.text('Слушать').evaluate().isNotEmpty && find.byType(TrackRow).evaluate().length > 3,
        what: 'the album page');
    expect(router.state.uri.path, startsWith('/album/MPRE'));
    await shot('live_album');

    // Muted: the test daemon must not make noise.
    await tester.runAsync(() => t.call('player_set_settings', {'patch': {'muted': true}}));
    final row = tester.widget<TrackRow>(find.byType(TrackRow).at(1));
    await tester.tap(find.byType(TrackRow).at(1));
    await wait(() => true, seconds: 2);
    final snap = await tester.runAsync(() => t.call('player_snapshot'));
    final queue = ((snap as Map)['queue'] as Map)['tracks'] as List;
    expect(queue.map((e) => (e as Map)['id']), contains(row.track.videoId));

    // Like it from the row, then find it in the library.
    await tester.tap(find.descendant(of: find.byType(TrackRow).at(1), matching: find.byType(LikeButton)));
    await wait(() => find.byTooltip('Убрать из любимых').evaluate().isNotEmpty, what: 'the like');
    router.go('/library');
    await wait(() => find.text(row.track.title).evaluate().isNotEmpty, what: 'the liked track in the library');
    await shot('live_library');

    // Clean up: unlike, stop.
    await tester.runAsync(() async {
      await t.call('library_like', {'kind': 'track', 'item': row.track.videoId, 'on': false});
      await t.call('player_stop');
      await t.close();
    });
  }, skip: !live || socket == null); // VOLNA_LIVE=1 + VOLNA_LIVE_SOCKET (an isolated daemon)
}
