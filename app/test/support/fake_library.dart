/// An in-memory daemon library on a [FakeTransport]: the library commands
/// of `crates/core/src/api/library.rs` with the same shapes, emitting
/// `library_changed` like the daemon.
library;

import 'fake_transport_browse.dart';

class FakeLibrary {
  final FakeTransport t;
  final likes = <String, List<Map<String, Object?>>>{
    'track': [],
    'album': [],
    'artist': [],
    'playlist': [],
  };
  final playlists = <String, Map<String, Object?>>{};
  final playlistTracks = <String, List<Map<String, Object?>>>{};
  final history = <Map<String, Object?>>[];
  final pins = <Map<String, Object?>>[];
  int _ids = 0;
  int now = 1760000000000;

  FakeLibrary(this.t) {
    t.on('library_get', (a) => likes[a['kind']]!);
    t.on('library_like', _like);
    t.on('library_is_liked', (a) {
      final kind = a['kind'] as String;
      final have = {for (final l in likes[kind]!) _id(kind, l['item'])};
      return {for (final id in (a['ids'] as List)) id as String: have.contains(id)};
    });
    t.on('playlist_list', (_) => [for (final id in playlists.keys) _info(id)]);
    t.on('playlist_get', (a) => {
          ..._info(a['id'] as String),
          'tracks': [
            for (final tr in playlistTracks[a['id']]!) {'track': tr, 'addedAt': now}
          ],
        });
    t.on('playlist_create', (a) {
      final id = 'pl-${++_ids}';
      playlists[id] = {'name': a['name'], 'description': a['description'] ?? ''};
      playlistTracks[id] = [...((a['tracks'] as List?) ?? const []).cast()];
      _changed('user_playlist');
      return _info(id);
    });
    t.on('playlist_rename', (a) {
      final id = a['id'] as String;
      if (a['name'] != null) playlists[id]!['name'] = a['name'];
      _changed('user_playlist');
      return _info(id);
    });
    t.on('playlist_delete', (a) {
      final gone = playlists.remove(a['id']) != null;
      playlistTracks.remove(a['id']);
      _changed('user_playlist');
      return gone;
    });
    t.on('playlist_add', (a) {
      final list = playlistTracks[a['id']]!;
      var n = 0;
      for (final tr in (a['tracks'] as List).cast<Map<String, Object?>>()) {
        if (list.any((x) => x['video_id'] == tr['video_id'])) continue;
        list.add(tr);
        n++;
      }
      _changed('user_playlist');
      return n;
    });
    t.on('playlist_remove', (a) {
      final ids = (a['videoIds'] as List).toSet();
      final list = playlistTracks[a['id']]!;
      final before = list.length;
      list.removeWhere((x) => ids.contains(x['video_id']));
      _changed('user_playlist');
      return before - list.length;
    });
    t.on('playlist_move', (a) {
      final list = playlistTracks[a['id']]!;
      final v = list.removeAt(a['from'] as int);
      list.insert((a['to'] as int).clamp(0, list.length), v);
      _changed('user_playlist');
      return null;
    });
    t.on('history_get', (a) => history);
    t.on('history_clear', (_) {
      history.clear();
      _changed('history');
      return null;
    });
    t.on('pin_list', (_) => pins);
    t.on('pin_set', (a) {
      final kind = a['kind'] as String;
      final item = a['item'];
      final id = item is String ? item : ((item as Map)['browse_id'] ?? item['id']) as String;
      pins.removeWhere((p) => p['kind'] == kind && p['id'] == id);
      if (a['on'] == true) {
        final m = item as Map;
        pins.insert(0, {
          'kind': kind,
          'id': id,
          'title': m['title'] ?? m['name'] ?? '',
          'thumbnail': m['thumbnail'],
          'pinnedAt': now,
        });
      }
      _changed('pinned');
      return a['on'];
    });
    t.on('legacy_status', (_) => {'imported': true, 'importedAt': now});
  }

  static String _id(String kind, Object? item) {
    if (item is String) return item;
    final m = item as Map;
    return (m['video_id'] ?? m['browse_id'] ?? m['id']) as String;
  }

  Object? _like(Map<String, Object?> a) {
    final kind = a['kind'] as String;
    final id = _id(kind, a['item']);
    final list = likes[kind]!;
    list.removeWhere((l) => _id(kind, l['item']) == id);
    if (a['on'] == true) list.insert(0, {'item': a['item'], 'addedAt': now});
    _changed(kind);
    return a['on'];
  }

  Map<String, Object?> _info(String id) {
    final p = playlists[id]!;
    final tracks = playlistTracks[id]!;
    return {
      'id': id,
      'name': p['name'],
      'description': p['description'],
      'cover': null,
      'thumbnails': [
        for (final tr in tracks.take(4))
          if ((tr['thumbnails'] as List).isNotEmpty) (tr['thumbnails'] as List).first['url'],
      ],
      'trackCount': tracks.length,
      'createdAt': now,
      'updatedAt': now,
    };
  }

  void _changed(String kind) => t.emit('library_changed', {'kind': kind});
}
