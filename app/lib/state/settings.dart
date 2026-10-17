/// Providers for the settings screens and the Yandex import that outlives them.
library;

import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/network.dart';
import '../api/scrobble.dart';
import '../api/settings.dart';
import '../api/yandex.dart';
import '../ipc/transport.dart';
import 'connection.dart';
import 'images.dart';

final networkApiProvider = Provider((ref) => NetworkApi(ref.watch(transportProvider)));
final scrobbleApiProvider = Provider((ref) => ScrobbleApi(ref.watch(transportProvider)));
final yandexApiProvider = Provider((ref) => YandexApi(ref.watch(transportProvider)));
final waveSettingsApiProvider = Provider((ref) => WaveSettingsApi(ref.watch(transportProvider)));
final legacyApiProvider = Provider((ref) => LegacyApi(ref.watch(transportProvider)));

/// Images of the cold-start artist picker: the app's one cache.
final settingsImageCacheProvider = protoImageCacheProvider;

/// Russian text of a daemon error, without the `code:` prefix.
String errorText(Object e) => switch (e) {
      DaemonException(code: DaemonException.unavailable) => 'Демон недоступен',
      DaemonException(:final message) => message,
      _ => e.toString(),
    };

/// The Yandex import as the settings page shows it. Kept in a provider so a
/// running import keeps its progress when the page is left and reopened.
class YandexImportState {
  final YandexStatus? status;
  final bool running;
  final YandexProgress? progress;
  final YandexDone? done;
  final String? error;
  const YandexImportState({this.status, this.running = false, this.progress, this.done, this.error});

  YandexImportState copyWith({
    YandexStatus? status,
    bool? running,
    YandexProgress? Function()? progress,
    YandexDone? Function()? done,
    String? Function()? error,
  }) =>
      YandexImportState(
        status: status ?? this.status,
        running: running ?? this.running,
        progress: progress == null ? this.progress : progress(),
        done: done == null ? this.done : done(),
        error: error == null ? this.error : error(),
      );
}

class YandexImportController extends Notifier<YandexImportState> {
  StreamSubscription<YandexEvent>? _sub;

  YandexApi get _api => ref.read(yandexApiProvider);

  @override
  YandexImportState build() {
    _sub = ref.watch(yandexApiProvider).events.listen(_onEvent);
    ref.onDispose(() => _sub?.cancel());
    return const YandexImportState();
  }

  void _onEvent(YandexEvent e) {
    switch (e) {
      case YandexProgressEvent(:final progress):
        state = state.copyWith(running: true, progress: () => progress, done: () => null);
      case YandexDoneEvent(:final done):
        state = state.copyWith(running: false, done: () => done, progress: () => null);
        unawaited(refresh());
    }
  }

  Future<void> refresh() async {
    try {
      final s = await _api.status();
      state = state.copyWith(
        status: s,
        running: s.importing || (state.running && state.done == null),
        error: () => null,
      );
    } catch (e) {
      state = state.copyWith(error: () => errorText(e));
    }
  }

  Future<void> start(YandexImportOptions options) async {
    state = state.copyWith(
      running: true,
      done: () => null,
      error: () => null,
      progress: () => const YandexProgress(stage: 'account'),
    );
    try {
      await _api.start(options);
    } catch (e) {
      state = state.copyWith(running: false, progress: () => null, error: () => errorText(e));
    }
  }

  Future<void> cancel() async {
    try {
      await _api.cancel();
    } catch (e) {
      state = state.copyWith(error: () => errorText(e));
    }
  }

  /// A pasted token or redirect URL (or one caught by the login window).
  Future<bool> setToken(String input) async {
    try {
      final s = await _api.setToken(input);
      state = state.copyWith(status: s, error: () => null);
      return true;
    } catch (e) {
      state = state.copyWith(error: () => errorText(e));
      return false;
    }
  }

  void dismissResult() => state = state.copyWith(done: () => null);
}

final yandexImportProvider =
    NotifierProvider<YandexImportController, YandexImportState>(YandexImportController.new);
