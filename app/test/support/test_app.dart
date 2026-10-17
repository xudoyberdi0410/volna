/// Pumping the real app (router, theme, shell) over a [FakeTransport], plus
/// real fonts and synthetic cover images for screenshot tests.
library;

import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:volna/ipc/transport.dart';
import 'package:volna/state/connection.dart';
import 'package:volna/state/player.dart';
import 'package:volna/state/theme.dart';
import 'package:volna/ui/app.dart';
import 'package:volna/ui/router.dart';
import 'package:volna/ui/shell/background.dart';

import 'fake_transport.dart';

/// Fixed wall clock for tests (Unix ms).
const kTestNow = 1760000000000;

bool _fontsLoaded = false;

/// Loads every font of the asset manifest (Inter, Material icons), so
/// screenshots show text instead of boxes.
Future<void> loadAppFonts() async {
  if (_fontsLoaded) return;
  _fontsLoaded = true;
  final manifest = json.decode(await rootBundle.loadString('FontManifest.json')) as List;
  for (final f in manifest) {
    final family = (f as Map)['family'] as String;
    final loader = FontLoader(family.replaceFirst(RegExp(r'^packages/[^/]+/'), ''));
    for (final a in f['fonts'] as List) {
      loader.addFont(rootBundle.load((a as Map)['asset'] as String));
    }
    await loader.load();
    // Also under the full name, for package fonts.
    if (family.startsWith('packages/')) {
      final l2 = FontLoader(family);
      for (final a in f['fonts'] as List) {
        l2.addFont(rootBundle.load((a as Map)['asset'] as String));
      }
      await l2.load();
    }
  }
}

/// A square PNG: a diagonal gradient between [a] and [b] with a soft disc.
Future<Uint8List> coverPng(Color a, Color b, {int size = 256}) async {
  final rec = ui.PictureRecorder();
  final c = Canvas(rec);
  final rect = Rect.fromLTWH(0, 0, size.toDouble(), size.toDouble());
  c.drawRect(
      rect,
      Paint()
        ..shader = LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [a, b],
        ).createShader(rect));
  c.drawCircle(
    Offset(size * 0.68, size * 0.36),
    size * 0.22,
    Paint()..color = Colors.white.withValues(alpha: 0.22),
  );
  c.drawCircle(
    Offset(size * 0.3, size * 0.72),
    size * 0.34,
    Paint()..color = Colors.black.withValues(alpha: 0.18),
  );
  final img = await rec.endRecording().toImage(size, size);
  final bytes = await img.toByteData(format: ui.ImageByteFormat.png);
  return bytes!.buffer.asUint8List();
}

String hex(Color c) =>
    '#${(c.toARGB32() & 0xffffff).toRadixString(16).padLeft(6, '0')}';

/// Registers cover [url] on [t] with colours [a]→[b] and a matching palette.
Future<void> addCover(FakeTransport t, String url, Color a, Color b) async {
  final png = await coverPng(a, b);
  final prev = t.protoFallback;
  t.protoFallback = (uri) {
    if (Uri.decodeComponent(uri).contains(url)) {
      return ProtoResponse(200, const [('content-type', 'image/png')], png);
    }
    return prev?.call(uri) ?? ProtoResponse(404, const [], Uint8List(0));
  };
  final prevPalette = t.handlers['color_palette'];
  t.on('color_palette', (args) {
    if ((args['imageUri'] as String).contains(url)) {
      return {
        'dominant': hex(a),
        'accent': hex(b),
        'onDominant': '#ffffff',
        'onAccent': '#000000',
        'dark': true,
        'swatches': [
          {'color': hex(a), 'share': 0.5},
          {'color': hex(Color.lerp(a, b, 0.5)!), 'share': 0.3},
          {'color': hex(b), 'share': 0.2},
        ],
      };
    }
    return prevPalette?.call(args);
  });
}

/// Pumps [VolnaApp] at [location] over [t] in a [size] window.
Future<GoRouter> pumpApp(
  WidgetTester tester,
  FakeTransport t, {
  String location = '/wave',
  Size size = const Size(1440, 900),
  ThemeMode theme = ThemeMode.dark,
}) async {
  SharedPreferences.setMockInitialValues({'volna.themeMode': theme.name});
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final router = buildRouter(initialLocation: location);
  addTearDown(router.dispose);
  await tester.pumpWidget(ProviderScope(
    overrides: [
      transportProvider.overrideWithValue(t),
      ambientMotionProvider.overrideWithValue(false),
      clockProvider.overrideWithValue(() => kTestNow),
      routerProvider.overrideWithValue(router),
      themeModeProvider.overrideWith(() => _FixedTheme(theme)),
    ],
    child: const VolnaApp(),
  ));
  await tester.pump();
  return router;
}

class _FixedTheme extends ThemeModeController {
  final ThemeMode mode;
  _FixedTheme(this.mode);
  @override
  ThemeMode build() => mode;
}

/// Lets real async work (image decoding) finish, then settles frames.
Future<void> settleImages(WidgetTester tester) async {
  for (var i = 0; i < 3; i++) {
    await tester.runAsync(() async {
      for (final e in find.byType(Image).evaluate()) {
        final w = e.widget as Image;
        await precacheImage(w.image, e).timeout(const Duration(seconds: 2), onTimeout: () {});
      }
      await Future<void>.delayed(const Duration(milliseconds: 20));
    });
    await tester.pump(const Duration(milliseconds: 100));
  }
  await tester.pump(const Duration(seconds: 2));
  await tester.pump(const Duration(seconds: 2));
}
