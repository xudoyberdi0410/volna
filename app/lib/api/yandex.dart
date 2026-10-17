/// Typed wrappers over `yandex_*` (`crates/daemon/src/api/yandex/mod.rs`)
/// and its events `yandex_import_progress` / `yandex_import_done`.
library;

import '../ipc/transport.dart';
import '_json.dart';

/// The Yandex OAuth app the daemon uses (`client.rs::CLIENT_ID`); its
/// authorize page redirects to an URL with `#access_token=`.
const yandexClientId = '23cabbbdc6cd418abb4b39c32c41195d';

String yandexAuthorizeUrl([String host = 'oauth.yandex.ru']) =>
    'https://$host/authorize?response_type=token&client_id=$yandexClientId';

/// The token in a redirect URL (`…#access_token=…&…`), if any.
String? tokenFromRedirect(String url) {
  final i = url.indexOf('access_token=');
  if (i < 0) return null;
  final rest = url.substring(i + 'access_token='.length);
  final end = rest.indexOf(RegExp('[&#]'));
  final token = end < 0 ? rest : rest.substring(0, end);
  return token.isEmpty ? null : token;
}

class YandexStatus {
  final bool signedIn;
  final String? login;
  final bool importing;
  const YandexStatus({this.signedIn = false, this.login, this.importing = false});

  factory YandexStatus.fromJson(Json j) => YandexStatus(
        signedIn: j['signedIn'] as bool? ?? false,
        login: j['login'] as String?,
        importing: j['importing'] as bool? ?? false,
      );
}

class YandexImportOptions {
  final bool likes;
  final bool playlists;
  final bool albums;
  final bool artists;

  /// Try the browser's Yandex session when no token is held.
  final bool browser;
  const YandexImportOptions(
      {this.likes = true, this.playlists = true, this.albums = true, this.artists = true, this.browser = true});

  YandexImportOptions copyWith({bool? likes, bool? playlists, bool? albums, bool? artists, bool? browser}) =>
      YandexImportOptions(
        likes: likes ?? this.likes,
        playlists: playlists ?? this.playlists,
        albums: albums ?? this.albums,
        artists: artists ?? this.artists,
        browser: browser ?? this.browser,
      );

  bool get any => likes || playlists || albums || artists;

  Json toJson() =>
      {'likes': likes, 'playlists': playlists, 'albums': albums, 'artists': artists, 'browser': browser};
}

/// `yandex_import_progress`.
class YandexProgress {
  /// `browser`, `account`, `likes`, `playlists`, `albums`, then `tracks`,
  /// `album_match`, `artist_match`, `saving`.
  final String stage;
  final String? detail;
  final int done;
  final int total;
  final int matched;
  final String? current;
  const YandexProgress(
      {required this.stage, this.detail, this.done = 0, this.total = 0, this.matched = 0, this.current});

  factory YandexProgress.fromJson(Json j) => YandexProgress(
        stage: j['stage'] as String? ?? '',
        detail: j['detail'] as String?,
        done: asInt(j['done']) ?? 0,
        total: asInt(j['total']) ?? 0,
        matched: asInt(j['matched']) ?? 0,
        current: j['current'] as String?,
      );

  /// 0..1 while matching, null while reading Yandex.
  double? get fraction => total > 0 ? (done / total).clamp(0, 1).toDouble() : null;
}

class YandexSummary {
  final int likedTracks;
  final int playlists;
  final int albums;
  final int artists;
  final int matched;
  final int total;
  const YandexSummary(
      {this.likedTracks = 0, this.playlists = 0, this.albums = 0, this.artists = 0, this.matched = 0, this.total = 0});

  factory YandexSummary.fromJson(Json? j) => YandexSummary(
        likedTracks: asInt(j?['likedTracks']) ?? 0,
        playlists: asInt(j?['playlists']) ?? 0,
        albums: asInt(j?['albums']) ?? 0,
        artists: asInt(j?['artists']) ?? 0,
        matched: asInt(j?['matched']) ?? 0,
        total: asInt(j?['total']) ?? 0,
      );
}

class YandexMiss {
  final String title;
  final String artist;
  const YandexMiss(this.title, this.artist);
  factory YandexMiss.fromJson(Json j) => YandexMiss(j['title'] as String? ?? '', j['artist'] as String? ?? '');
}

/// `yandex_import_done`.
class YandexDone {
  final bool ok;
  final bool cancelled;

  /// No usable token: log in, `yandex_set_token`, start again.
  final bool needLogin;
  final String? error;
  final YandexSummary summary;
  final List<YandexMiss> misses;
  const YandexDone(
      {this.ok = false,
      this.cancelled = false,
      this.needLogin = false,
      this.error,
      this.summary = const YandexSummary(),
      this.misses = const []});

  factory YandexDone.fromJson(Json j) => YandexDone(
        ok: j['ok'] as bool? ?? false,
        cancelled: j['cancelled'] as bool? ?? false,
        needLogin: j['needLogin'] as bool? ?? false,
        error: j['error'] as String?,
        summary: YandexSummary.fromJson(j['summary'] as Json?),
        misses: listOf(j['misses'], YandexMiss.fromJson),
      );
}

sealed class YandexEvent {
  const YandexEvent();
}

final class YandexProgressEvent extends YandexEvent {
  final YandexProgress progress;
  const YandexProgressEvent(this.progress);
}

final class YandexDoneEvent extends YandexEvent {
  final YandexDone done;
  const YandexDoneEvent(this.done);
}

class YandexApi {
  final Transport _t;
  const YandexApi(this._t);

  Stream<YandexEvent> get events => _t.events.expand((e) => switch (e.name) {
        'yandex_import_progress' => [YandexProgressEvent(YandexProgress.fromJson(e.payload as Json))],
        'yandex_import_done' => [YandexDoneEvent(YandexDone.fromJson(e.payload as Json))],
        _ => const <YandexEvent>[],
      });

  Future<YandexStatus> status() async => YandexStatus.fromJson((await _t.call('yandex_status')) as Json);

  /// A bare token or the redirect URL with `#access_token=`; checked by the daemon.
  Future<YandexStatus> setToken(String input) async =>
      YandexStatus.fromJson((await _t.call('yandex_set_token', {'token': input})) as Json);

  /// Runs in the background; follow [events]. Throws `busy` when one is running.
  Future<void> start(YandexImportOptions options) => _t.call('yandex_import_start', options.toJson());

  /// True when an import was running.
  Future<bool> cancel() async => (await _t.call('yandex_import_cancel')) as bool? ?? false;
}
