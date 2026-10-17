/// What `lib/api` talks to. Desktop implements it over the daemon socket
/// ([SocketTransport]); Android will implement it over FFI into `volna-core`.
/// Nothing outside `lib/ipc` may depend on a concrete transport.
library;

import 'dart:typed_data';

import 'frame.dart' show Header;

export 'frame.dart' show Header;

/// A daemon event (`Msg::Event`): `player:state`, `player:queue`, …
class DaemonEvent {
  final String name;
  final Object? payload;
  const DaemonEvent(this.name, this.payload);

  @override
  String toString() => 'DaemonEvent($name)';
}

/// The answer to a `volna://` request.
class ProtoResponse {
  final int status;
  final List<Header> headers;
  final Uint8List body;
  const ProtoResponse(this.status, this.headers, this.body);

  bool get ok => status >= 200 && status < 300;

  /// First header named [name] (case-insensitive).
  String? header(String name) {
    final n = name.toLowerCase();
    for (final (k, v) in headers) {
      if (k.toLowerCase() == n) return v;
    }
    return null;
  }
}

/// A failed command. Commands fail with `"<code>: <message>"`; [code] is
/// the part before the first `": "` (`bad_args`, `unavailable`, …), or
/// `error` when there is none.
class DaemonException implements Exception {
  final String code;
  final String message;
  const DaemonException(this.code, this.message);

  factory DaemonException.parse(String raw) {
    final i = raw.indexOf(': ');
    if (i > 0 && !raw.substring(0, i).contains(' ')) {
      return DaemonException(raw.substring(0, i), raw.substring(i + 2));
    }
    return DaemonException('error', raw);
  }

  /// The daemon is not running or the connection dropped mid-call.
  static const unavailable = 'unavailable';

  @override
  String toString() => '$code: $message';
}

enum TransportStatus { connecting, connected, disconnected, closed }

abstract interface class Transport {
  /// Current connection state; changes are on [statusChanges].
  TransportStatus get status;
  Stream<TransportStatus> get statusChanges;

  /// Base URL of the daemon's audio server (`Welcome.stream_base`), once known.
  String? get streamBase;

  /// Runs a daemon command (`args` use the camelCase keys). Completes with
  /// the decoded JSON reply, or throws [DaemonException].
  Future<Object?> call(String cmd, [Map<String, Object?> args = const {}]);

  /// A `volna://` request.
  Future<ProtoResponse> proto(
    String method,
    String uri, {
    List<Header> headers = const [],
    Uint8List? body,
  });

  /// Daemon events (broadcast).
  Stream<DaemonEvent> get events;

  /// Visualizer frames (broadcast).
  Stream<Uint8List> get vis;

  /// The daemon asks the window to come to front (broadcast).
  Stream<void> get show;

  /// The daemon asks the window to exit now (broadcast).
  Stream<void> get quit;

  Future<void> close();
}
