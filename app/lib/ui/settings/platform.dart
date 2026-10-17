/// Desktop services the settings use — opening the browser, picking a file,
/// the sign-in window — behind providers so widget tests can fake them.
library;

import 'dart:io';

import 'package:file_selector/file_selector.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:url_launcher/url_launcher.dart';

typedef UrlOpener = Future<bool> Function(Uri url);

/// Opens a page in the user's browser; false when that failed.
final urlOpenerProvider = Provider<UrlOpener>((ref) => (url) async {
      try {
        return await launchUrl(url, mode: LaunchMode.externalApplication);
      } catch (_) {
        return false;
      }
    });

typedef JsonFilePicker = Future<String?> Function();

/// Asks for a `.json` file; its path or null.
final jsonFilePickerProvider = Provider<JsonFilePicker>((ref) => () async {
      const group = XTypeGroup(label: 'JSON', extensions: ['json']);
      final f = await openFile(acceptedTypeGroups: const [group], confirmButtonText: 'Импортировать');
      return f?.path;
    });

/// A sign-in window with a web view (`linux/runner/web_login.cc`).
abstract interface class WebLogin {
  /// The runner can show one (WebKitGTK is installed).
  Future<bool> available();

  /// Opens [url]; completes with the first URL containing [match], or null
  /// when the user closed the window.
  Future<String?> open(String url, {required String title, String match = 'access_token='});
}

class ChannelWebLogin implements WebLogin {
  static const _channel = MethodChannel('volna/login');
  const ChannelWebLogin();

  @override
  Future<bool> available() async {
    if (!Platform.isLinux) return false;
    try {
      return await _channel.invokeMethod<bool>('available') ?? false;
    } on MissingPluginException {
      return false;
    } on PlatformException {
      return false;
    }
  }

  @override
  Future<String?> open(String url, {required String title, String match = 'access_token='}) =>
      _channel.invokeMethod<String>('open', {'url': url, 'title': title, 'match': match});
}

final webLoginProvider = Provider<WebLogin>((ref) => const ChannelWebLogin());

/// Feature detection, once per run.
final webLoginAvailableProvider = FutureProvider<bool>((ref) => ref.watch(webLoginProvider).available());
