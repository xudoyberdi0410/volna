/// An in-memory [Transport] for widget tests of the browse screens:
/// commands are answered by handlers, calls are recorded, events are pushed
/// by the test, images come back as a generated PNG.
library;

import 'dart:async';
import 'dart:typed_data';

import 'package:volna/ipc/transport.dart';

typedef Handler = FutureOr<Object?> Function(Map<String, Object?> args);

class FakeCall {
  final String cmd;
  final Map<String, Object?> args;
  const FakeCall(this.cmd, this.args);
  @override
  String toString() => '$cmd $args';
}

class FakeTransport implements Transport {
  final Map<String, Handler> handlers;
  final calls = <FakeCall>[];
  final _events = StreamController<DaemonEvent>.broadcast();
  final _status = StreamController<TransportStatus>.broadcast();

  /// Bytes served for every `volna://img/` request; null answers 404.
  Uint8List? image;

  FakeTransport([Map<String, Handler>? handlers, this.image]) : handlers = {...?handlers};

  void on(String cmd, Handler h) => handlers[cmd] = h;

  void emit(String name, [Object? payload]) => _events.add(DaemonEvent(name, payload));

  List<FakeCall> callsOf(String cmd) => [for (final c in calls) if (c.cmd == cmd) c];

  @override
  TransportStatus get status => TransportStatus.connected;

  @override
  Stream<TransportStatus> get statusChanges => _status.stream;

  @override
  String? get streamBase => 'http://127.0.0.1:1';

  @override
  Future<Object?> call(String cmd, [Map<String, Object?> args = const {}]) async {
    calls.add(FakeCall(cmd, args));
    final h = handlers[cmd];
    if (h == null) throw DaemonException('bad_args', 'unknown command $cmd');
    return await h(args);
  }

  @override
  Future<ProtoResponse> proto(String method, String uri,
      {List<Header> headers = const [], Uint8List? body}) async {
    final img = image;
    if (img == null) return ProtoResponse(404, const [], Uint8List(0));
    return ProtoResponse(200, const [('content-type', 'image/png')], img);
  }

  @override
  Stream<DaemonEvent> get events => _events.stream;

  @override
  Stream<Uint8List> get vis => const Stream.empty();

  @override
  Stream<void> get show => const Stream.empty();

  @override
  Stream<void> get quit => const Stream.empty();

  @override
  Future<void> close() async {
    await _events.close();
    await _status.close();
  }
}
