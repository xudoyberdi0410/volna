/// [Transport] over the daemon socket (desktop), the Dart twin of
/// `ipc::Client` + `src-tauri/src/shell.rs`.
library;

import 'dart:async';
import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';

import 'frame.dart';
import 'transport.dart';

/// Where the daemon listens.
sealed class DaemonEndpoint {
  const DaemonEndpoint();
}

final class UnixEndpoint extends DaemonEndpoint {
  final String path;
  const UnixEndpoint(this.path);
  @override
  String toString() => 'unix:$path';
}

/// Windows: loopback TCP; [token] goes into `Hello`.
final class TcpEndpoint extends DaemonEndpoint {
  final int port;
  final String token;
  const TcpEndpoint(this.port, this.token);
  @override
  String toString() => 'tcp:127.0.0.1:$port';
}

/// `ipc::socket_path`: `VOLNA_SOCKET`, else `$XDG_RUNTIME_DIR/volna/daemon.sock`
/// (Linux only, absolute paths only, like `dirs::runtime_dir`), else
/// `<temp>/volna-<user>/volna/daemon.sock`.
String socketPath({
  Map<String, String>? env,
  bool? windows,
  String? tempDir,
}) {
  env ??= Platform.environment;
  windows ??= Platform.isWindows;
  final sep = windows ? r'\' : '/';
  final override = env['VOLNA_SOCKET'];
  if (override != null && override.isNotEmpty) return override;
  String? base;
  final xdg = env['XDG_RUNTIME_DIR'];
  if (!windows && xdg != null && xdg.startsWith('/')) base = xdg;
  if (base == null) {
    final user = env['USER'] ?? env['USERNAME'] ?? 'user';
    var tmp = tempDir ?? Directory.systemTemp.path;
    if (tmp.endsWith(sep)) tmp = tmp.substring(0, tmp.length - 1);
    base = '$tmp${sep}volna-$user';
  }
  return '$base${sep}volna${sep}daemon.sock';
}

