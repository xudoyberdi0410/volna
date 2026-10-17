import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:volna/ipc/frame.dart';

/// The same bytes as `frames_match_the_dart_fixtures` in
/// `crates/daemon/src/ipc.rs`: change both sides together.
const fixtureHello =
    '2a000000000000007b2274223a2268656c6c6f222c22726f6c65223a2277696e646f77222c22746f6b656e223a6e756c6c7d';
const fixtureProto =
    '62000000020000007b2274223a2270726f746f222c226964223a322c226d6574686f64223a22474554222c22757269223a22766f6c6e613a2f2f696d672f3f753d7826733d3634222c2268656164657273223a5b5b22616363657074222c22696d6167652f2a225d5d7ddead';
const fixtureProtoReply =
    '50000000030000007b2274223a2270726f746f5f7265706c79222c226964223a322c22737461747573223a3230302c2268656164657273223a5b5b22636f6e74656e742d74797065222c22696d6167652f706e67225d5d7d010203';

String hex(List<int> b) => b.map((x) => x.toRadixString(16).padLeft(2, '0')).join();

Uint8List unhex(String s) => Uint8List.fromList(
    [for (var i = 0; i < s.length; i += 2) int.parse(s.substring(i, i + 2), radix: 16)]);

void main() {
  test('encodes exactly like write_frame', () {
    expect(hex(encodeFrame(Frame(const Hello(Role.window)))), fixtureHello);
    expect(
      hex(encodeFrame(Frame(
          const Proto(2, 'GET', 'volna://img/?u=x&s=64', [('accept', 'image/*')]),
          Uint8List.fromList([0xde, 0xad])))),
      fixtureProto,
    );
    expect(
      hex(encodeFrame(Frame(const ProtoReply(2, 200, [('content-type', 'image/png')]),
          Uint8List.fromList([1, 2, 3])))),
      fixtureProtoReply,
    );
  });

  test('decodes the fixtures, fed one byte at a time', () {
    final d = FrameDecoder();
    final all = [...unhex(fixtureHello), ...unhex(fixtureProto), ...unhex(fixtureProtoReply)];
    final frames = <Frame>[];
    for (final b in all) {
      frames.addAll(d.add([b]));
    }
    expect(frames, hasLength(3));
    expect((frames[0].msg as Hello).role, Role.window);
    expect((frames[0].msg as Hello).token, isNull);
    final p = frames[1].msg as Proto;
    expect((p.id, p.method, p.uri), (2, 'GET', 'volna://img/?u=x&s=64'));
    expect(p.headers, [('accept', 'image/*')]);
    expect(frames[1].body, [0xde, 0xad]);
    final r = frames[2].msg as ProtoReply;
    expect((r.id, r.status), (2, 200));
    expect(r.headers, [('content-type', 'image/png')]);
    expect(frames[2].body, [1, 2, 3]);
  });

  test('decodes several frames from one chunk', () {
    final d = FrameDecoder();
    final frames = d.add([
      ...encodeFrame(Frame(const Event('player:wave', {'active': true}))),
      ...encodeFrame(Frame(const Vis(), Uint8List.fromList([9]))),
      ...encodeFrame(Frame(const Show())),
      ...encodeFrame(Frame(const Quit())),
    ]);
    expect(frames.map((f) => f.msg.runtimeType), [Event, Vis, Show, Quit]);
    expect((frames[0].msg as Event).payload, {'active': true});
    expect(frames[1].body, [9]);
  });

  test('reply: err and default ok, as serde reads them', () {
    final d = FrameDecoder();
    final head = '{"t":"reply","id":3,"err":"bad_args: x"}'.codeUnits;
    final f = d.add([head.length, 0, 0, 0, 0, 0, 0, 0, ...head]).single;
    final r = f.msg as Reply;
    expect((r.id, r.ok, r.err), (3, null, 'bad_args: x'));
    expect(Msg.fromJson({'t': 'welcome', 'stream_base': 'http://127.0.0.1:1/'}),
        isA<Welcome>().having((w) => w.streamBase, 'streamBase', 'http://127.0.0.1:1/'));
  });

  test('call keeps the Rust key order', () {
    final bytes = encodeFrame(Frame(const Call(7, 'player_seek', {'position': 12.5})));
    expect(String.fromCharCodes(bytes.sublist(8)),
        '{"t":"call","id":7,"cmd":"player_seek","args":{"position":12.5}}');
  });

  test('oversized frames are refused', () {
    expect(() => FrameDecoder().add([0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0]), throwsFormatException);
    // Body 0x04000001 > 64 MB.
    expect(() => FrameDecoder().add([0, 0, 0, 0, 0x01, 0x00, 0x00, 0x04]), throwsFormatException);
  });
}
