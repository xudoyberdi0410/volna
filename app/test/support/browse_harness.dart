/// Browse screens inside the real app (shell, theme, fonts) over a fake daemon.
library;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';

import 'fake_transport.dart';
import 'test_app.dart';

/// Pumps the app at [location] and waits for data and covers.
Future<GoRouter> pumpBrowse(
  WidgetTester tester,
  FakeTransport t,
  String location, {
  Size size = const Size(1440, 900),
  ThemeMode theme = ThemeMode.dark,
}) async {
  final router = await pumpApp(tester, t, location: location, size: size, theme: theme);
  await settle(tester);
  return router;
}

/// Pumps until data and images are in (real async for image decoding).
Future<void> settle(WidgetTester tester, {int rounds = 6}) async {
  for (var i = 0; i < rounds; i++) {
    await tester.pump(const Duration(milliseconds: 300));
    await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 40)));
  }
  await tester.pump(const Duration(milliseconds: 400));
}
