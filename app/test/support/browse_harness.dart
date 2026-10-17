/// Mounts the browse routes on a fake daemon for widget and golden tests.
library;

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:volna/ipc/proto_image.dart';
import 'package:volna/state/connection.dart';
import 'package:volna/ui/browse/history_screen.dart';
import 'package:volna/ui/browse/routes.dart';
import 'package:volna/ui/widgets/cover.dart';

import 'fake_transport_browse.dart';

const _fontFamily = 'NotoSans';

/// Real fonts instead of the test font, when the system has them; false otherwise.
Future<bool> loadFonts() async {
  const noto = '/usr/share/fonts/noto';
  final icons = '${Platform.environment['FLUTTER_ROOT'] ?? '${Platform.environment['HOME']}/.local/share/flutter'}'
      '/bin/cache/artifacts/material_fonts/MaterialIcons-Regular.otf';
  final files = [
    for (final w in ['Regular', 'Medium', 'SemiBold', 'Bold', 'ExtraBold']) '$noto/NotoSans-$w.ttf',
  ].where((f) => File(f).existsSync()).toList();
  if (files.isEmpty || !File(icons).existsSync()) return false;
  final text = FontLoader(_fontFamily);
  for (final f in files) {
    text.addFont(Future.value(ByteData.sublistView(File(f).readAsBytesSync())));
  }
  await text.load();
  final ic = FontLoader('MaterialIcons')..addFont(Future.value(ByteData.sublistView(File(icons).readAsBytesSync())));
  await ic.load();
  return true;
}

ThemeData browseTheme(Brightness b) => ThemeData(
      colorSchemeSeed: const Color(0xFF6E5BFF),
      brightness: b,
      fontFamily: _fontFamily,
      visualDensity: VisualDensity.standard,
    );

class BrowseTestApp extends StatelessWidget {
  final FakeTransport transport;
  final String location;
  final Brightness brightness;
  final GoRouter? router;
  const BrowseTestApp({
    super.key,
    required this.transport,
    required this.location,
    this.brightness = Brightness.dark,
    this.router,
  });

  @override
  Widget build(BuildContext context) => ProviderScope(
        overrides: [
          transportProvider.overrideWithValue(transport),
          protoImageCacheProvider.overrideWithValue(ProtoImageCache(transport, cacheDir: () async => null)),
          clockProvider.overrideWithValue(() => DateTime(2026, 10, 17, 21, 30)),
        ],
        child: MaterialApp.router(
          debugShowCheckedModeBanner: false,
          theme: browseTheme(brightness),
          // The shell paints the background; screens are transparent.
          builder: (context, child) => ColoredBox(color: Theme.of(context).colorScheme.surface, child: child),
          routerConfig: router ?? GoRouter(initialLocation: location, routes: browseRoutes),
        ),
      );
}

/// Pumps until data and images are in (real async for image decoding).
Future<void> settle(WidgetTester tester, {int rounds = 6}) async {
  for (var i = 0; i < rounds; i++) {
    await tester.pump(const Duration(milliseconds: 300));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 40)));
  }
  await tester.pump(const Duration(milliseconds: 400));
}

/// Sets the test window size (logical px, DPR 1).
void setSize(WidgetTester tester, Size size) {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}
