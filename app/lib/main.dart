import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'ipc/platform_transport.dart';
import 'state/connection.dart';
import 'ui/app.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final transport = await openPlatformTransport();
  // The daemon owns playback; the window just goes away when told to.
  transport.quit.listen((_) => exit(0));
  // `show` from the daemon (a second launch, the tray): raise the window.
  // Implemented by the Linux runner; elsewhere the call is a no-op.
  const window = MethodChannel('volna/window');
  transport.show.listen((_) => window.invokeMethod<void>('show').catchError((_) {}));
  runApp(ProviderScope(
    overrides: [transportProvider.overrideWithValue(transport)],
    child: const VolnaApp(),
  ));
}
