/// Daemon ↔ window frames, byte-for-byte as `crates/daemon/src/ipc.rs`
/// (`read_frame` / `write_frame`).
///
/// A frame is `u32 LE header length, u32 LE body length, JSON header, raw
/// body`. The header is a [Msg] (serde `tag = "t"`, snake_case variants).
/// Keys are written in the Rust field order so encoded bytes match serde's.
library;

import 'dart:convert';
import 'dart:typed_data';

/// Frames with a header or body bigger than this are a protocol error.
const int maxPart = 64 * 1024 * 1024;

/// An HTTP-style header pair (`Vec<(String, String)>` in Rust, a 2-array in JSON).
typedef Header = (String, String);

List<List<String>> _headersToJson(List<Header> h) =>
    [for (final (k, v) in h) [k, v]];

List<Header> _headersFromJson(Object? v) => [
      for (final pair in (v as List? ?? const []))
        ((pair as List)[0] as String, pair[1] as String),
    ];

/// `ipc::Role`.
enum Role {
  window,
  control;

  static Role parse(String s) =>
      values.firstWhere((r) => r.name == s, orElse: () => throw FormatException('bad role $s'));
}

/// `ipc::Msg`.
sealed class Msg {
  const Msg();

  Map<String, Object?> toJson();

  static Msg fromJson(Map<String, Object?> j) {
    final t = j['t'];
    switch (t) {
      case 'hello':
        return Hello(Role.parse(j['role'] as String), token: j['token'] as String?);
      case 'welcome':
        return Welcome(j['stream_base'] as String?);
      case 'call':
        return Call(_int(j['id']), j['cmd'] as String, j['args']);
      case 'reply':
        return Reply(_int(j['id']), ok: j['ok'], err: j['err'] as String?);
      case 'proto':
        return Proto(_int(j['id']), j['method'] as String, j['uri'] as String,
            _headersFromJson(j['headers']));
      case 'proto_reply':
        return ProtoReply(_int(j['id']), _int(j['status']), _headersFromJson(j['headers']));
      case 'event':
        return Event(j['name'] as String, j['payload']);
      case 'vis':
        return const Vis();
      case 'show':
        return const Show();
      case 'quit':
        return const Quit();
      default:
        throw FormatException('unknown IPC message type $t');
    }
  }
}

int _int(Object? v) => (v as num).toInt();

final class Hello extends Msg {
  final Role role;

  /// Windows only: the token from the port file.
  final String? token;
  const Hello(this.role, {this.token});
  @override
  Map<String, Object?> toJson() => {'t': 'hello', 'role': role.name, 'token': token};
}

final class Welcome extends Msg {
  final String? streamBase;
  const Welcome(this.streamBase);
  @override
  Map<String, Object?> toJson() => {'t': 'welcome', 'stream_base': streamBase};
}

final class Call extends Msg {
  final int id;
  final String cmd;
  final Object? args;
  const Call(this.id, this.cmd, this.args);
  @override
  Map<String, Object?> toJson() => {'t': 'call', 'id': id, 'cmd': cmd, 'args': args};
}

final class Reply extends Msg {
  final int id;
  final Object? ok;
  final String? err;
  const Reply(this.id, {this.ok, this.err});
  @override
  Map<String, Object?> toJson() => {'t': 'reply', 'id': id, 'ok': ok, 'err': err};
}

/// A `volna://` request; the frame body is the request body.
final class Proto extends Msg {
  final int id;
  final String method;
  final String uri;
  final List<Header> headers;
  const Proto(this.id, this.method, this.uri, this.headers);
  @override
  Map<String, Object?> toJson() => {
        't': 'proto',
        'id': id,
        'method': method,
        'uri': uri,
        'headers': _headersToJson(headers),
      };
}

/// The frame body is the response body.
final class ProtoReply extends Msg {
  final int id;
  final int status;
  final List<Header> headers;
  const ProtoReply(this.id, this.status, this.headers);
  @override
  Map<String, Object?> toJson() =>
      {'t': 'proto_reply', 'id': id, 'status': status, 'headers': _headersToJson(headers)};
}

/// Daemon → window.
final class Event extends Msg {
  final String name;
  final Object? payload;
  const Event(this.name, this.payload);
  @override
  Map<String, Object?> toJson() => {'t': 'event', 'name': name, 'payload': payload};
}

/// Daemon → window: one visualizer frame in the body.
final class Vis extends Msg {
  const Vis();
  @override
  Map<String, Object?> toJson() => {'t': 'vis'};
}

/// Control → daemon: show the window. Daemon → window: focus it.
final class Show extends Msg {
  const Show();
  @override
  Map<String, Object?> toJson() => {'t': 'show'};
}

/// Daemon → window: exit now. Control → daemon: quit.
final class Quit extends Msg {
  const Quit();
  @override
  Map<String, Object?> toJson() => {'t': 'quit'};
}

/// `ipc::Frame`.
class Frame {
  final Msg msg;
  final Uint8List body;
  Frame(this.msg, [Uint8List? body]) : body = body ?? Uint8List(0);
}

/// `write_frame`, as one buffer.
Uint8List encodeFrame(Frame f) {
  final head = utf8.encode(jsonEncode(f.msg.toJson()));
  if (head.length > maxPart || f.body.length > maxPart) {
    throw ArgumentError('IPC frame too large');
  }
  final out = Uint8List(8 + head.length + f.body.length);
  final bd = ByteData.sublistView(out);
  bd.setUint32(0, head.length, Endian.little);
  bd.setUint32(4, f.body.length, Endian.little);
  out.setRange(8, 8 + head.length, head);
  out.setRange(8 + head.length, out.length, f.body);
  return out;
}

/// `read_frame` over a byte stream that arrives in arbitrary chunks.
///
/// [add] the chunks as they come; every complete frame is returned. Throws
/// [FormatException] on an oversized part or a bad header, after which the
/// connection must be dropped.
class FrameDecoder {
  final _buf = BytesBuilder(copy: false);

  /// Bytes needed before anything can be parsed: the length prefix, or the
  /// whole frame once its prefix is known. Avoids re-copying a large body
  /// for every chunk.
  int _need = 8;

  List<Frame> add(List<int> chunk) {
    _buf.add(chunk);
    final frames = <Frame>[];
    if (_buf.length < _need) return frames;
    final bytes = _buf.takeBytes();
    var off = 0;
    while (true) {
      final avail = bytes.length - off;
      if (avail < 8) {
        _need = 8;
        break;
      }
      final bd = ByteData.sublistView(bytes, off, off + 8);
      final head = bd.getUint32(0, Endian.little);
      final body = bd.getUint32(4, Endian.little);
      if (head > maxPart || body > maxPart) {
        throw const FormatException('IPC frame too large');
      }
      final total = 8 + head + body;
      if (avail < total) {
        _need = total;
        break;
      }
      final hStart = off + 8;
      final bStart = hStart + head;
      final json = jsonDecode(utf8.decode(Uint8List.sublistView(bytes, hStart, bStart)));
      if (json is! Map<String, Object?>) {
        throw const FormatException('IPC header is not an object');
      }
      frames.add(Frame(Msg.fromJson(json), bytes.sublist(bStart, bStart + body)));
      off += total;
    }
    if (off < bytes.length) _buf.add(Uint8List.sublistView(bytes, off));
    return frames;
  }
}
