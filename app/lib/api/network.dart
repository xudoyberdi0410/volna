/// Typed wrappers over the network commands of `crates/daemon/src/network.rs`
/// (`net_*`) and the yt-dlp ones of `commands.rs` (`ytdlp_*`). These replies
/// keep Rust's snake_case field names (`xray_running`, `latency_ms`, …).
library;

import '../ipc/transport.dart';
import '_json.dart';

/// `volna_net::ProxyConfig`: `{mode: direct}` or `{mode: url|vless, value}`.
enum NetMode { direct, url, vless }

class ProxyConfig {
  final NetMode mode;
  final String value;
  const ProxyConfig(this.mode, [this.value = '']);
  const ProxyConfig.direct() : this(NetMode.direct);

  factory ProxyConfig.fromJson(Json? j) => ProxyConfig(
        enumOf(NetMode.values, j?['mode'], NetMode.direct),
        j?['value'] as String? ?? '',
      );

  Json toJson() => mode == NetMode.direct
      ? {'mode': 'direct'}
      : {'mode': mode.name, 'value': value.trim()};

  @override
  bool operator ==(Object other) =>
      other is ProxyConfig && other.mode == mode && other.value == value;

  @override
  int get hashCode => Object.hash(mode, value);
}

/// `volna_streams::Cookies`: none, a browser (`name` or `name:profile`) or a
/// Netscape cookies.txt path.
enum CookiesMode { none, browser, file }

class CookiesConfig {
  final CookiesMode mode;
  final String value;
  const CookiesConfig(this.mode, [this.value = '']);
  const CookiesConfig.none() : this(CookiesMode.none);

  factory CookiesConfig.fromJson(Json? j) => CookiesConfig(
        enumOf(CookiesMode.values, j?['mode'], CookiesMode.none),
        j?['value'] as String? ?? '',
      );

  Json toJson() => mode == CookiesMode.none
      ? {'mode': 'none'}
      : {'mode': mode.name, 'value': value.trim()};
}

/// `volna_net::NetStatus`.
class NetStatus {
  /// `direct` / `url` / `vless`.
  final String mode;

  /// Proxy URL with the password masked.
  final String? proxy;
  final bool? xrayRunning;
  final int? xrayPid;
  const NetStatus({this.mode = 'direct', this.proxy, this.xrayRunning, this.xrayPid});

  factory NetStatus.fromJson(Json j) => NetStatus(
        mode: j['mode'] as String? ?? 'direct',
        proxy: j['proxy'] as String?,
        xrayRunning: j['xray_running'] as bool?,
        xrayPid: asInt(j['xray_pid']),
      );
}

/// `net_settings`.
class NetSettings {
  final ProxyConfig config;
  final NetStatus status;

  /// Why the saved setting did not apply at startup.
  final String? startupError;
  final CookiesConfig cookies;
  const NetSettings(this.config, this.status, this.startupError, this.cookies);

  factory NetSettings.fromJson(Json j) => NetSettings(
        ProxyConfig.fromJson(j['config'] as Json?),
        NetStatus.fromJson(j['status'] as Json? ?? const {}),
        j['startup_error'] as String?,
        CookiesConfig.fromJson(j['cookies'] as Json?),
      );
}

/// `volna_net::HealthReport`.
class HealthReport {
  final bool ok;
  final int? latencyMs;
  final String? exitIp;

  /// false: "not available in your area"; null: unclear.
  final bool? ytmAvailable;
  final String? error;
  const HealthReport({required this.ok, this.latencyMs, this.exitIp, this.ytmAvailable, this.error});

  factory HealthReport.fromJson(Json j) => HealthReport(
        ok: j['ok'] as bool? ?? false,
        latencyMs: asInt(j['latency_ms']),
        exitIp: j['exit_ip'] as String?,
        ytmAvailable: j['ytm_available'] as bool?,
        error: j['error'] as String?,
      );
}

/// `volna_streams::FailureKind`.
enum FailureKind { botCheck, unavailable, noYtDlp, other }

FailureKind? _failureKind(Object? s) => switch (s) {
      null => null,
      'bot_check' => FailureKind.botCheck,
      'unavailable' => FailureKind.unavailable,
      'no_yt_dlp' => FailureKind.noYtDlp,
      _ => FailureKind.other,
    };

/// `net_audio_check`: can yt-dlp resolve audio through this network?
class AudioCheck {
  final bool ok;
  final FailureKind? kind;
  final String? message;
  final int latencyMs;
  const AudioCheck({required this.ok, this.kind, this.message, this.latencyMs = 0});

  factory AudioCheck.fromJson(Json j) => AudioCheck(
        ok: j['ok'] as bool? ?? false,
        kind: _failureKind(j['kind']),
        message: j['message'] as String?,
        latencyMs: asInt(j['latency_ms']) ?? 0,
      );
}

/// `volna_streams::Origin`: where yt-dlp came from.
enum YtDlpOrigin { configured, sidecar, system }

class YtDlpInfo {
  final String binary;
  final YtDlpOrigin origin;
  final String version;
  const YtDlpInfo(this.binary, this.origin, this.version);

  factory YtDlpInfo.fromJson(Json j) => YtDlpInfo(
        j['binary'] as String? ?? '',
        enumOf(YtDlpOrigin.values, j['origin'], YtDlpOrigin.system),
        j['version'] as String? ?? '',
      );
}

/// `volna_streams::UpdateReport`.
class YtDlpUpdate {
  final YtDlpOrigin origin;
  final String versionBefore;
  final String versionAfter;
  final bool updated;

  /// yt-dlp's own output, or why no update was attempted.
  final String message;
  const YtDlpUpdate(this.origin, this.versionBefore, this.versionAfter, this.updated, this.message);

  factory YtDlpUpdate.fromJson(Json j) => YtDlpUpdate(
        enumOf(YtDlpOrigin.values, j['origin'], YtDlpOrigin.system),
        j['version_before'] as String? ?? '',
        j['version_after'] as String? ?? '',
        j['updated'] as bool? ?? false,
        j['message'] as String? ?? '',
      );
}

class NetworkApi {
  final Transport _t;
  const NetworkApi(this._t);

  Future<NetSettings> settings() async =>
      NetSettings.fromJson((await _t.call('net_settings')) as Json);

  /// Checks a config without applying it; throws [DaemonException] when invalid.
  Future<void> validate(ProxyConfig config) => _t.call('net_validate', {'config': config.toJson()});

  /// Validates, applies (starts xray for VLESS) and saves.
  Future<NetStatus> apply(ProxyConfig config) async =>
      NetStatus.fromJson((await _t.call('net_apply', {'config': config.toJson()})) as Json);

  Future<void> setCookies(CookiesConfig cookies) =>
      _t.call('net_set_cookies', {'cookies': cookies.toJson()});

  Future<HealthReport> health() async => HealthReport.fromJson((await _t.call('net_health')) as Json);

  Future<AudioCheck> audioCheck() async =>
      AudioCheck.fromJson((await _t.call('net_audio_check')) as Json);

  Future<List<String>> xrayLogs() async =>
      [for (final l in (await _t.call('net_xray_logs')) as List? ?? const []) l as String];

  Future<YtDlpInfo> ytdlpVersion() async =>
      YtDlpInfo.fromJson((await _t.call('ytdlp_version')) as Json);

  Future<YtDlpUpdate> ytdlpUpdate() async =>
      YtDlpUpdate.fromJson((await _t.call('ytdlp_update')) as Json);
}