/// `Path::with_extension("port")`.
String portFilePath(String socket) {
  final cut = math.max(socket.lastIndexOf('/'), socket.lastIndexOf(r'\'));
  final name = socket.substring(cut + 1);
  final dot = name.lastIndexOf('.');
  final stem = dot > 0 ? name.substring(0, dot) : name;
  return '${socket.substring(0, cut + 1)}$stem.port';
}

/// `ipc::read_port_file`: `"<port> <token>"`.
TcpEndpoint parsePortFile(String text) {
  final parts = text.trim().split(RegExp(r'\s+'));
  final port = int.tryParse(parts.isEmpty ? '' : parts[0]);
  if (port == null || port <= 0 || port > 65535) {
    throw const FormatException('bad port file');
  }
  return TcpEndpoint(port, parts.length > 1 ? parts[1] : '');
}

/// Resolves the endpoint the way `ipc::connect` does on this platform.
Future<DaemonEndpoint> resolveEndpoint() async {
  final path = socketPath();
  if (!Platform.isWindows) return UnixEndpoint(path);
  return parsePortFile(await File(portFilePath(path)).readAsString());
}

/// `volna` next to this executable (`VOLNA_DAEMON_BIN` overrides), as
/// `shell.rs::daemon_exe`.
String daemonExe() {
  final o = Platform.environment['VOLNA_DAEMON_BIN'];
  if (o != null && o.isNotEmpty) return o;
  final name = Platform.isWindows ? 'volna.exe' : 'volna';
  return File(Platform.resolvedExecutable).parent.uri.resolve(name).toFilePath();
}

class SocketTransport implements Transport {
  /// Start the daemon (`volna --tray`) when nobody listens, once.
  final bool autostart;
  final Duration minBackoff;
  final Duration maxBackoff;

  /// How long a call waits for a connection before failing.
  final Duration connectWait;
  final Future<DaemonEndpoint> Function() _resolve;

  SocketTransport({
    this.autostart = false,
    this.minBackoff = const Duration(milliseconds: 100),
    this.maxBackoff = const Duration(seconds: 5),
    this.connectWait = const Duration(seconds: 10),
    Future<DaemonEndpoint> Function()? resolve,
  }) : _resolve = resolve ?? resolveEndpoint;

  final _status = StreamController<TransportStatus>.broadcast();
  final _events = StreamController<DaemonEvent>.broadcast();
  final _vis = StreamController<Uint8List>.broadcast();
  final _show = StreamController<void>.broadcast();
  final _quit = StreamController<void>.broadcast();

  TransportStatus _state = TransportStatus.disconnected;
  String? _streamBase;
  Socket? _socket;
  Completer<Socket> _ready = Completer<Socket>();
  final _pending = <int, Completer<Frame>>{};
  int _next = 1;
  bool _closed = false;
  bool _running = false;
  bool _started = false;

  @override
  TransportStatus get status => _state;
  @override
  Stream<TransportStatus> get statusChanges => _status.stream;
  @override
  String? get streamBase => _streamBase;
  @override
  Stream<DaemonEvent> get events => _events.stream;
  @override
  Stream<Uint8List> get vis => _vis.stream;
  @override
  Stream<void> get show => _show.stream;
  @override
  Stream<void> get quit => _quit.stream;

  void _set(TransportStatus s) {
    if (_state == s) return;
    _state = s;
    if (!_status.isClosed) _status.add(s);
  }

  /// Starts connecting; reconnects with backoff until [close] or a `quit`.
  void start() {
    if (_running || _closed) return;
    _running = true;
    unawaited(_loop());
  }

  Future<void> _loop() async {
    var delay = minBackoff;
    while (!_closed) {
      _set(TransportStatus.connecting);
      Socket? sock;
      DaemonEndpoint? ep;
      try {
        ep = await _resolve();
        sock = await switch (ep) {
          UnixEndpoint(:final path) =>
            Socket.connect(InternetAddress(path, type: InternetAddressType.unix), 0),
          TcpEndpoint(:final port) => Socket.connect(InternetAddress.loopbackIPv4, port),
        };
      } catch (_) {
        if (autostart && !_started) {
          _started = true;
          await _startDaemon();
          delay = minBackoff;
        }
      }
      if (sock != null && ep != null) {
        try {
          await _session(sock, ep is TcpEndpoint ? ep.token : null);
          delay = minBackoff;
        } catch (_) {
          // Handshake failed or the stream broke; retry below.
        }
      }
      if (_closed) break;
      _set(TransportStatus.disconnected);
      await Future<void>.delayed(delay);
      delay = Duration(
          microseconds: math.min(delay.inMicroseconds * 2, maxBackoff.inMicroseconds));
    }
    _set(TransportStatus.closed);
  }

  Future<void> _startDaemon() async {
    final exe = daemonExe();
    if (!File(exe).existsSync()) return;
    try {
      await Process.start(exe, ['--tray'], mode: ProcessStartMode.detached);
    } catch (_) {}
  }

  /// One connection: hello → welcome, then frames until it drops.
  Future<void> _session(Socket sock, String? token) async {
    final decoder = FrameDecoder();
    final done = Completer<void>();
    var welcomed = false;
    sock.add(encodeFrame(Frame(Hello(Role.window, token: token))));
    late final StreamSubscription<Uint8List> sub;
    void finish([Object? e]) {
      if (done.isCompleted) return;
      sub.cancel();
      sock.destroy();
      if (_socket == sock) _socket = null;
      _failPending('the Volna daemon went away');
      if (_ready.isCompleted) _ready = Completer<Socket>();
      e == null ? done.complete() : done.completeError(e);
    }

    sub = sock.listen(
      (chunk) {
        final List<Frame> frames;
        try {
          frames = decoder.add(chunk);
        } catch (e) {
          finish(e);
          return;
        }
        for (final f in frames) {
          if (!welcomed) {
            final m = f.msg;
            if (m is! Welcome) {
              finish(FormatException('expected welcome, got ${m.runtimeType}'));
              return;
            }
            welcomed = true;
            _streamBase = m.streamBase;
            _socket = sock;
            _set(TransportStatus.connected);
            if (!_ready.isCompleted) _ready.complete(sock);
            continue;
          }
          _dispatch(f);
        }
      },
      onError: (Object e) => finish(e),
      onDone: () => finish(),
      cancelOnError: true,
    );
    return done.future;
  }

  void _dispatch(Frame f) {
    switch (f.msg) {
      case Reply(:final id) || ProtoReply(:final id):
        _pending.remove(id)?.complete(f);
      case Event(:final name, :final payload):
        _events.add(DaemonEvent(name, payload));
      case Vis():
        _vis.add(f.body);
      case Show():
        _show.add(null);
      case Quit():
        // A newer window took over, or the daemon is quitting: no reconnect.
        _closed = true;
        _quit.add(null);
        _socket?.destroy();
      default:
        break;
    }
  }

  void _failPending(String why) {
    final p = Map.of(_pending);
    _pending.clear();
    for (final c in p.values) {
      c.completeError(DaemonException(DaemonException.unavailable, why));
    }
  }

  Future<Socket> _connection() async {
    if (_closed) {
      throw const DaemonException(DaemonException.unavailable, 'the transport is closed');
    }
    final s = _socket;
    if (s != null) return s;
    start();
    try {
      return await _ready.future.timeout(connectWait);
    } on TimeoutException {
      throw const DaemonException(
          DaemonException.unavailable, 'the Volna daemon is not running');
    }
  }

  Future<Frame> _request(Frame Function(int id) make) async {
    final sock = await _connection();
    final id = _next++;
    final c = Completer<Frame>();
    _pending[id] = c;
    try {
      sock.add(encodeFrame(make(id)));
    } catch (e) {
      _pending.remove(id);
      throw DaemonException(DaemonException.unavailable, '$e');
    }
    return c.future;
  }

  @override
  Future<Object?> call(String cmd, [Map<String, Object?> args = const {}]) async {
    final f = await _request((id) => Frame(Call(id, cmd, args)));
    return switch (f.msg) {
      Reply(:final err?) => throw DaemonException.parse(err),
      Reply(:final ok) => ok,
      _ => throw DaemonException('internal', 'unexpected reply ${f.msg.runtimeType}'),
    };
  }

  @override
  Future<ProtoResponse> proto(
    String method,
    String uri, {
    List<Header> headers = const [],
    Uint8List? body,
  }) async {
    final f = await _request((id) => Frame(Proto(id, method, uri, headers), body));
    return switch (f.msg) {
      ProtoReply(:final status, :final headers) => ProtoResponse(status, headers, f.body),
      _ => throw DaemonException('internal', 'unexpected reply ${f.msg.runtimeType}'),
    };
  }

  @override
  Future<void> close() async {
    _closed = true;
    _socket?.destroy();
    _socket = null;
    _failPending('the transport is closed');
    if (!_ready.isCompleted) {
      _ready.future.ignore();
      _ready.completeError(
          const DaemonException(DaemonException.unavailable, 'the transport is closed'));
    }
    _set(TransportStatus.closed);
    await Future.wait([
      _events.close(),
      _vis.close(),
      _show.close(),
      _quit.close(),
    ]);
    await _status.close();
  }
}
