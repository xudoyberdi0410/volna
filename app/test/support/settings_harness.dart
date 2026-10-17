/// Pumps settings screens with a [FakeSettingsTransport] and fake desktop
/// services; loads real fonts so golden PNGs are readable.
library;

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/misc.dart' show Override;
import 'package:flutter_test/flutter_test.dart';
import 'package:volna/state/connection.dart';
import 'package:volna/ui/settings/platform.dart';

import 'fake_transport_settings.dart';

class FakeWebLogin implements WebLogin {
  bool isAvailable;
  String? result;
  final opened = <String>[];
  FakeWebLogin({this.isAvailable = true, this.result});

  @override
  Future<bool> available() async => isAvailable;

  @override
  Future<String?> open(String url, {required String title, String match = 'access_token='}) async {
    opened.add(url);
    return result;
  }
}

class Harness {
  final FakeSettingsTransport transport;
  final opened = <Uri>[];
  final FakeWebLogin webLogin;
  String? pickedFile;
  Harness(this.transport, {FakeWebLogin? webLogin}) : webLogin = webLogin ?? FakeWebLogin();

  List<Override> get overrides => [
        transportProvider.overrideWithValue(transport),
        urlOpenerProvider.overrideWithValue((u) async {
          opened.add(u);
          return true;
        }),
        jsonFilePickerProvider.overrideWithValue(() async => pickedFile),
        webLoginProvider.overrideWithValue(webLogin),
      ];

  Widget app(Widget child, {ThemeMode mode = ThemeMode.light}) => ProviderScope(
        overrides: overrides,
        child: MaterialApp(
          debugShowCheckedModeBanner: false,
          themeMode: mode,
          theme: ThemeData(colorSchemeSeed: Colors.indigo, brightness: Brightness.light, fontFamily: 'Roboto'),
          darkTheme: ThemeData(colorSchemeSeed: Colors.indigo, brightness: Brightness.dark, fontFamily: 'Roboto'),
          home: Scaffold(body: child),
        ),
      );
}

bool _fontsLoaded = false;

/// Roboto (with Cyrillic) and Material Icons from the Flutter SDK; the test
/// default font draws boxes.
Future<void> loadFonts() async {
  if (_fontsLoaded) return;
  _fontsLoaded = true;
  final root = Platform.environment['FLUTTER_ROOT'] ??
      File(Platform.resolvedExecutable).parent.parent.parent.parent.parent.parent.path;
  final dir = '$root/bin/cache/artifacts/material_fonts';
  Future<void> load(String family, List<String> files) async {
    final loader = FontLoader(family);
    for (final f in files) {
      final file = File('$dir/$f');
      if (file.existsSync()) {
        loader.addFont(Future.value(ByteData.sublistView(file.readAsBytesSync())));
      }
    }
    await loader.load();
  }

  const roboto = ['Roboto-Regular.ttf', 'Roboto-Medium.ttf', 'Roboto-Bold.ttf'];
  await load('Roboto', roboto);
  await load('monospace', ['Roboto-Regular.ttf']);
  await load('MaterialIcons', ['MaterialIcons-Regular.otf']);
}

/// A desktop-sized window.
void setWindow(WidgetTester tester, {double width = 1180, double height = 900}) {
  tester.view.physicalSize = Size(width, height);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}
