/// Picks the [Transport] for this platform: the daemon socket on desktop,
/// FFI into the process's `volna-core` on Android.
library;

import 'dart:io';

import 'package:flutter/services.dart';

import 'ffi_transport.dart';
import 'socket_transport.dart';
import 'transport.dart';

/// The Android side (`MainActivity.kt`, channel `volna/core`).
const androidCore = MethodChannel('volna/core');

/// Desktop: [SocketTransport] (starts the daemon if needed). Android: the
/// core is started by the app process (it outlives this engine: the media
/// service plays without the UI); Dart attaches to its handle.
Future<Transport> openPlatformTransport() async {
  if (Platform.isAndroid) {
    final address = await androidCore.invokeMethod<int>('handle');
    return FfiTransport.attach(VolnaCoreLib.open(), address ?? 0);
  }
  return SocketTransport(autostart: true)..start();
}

/// Android: Google sign-in in a WebView for the YouTube session playback
/// needs. Null when cancelled; `applied` false means it takes effect on
/// the next start of the app.
Future<({bool applied})?> androidGoogleSignIn() async {
  final r = await androidCore.invokeMapMethod<String, Object?>('googleSignIn');
  if (r == null) return null;
  return (applied: r['applied'] == true);
}

Future<bool> androidGoogleSignedIn() async =>
    await androidCore.invokeMethod<bool>('googleSignedIn') ?? false;

Future<void> androidGoogleSignOut() => androidCore.invokeMethod<void>('googleSignOut');
