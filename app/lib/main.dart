import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'ipc/socket_transport.dart';
import 'ipc/transport.dart';
import 'state/connection.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  final transport = SocketTransport(autostart: true)..start();
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

class VolnaApp extends StatelessWidget {
  const VolnaApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Volna',
      debugShowCheckedModeBanner: false,
      themeMode: ThemeMode.system,
      theme: ThemeData(colorSchemeSeed: Colors.indigo, brightness: Brightness.light),
      darkTheme: ThemeData(colorSchemeSeed: Colors.indigo, brightness: Brightness.dark),
      home: const ConnectionScreen(),
    );
  }
}

class ConnectionScreen extends ConsumerWidget {
  const ConnectionScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final status = ref.watch(connectionStatusProvider).value ?? TransportStatus.connecting;
    final (label, icon) = switch (status) {
      TransportStatus.connected => ('Connected to the Volna daemon', Icons.check_circle_outline),
      TransportStatus.connecting => ('Connecting to the daemon…', Icons.sync),
      TransportStatus.disconnected => ('Daemon not available, retrying…', Icons.cloud_off),
      TransportStatus.closed => ('Disconnected', Icons.power_settings_new),
    };
    final base = ref.watch(transportProvider).streamBase;
    return Scaffold(
      body: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 48),
            const SizedBox(height: 16),
            Text(label, style: Theme.of(context).textTheme.titleMedium),
            if (status == TransportStatus.connected && base != null) ...[
              const SizedBox(height: 8),
              Text('Audio server: $base', style: Theme.of(context).textTheme.bodySmall),
            ],
          ],
        ),
      ),
    );
  }
}

