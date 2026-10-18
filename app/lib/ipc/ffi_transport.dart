/// [Transport] over `dart:ffi` into `volna-core` (Android): the C ABI in
/// `crates/core/include/volna_core.h`. Same commands, events and `volna://`
/// requests as the daemon socket, so `lib/api` works unchanged.
///
/// Ownership (header): every string or body the core hands to a callback
/// belongs to us and is freed here with `volna_core_free_string` /
/// `volna_core_free_bytes`; strings we pass in are only borrowed for the
/// duration of the call.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'transport.dart';

/// `struct VolnaCore`.
final class VolnaCoreHandle extends Opaque {}

typedef _ReplyN = Void Function(Pointer<Void>, Uint64, Bool, Pointer<Utf8>);
typedef _EventN = Void Function(Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>);
typedef _ProtoN = Void Function(
    Pointer<Void>, Uint64, Uint16, Pointer<Utf8>, Pointer<Uint8>, Size);

typedef _Core = Pointer<VolnaCoreHandle>;

/// The functions of `libvolna_core`.
class VolnaCoreLib {
  final DynamicLibrary lib;

  VolnaCoreLib(this.lib);

  /// `libvolna_core.so` from the app's native libraries (Android), or [path].
  factory VolnaCoreLib.open([String path = 'libvolna_core.so']) =>
      VolnaCoreLib(DynamicLibrary.open(path));

  late final init = lib.lookupFunction<_Core Function(Pointer<Utf8>),
      _Core Function(Pointer<Utf8>)>('volna_core_init');
  late final lastError = lib.lookupFunction<Pointer<Utf8> Function(),
      Pointer<Utf8> Function()>('volna_core_last_error');
  late final streamBase = lib.lookupFunction<Pointer<Utf8> Function(_Core),
      Pointer<Utf8> Function(_Core)>('volna_core_stream_base');
  late final setReplyCallback = lib.lookupFunction<
      Void Function(_Core, Pointer<NativeFunction<_ReplyN>>, Pointer<Void>),
      void Function(_Core, Pointer<NativeFunction<_ReplyN>>,
          Pointer<Void>)>('volna_core_set_reply_callback');
  late final setEventCallback = lib.lookupFunction<
      Void Function(_Core, Pointer<NativeFunction<_EventN>>, Pointer<Void>),
      void Function(_Core, Pointer<NativeFunction<_EventN>>,
          Pointer<Void>)>('volna_core_set_event_callback');
  late final setProtoCallback = lib.lookupFunction<
      Void Function(_Core, Pointer<NativeFunction<_ProtoN>>, Pointer<Void>),
      void Function(_Core, Pointer<NativeFunction<_ProtoN>>,
          Pointer<Void>)>('volna_core_set_proto_callback');
  late final call = lib.lookupFunction<
      Void Function(_Core, Uint64, Pointer<Utf8>, Pointer<Utf8>),
      void Function(_Core, int, Pointer<Utf8>, Pointer<Utf8>)>('volna_core_call');
  late final proto = lib.lookupFunction<Void Function(_Core, Uint64, Pointer<Utf8>),
      void Function(_Core, int, Pointer<Utf8>)>('volna_core_proto');
  late final freeString = lib.lookupFunction<Void Function(Pointer<Utf8>),
      void Function(Pointer<Utf8>)>('volna_core_free_string');
  late final freeBytes = lib.lookupFunction<Void Function(Pointer<Uint8>, Size),
      void Function(Pointer<Uint8>, int)>('volna_core_free_bytes');
  late final shutdown = lib.lookupFunction<Void Function(_Core),
      void Function(_Core)>('volna_core_shutdown');

  /// Reads and frees a string the core handed over (NULL → null).
  String? take(Pointer<Utf8> s) {
    if (s == nullptr) return null;
    try {
      return s.toDartString();
    } finally {
      freeString(s);
    }
  }
}

class FfiTransport implements Transport {
  final VolnaCoreLib _lib;
  final _Core _core;

  /// True when this transport started the core and shuts it down on
  /// [close]; false when it is attached to a core owned by the platform
  /// (Android: the media service) and only detaches.
  final bool _owned;

  late final NativeCallable<_ReplyN> _onReply;
  late final NativeCallable<_EventN> _onEvent;
  late final NativeCallable<_ProtoN> _onProto;

  final _status = StreamController<TransportStatus>.broadcast();
  final _events = StreamController<DaemonEvent>.broadcast();
  final _vis = StreamController<Uint8List>.broadcast();
  final _show = StreamController<void>.broadcast();
  final _quit = StreamController<void>.broadcast();

  final _calls = <int, Completer<Object?>>{};
  final _protos = <int, Completer<ProtoResponse>>{};
  int _next = 1;
  bool _closed = false;
  late final String? _streamBase;

  FfiTransport._(this._lib, this._core, this._owned) {
    _streamBase = _lib.take(_lib.streamBase(_core));
    _onReply = NativeCallable<_ReplyN>.listener(_reply);
    _onEvent = NativeCallable<_EventN>.listener(_event);
    _onProto = NativeCallable<_ProtoN>.listener(_proto);
    _lib.setReplyCallback(_core, _onReply.nativeFunction, nullptr);
    _lib.setEventCallback(_core, _onEvent.nativeFunction, nullptr);
    _lib.setProtoCallback(_core, _onProto.nativeFunction, nullptr);
  }

