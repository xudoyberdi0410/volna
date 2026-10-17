import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:volna/api/models.dart';
import 'package:volna/state/connection.dart';
import 'package:volna/state/library.dart';

import 'support/fake_library.dart';
import 'support/fake_transport_browse.dart';

const _track = Track(
  videoId: 'abc',
  title: 'Song',
  artists: [ArtistRef('UC1', 'Artist')],
  album: AlbumRef('MPREb_1', 'Album'),
  durationS: 200,
  thumbnails: [Thumbnail('https://x/1.jpg', 60, 60)],
);

void main() {
  test('track json round-trips through the ytm shape', () {
    final j = _track.toJson();
    expect(j['video_id'], 'abc');
    expect(j['duration_s'], 200);
    expect(j.containsKey('year'), isTrue, reason: 'serde needs every field');
    final back = Track.fromJson(j);
    expect(back.album!.id, 'MPREb_1');
    expect(TrackItem(_track).toJson()['type'], 'track');
  });

  test('canonical ids strip VL and MPLA', () {
    expect(canonicalId(LikeKind.playlist, 'VLPL1'), 'PL1');
    expect(canonicalId(LikeKind.artist, 'MPLAUC1'), 'UC1');
    expect(canonicalId(LikeKind.track, ' x '), 'x');
  });

  test('library api: like, playlists, history', () async {
    final t = FakeTransport();
    FakeLibrary(t);
    final api = LibraryApi(t);
    expect(await api.like(const TrackItem(_track)), isTrue);
    expect(t.callsOf('library_like').single.args['item'], isA<Map>());
    expect((await api.liked(LikeKind.track)).single.item, isA<TrackItem>());
    expect(await api.isLiked(LikeKind.track, ['abc', 'zzz']), {'abc': true, 'zzz': false});
    await api.like(const TrackItem(_track), on: false);
    expect(t.callsOf('library_like').last.args['item'], 'abc');

    final pl = await api.createPlaylist('Мой', tracks: [_track]);
    expect(pl.trackCount, 1);
    expect(await api.addToPlaylist(pl.id, [_track]), 0);
    final full = await api.playlist(pl.id);
    expect(full.tracks.single.videoId, 'abc');
    expect((await api.renamePlaylist(pl.id, name: 'Новый')).name, 'Новый');
    expect(await api.deletePlaylist(pl.id), isTrue);
  });

  test('likes provider loads, toggles optimistically and follows events', () async {
    final t = FakeTransport();
    final lib = FakeLibrary(t);
    lib.likes['album']!.add({
      'item': {'type': 'album', 'browse_id': 'MPREb_9', 'title': 'A', 'artists': [], 'thumbnails': []},
      'addedAt': 1,
    });
    final c = ProviderContainer(overrides: [transportProvider.overrideWithValue(t)]);
    addTearDown(c.dispose);
    c.listen(likesProvider, (_, _) {});
    await pumpEventQueue();
    expect(c.read(likesProvider).loaded, isTrue);
    expect(c.read(likesProvider).contains(LikeKind.album, 'MPREb_9'), isTrue);

    final f = c.read(likesProvider.notifier).toggle(const TrackItem(_track));
    expect(c.read(isLikedProvider((LikeKind.track, 'abc'))), isTrue, reason: 'optimistic');
    expect(await f, isTrue);
    await pumpEventQueue();
    expect(c.read(isLikedProvider((LikeKind.track, 'abc'))), isTrue);

    // A change from elsewhere (another window, import) arrives as an event.
    lib.likes['track']!.clear();
    t.emit('library_changed', {'kind': 'track'});
    await pumpEventQueue();
    expect(c.read(isLikedProvider((LikeKind.track, 'abc'))), isFalse);
  });

  test('a failed like reverts', () async {
    final t = FakeTransport();
    FakeLibrary(t);
    t.on('library_like', (_) => throw Exception('boom'));
    final c = ProviderContainer(overrides: [transportProvider.overrideWithValue(t)]);
    addTearDown(c.dispose);
    c.listen(likesProvider, (_, _) {});
    await pumpEventQueue();
    await expectLater(c.read(likesProvider.notifier).toggle(const TrackItem(_track)), throwsException);
    expect(c.read(isLikedProvider((LikeKind.track, 'abc'))), isFalse);
  });
}
