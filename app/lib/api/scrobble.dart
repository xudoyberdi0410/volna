/// Typed wrappers over `scrobble_*` (`crates/daemon/src/api/scrobble.rs`,
/// shapes in `crates/scrobble/src/service.rs`). Tokens and secrets are never
/// echoed back: the status only says whether one is set.
library;

import '../ipc/transport.dart';
import '_json.dart';

enum ScrobbleService {
  lastfm('Last.fm'),
  librefm('Libre.fm'),
  listenbrainz('ListenBrainz'),
  maloja('Maloja');

  final String title;
  const ScrobbleService(this.title);
}

class ScrobbleQueue {
  final int pending;

  /// Unix seconds of the next retry.
  final int? nextRetryAt;
  final String? lastError;
  const ScrobbleQueue({this.pending = 0, this.nextRetryAt, this.lastError});

  factory ScrobbleQueue.fromJson(Json? j) => ScrobbleQueue(
        pending: asInt(j?['pending']) ?? 0,
        nextRetryAt: asInt(j?['nextRetryAt']),
        lastError: j?['lastError'] as String?,
      );
}

class ServiceStatus {
  final ScrobbleService service;
  final bool enabled;

  /// Enabled and has credentials: plays are sent.
  final bool active;
  final bool canLove;
  final bool loveOnLike;

  /// Last.fm and Libre.fm log in through the browser.
  final bool usesLogin;
  final String? username;
  final bool loginPending;
  final bool hasToken;
  final String? url;
  final bool? useCustomCredentials;
  final String? customApiKey;
  final bool? hasCustomApiSecret;
  final ScrobbleQueue queue;

  const ServiceStatus({
    required this.service,
    this.enabled = false,
    this.active = false,
    this.canLove = false,
    this.loveOnLike = false,
    this.usesLogin = false,
    this.username,
    this.loginPending = false,
    this.hasToken = false,
    this.url,
    this.useCustomCredentials,
    this.customApiKey,
    this.hasCustomApiSecret,
    this.queue = const ScrobbleQueue(),
  });

  factory ServiceStatus.fromJson(Json j) => ServiceStatus(
        service: enumOf(ScrobbleService.values, j['service'], ScrobbleService.lastfm),
        enabled: j['enabled'] as bool? ?? false,
        active: j['active'] as bool? ?? false,
        canLove: j['canLove'] as bool? ?? false,
        loveOnLike: j['loveOnLike'] as bool? ?? false,
        usesLogin: j['usesLogin'] as bool? ?? false,
        username: j['username'] as String?,
        loginPending: j['loginPending'] as bool? ?? false,
        hasToken: j['hasToken'] as bool? ?? false,
        url: j['url'] as String?,
        useCustomCredentials: j['useCustomCredentials'] as bool?,
        customApiKey: j['customApiKey'] as String?,
        hasCustomApiSecret: j['hasCustomApiSecret'] as bool?,
        queue: ScrobbleQueue.fromJson(j['queue'] as Json?),
      );
}

class ScrobbleStatus {
  /// Share of the track to play before it is scrobbled (or 4 minutes).
  final int percentage;
  final List<ServiceStatus> services;
  const ScrobbleStatus(this.percentage, this.services);

  factory ScrobbleStatus.fromJson(Json j) =>
      ScrobbleStatus(asInt(j['percentage']) ?? 50, listOf(j['services'], ServiceStatus.fromJson));

  ServiceStatus of(ScrobbleService s) => services.firstWhere((x) => x.service == s,
      orElse: () => ServiceStatus(service: s));
}

class ScrobbleApi {
  final Transport _t;
  const ScrobbleApi(this._t);

  Future<ScrobbleStatus> _status(String cmd, [Map<String, Object?> args = const {}]) async =>
      ScrobbleStatus.fromJson((await _t.call(cmd, args)) as Json);

  Future<ScrobbleStatus> status() => _status('scrobble_status');

  /// Only the given fields change. [percentage] is shared by all services.
  Future<ScrobbleStatus> set({
    ScrobbleService? service,
    bool? enabled,
    bool? loveOnLike,
    String? token,
    String? url,
    int? percentage,
    bool? useCustomCredentials,
    String? customApiKey,
    String? customApiSecret,
  }) =>
      _status('scrobble_set', {
        'service': ?service?.name,
        'enabled': ?enabled,
        'loveOnLike': ?loveOnLike,
        'token': ?token,
        'url': ?url,
        'percentage': ?percentage,
        'useCustomCredentials': ?useCustomCredentials,
        'customApiKey': ?customApiKey,
        'customApiSecret': ?customApiSecret,
      });

  /// Last.fm / Libre.fm: the page to open in the browser.
  Future<String> loginStart(ScrobbleService s) async =>
      ((await _t.call('scrobble_login_start', {'service': s.name})) as Json)['url'] as String;

  /// After the user allowed access in the browser: the username.
  Future<String?> loginFinish(ScrobbleService s) async =>
      ((await _t.call('scrobble_login_finish', {'service': s.name})) as Json)['username'] as String?;

  Future<ScrobbleStatus> logout(ScrobbleService s) =>
      _status('scrobble_logout', {'service': s.name});

  /// Retries the offline queue now.
  Future<ScrobbleStatus> flush() => _status('scrobble_flush');
}