  /// Attaches to a core someone else started (`address` is the
  /// `VolnaCore *` as an integer, e.g. from the Android plugin).
  factory FfiTransport.attach(VolnaCoreLib lib, int address) {
    if (address == 0) throw ArgumentError.value(address, 'address', 'NULL core');
    return FfiTransport._(lib, Pointer<VolnaCoreHandle>.fromAddress(address), false);
  }

  /// Starts a core with `volna_core_init(config)` (`dataDir`, `cacheDir`,
  /// `audioServer`, `log`, debug-only `devProxy` / `devStreamUrl`) and owns it.
  factory FfiTransport.start(VolnaCoreLib lib, Map<String, Object?> config) {
    final c = jsonEncode(config).toNativeUtf8();
    final _Core core;
    try {
      core = lib.init(c);
    } finally {
      malloc.free(c);
    }
    if (core == nullptr) {
      throw StateError('volna_core_init: ${lib.take(lib.lastError()) ?? 'failed'}');
    }
    return FfiTransport._(lib, core, true);
  }

  void _reply(Pointer<Void> _, int id, bool ok, Pointer<Utf8> json) {
    final text = _lib.take(json) ?? 'null';
    final c = _calls.remove(id);
    if (c == null) return;
    final Object? value;
    try {
      value = jsonDecode(text);
    } catch (e) {
      c.completeError(DaemonException('internal', 'bad reply JSON: $e'));
      return;
    }
    if (ok) {
      c.complete(value);
    } else {
      c.completeError(DaemonException.parse(value is String ? value : '$value'));
    }
  }

  void _event(Pointer<Void> _, Pointer<Utf8> name, Pointer<Utf8> payload) {
    final n = _lib.take(name) ?? '';
    final p = _lib.take(payload);
    if (_events.isClosed) return;
    Object? value;
    try {
      value = p == null ? null : jsonDecode(p);
    } catch (_) {
      value = null;
    }
    _events.add(DaemonEvent(n, value));
  }

  void _proto(Pointer<Void> _, int id, int status, Pointer<Utf8> headersJson,
      Pointer<Uint8> body, int len) {
    final headersText = _lib.take(headersJson) ?? '[]';
    final Uint8List bytes;
    if (body == nullptr) {
      bytes = Uint8List(0);
    } else {
      bytes = len == 0 ? Uint8List(0) : Uint8List.fromList(body.asTypedList(len));
      _lib.freeBytes(body, len);
    }
    final c = _protos.remove(id);
    if (c == null) return;
    final headers = <Header>[];
    try {
      for (final pair in jsonDecode(headersText) as List) {
        final p = pair as List;
        headers.add((p[0] as String, p[1] as String));
      }
    } catch (_) {
      // Headers are best-effort; the body still counts.
    }
    c.complete(ProtoResponse(status, headers, bytes));
  }

  @override
  TransportStatus get status =>
      _closed ? TransportStatus.closed : TransportStatus.connected;
  @override
  Stream<TransportStatus> get statusChanges => _status.stream;
  @override
  String? get streamBase => _streamBase;
  @override
  Stream<DaemonEvent> get events => _events.stream;

  /// No visualizer on the platform player.
  @override
  Stream<Uint8List> get vis => _vis.stream;

  /// No daemon to ask for the window: never fires.
  @override
  Stream<void> get show => _show.stream;
  @override
  Stream<void> get quit => _quit.stream;

  static const _gone = DaemonException(DaemonException.unavailable, 'the transport is closed');

  @override
  Future<Object?> call(String cmd, [Map<String, Object?> args = const {}]) {
    if (_closed) return Future.error(_gone);
    final id = _next++;
    final c = Completer<Object?>();
    _calls[id] = c;
    final cmdPtr = cmd.toNativeUtf8();
    final argsPtr = jsonEncode(args).toNativeUtf8();
    try {
      _lib.call(_core, id, cmdPtr, argsPtr);
    } finally {
      malloc.free(cmdPtr);
      malloc.free(argsPtr);
    }
    return c.future;
  }

  /// Only `GET` exists over FFI (`volna_core_proto`); [headers] and [body]
  /// are not sent.
  @override
  Future<ProtoResponse> proto(
    String method,
    String uri, {
    List<Header> headers = const [],
    Uint8List? body,
  }) {
    if (_closed) return Future.error(_gone);
    if (method.toUpperCase() != 'GET') {
      return Future.value(ProtoResponse(405, const [], Uint8List(0)));
    }
    final id = _next++;
    final c = Completer<ProtoResponse>();
    _protos[id] = c;
    final uriPtr = uri.toNativeUtf8();
    try {
      _lib.proto(_core, id, uriPtr);
    } finally {
      malloc.free(uriPtr);
    }
    return c.future;
  }

  @override
  Future<void> close() async {
    if (_closed) return;
    _closed = true;
    // After these return the core never calls the old callbacks again
    // (`ffi.rs`), so closing the NativeCallables is safe.
    _lib.setReplyCallback(_core, nullptr, nullptr);
    _lib.setEventCallback(_core, nullptr, nullptr);
    _lib.setProtoCallback(_core, nullptr, nullptr);
    if (_owned) _lib.shutdown(_core);
    _onReply.close();
    _onEvent.close();
    _onProto.close();
    for (final c in _calls.values) {
      c.completeError(_gone);
    }
    for (final c in _protos.values) {
      c.completeError(_gone);
    }
    _calls.clear();
    _protos.clear();
    _status.add(TransportStatus.closed);
    await Future.wait([_events.close(), _vis.close(), _show.close(), _quit.close()]);
    await _status.close();
  }
}
