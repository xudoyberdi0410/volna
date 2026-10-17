/// Typed wrappers over the daemon's library commands
/// (`crates/core/src/api/library.rs`, rows in `api/library/store.rs`):
/// likes, user playlists, history, pins and the legacy import status.
///
/// Every write makes the daemon send `library_changed{kind}` ([LibraryApi.changes]);
/// `kind` is a [LikeKind] name, `user_playlist`, `history`, `pinned` or `all`.
library;

import '../ipc/transport.dart';
import '_json.dart';
import 'models.dart';
import 'models_json.dart';

export 'models_json.dart';

/// `LikeKind`. Videos are tracks; mixes are playlists.
enum LikeKind {
  track,
  album,
  artist,
  playlist;

  static LikeKind of(Item item) => switch (item) {
        TrackItem() => track,
        AlbumItem() => album,
        ArtistItem() => artist,
        PlaylistItem() => playlist,
      };
}

/// `PinKind`.
enum PinKind {
  album('album'),
  artist('artist'),
  playlist('playlist'),
  userPlaylist('user_playlist');

  final String wire;
  const PinKind(this.wire);
  static PinKind parse(Object? s) =>
      values.firstWhere((v) => v.wire == s, orElse: () => PinKind.playlist);
}

/// The id the library keys an item by (as `store::canonical_id`).
String itemId(Item item) => switch (item) {
      TrackItem(:final track) => track.videoId,
      AlbumItem(:final album) => album.browseId,
      ArtistItem(:final artist) => _strip(artist.id, 'MPLA'),
      PlaylistItem(:final playlist) => _strip(playlist.id, 'VL'),
    };

/// [id] in the library's canonical form for [kind].
String canonicalId(LikeKind kind, String id) => switch (kind) {
      LikeKind.artist => _strip(id.trim(), 'MPLA'),
      LikeKind.playlist => _strip(id.trim(), 'VL'),
      _ => id.trim(),
    };

String _strip(String s, String prefix) => s.startsWith(prefix) ? s.substring(prefix.length) : s;

DateTime _ms(Object? v) => DateTime.fromMillisecondsSinceEpoch(asInt(v) ?? 0);

/// `Liked`.
class LikedItem {
  final Item item;
  final DateTime addedAt;
  const LikedItem(this.item, this.addedAt);

  /// Null for an unknown item type.
  static LikedItem? fromJson(Json j) {
    final item = Item.fromJson(j['item'] as Json);
    return item == null ? null : LikedItem(item, _ms(j['addedAt']));
  }
}

/// `PlaylistInfo`: a user playlist without its tracks.
class UserPlaylistInfo {
  /// A UUID.
  final String id;
  final String name;
  final String description;

  /// Custom cover; otherwise draw a collage of [thumbnails].
  final String? cover;

  /// Up to four distinct track covers.
  final List<String> thumbnails;
  final int trackCount;
  final DateTime createdAt;
  final DateTime updatedAt;

  const UserPlaylistInfo({
    required this.id,
    required this.name,
    this.description = '',
    this.cover,
    this.thumbnails = const [],
    this.trackCount = 0,
    required this.createdAt,
    required this.updatedAt,
  });

  factory UserPlaylistInfo.fromJson(Json j) => UserPlaylistInfo(
        id: j['id'] as String,
        name: j['name'] as String? ?? '',
        description: j['description'] as String? ?? '',
        cover: j['cover'] as String?,
        thumbnails: [for (final t in (j['thumbnails'] as List? ?? const [])) t as String],
        trackCount: asInt(j['trackCount']) ?? 0,
        createdAt: _ms(j['createdAt']),
        updatedAt: _ms(j['updatedAt']),
      );

  /// For `pin_set{kind: user_playlist}`.
  Json toPinJson() => {'id': id, 'title': name, 'thumbnail': cover ?? thumbnails.firstOrNull};
}

/// `PlaylistEntry`.
class UserPlaylistEntry {
  final Track track;
  final DateTime addedAt;
  const UserPlaylistEntry(this.track, this.addedAt);
  factory UserPlaylistEntry.fromJson(Json j) =>
      UserPlaylistEntry(Track.fromJson(j['track'] as Json), _ms(j['addedAt']));
}

/// `PlaylistDetail`.
class UserPlaylist {
  final UserPlaylistInfo info;
  final List<UserPlaylistEntry> entries;
  const UserPlaylist(this.info, this.entries);
  factory UserPlaylist.fromJson(Json j) =>
      UserPlaylist(UserPlaylistInfo.fromJson(j), listOf(j['tracks'], UserPlaylistEntry.fromJson));

  List<Track> get tracks => [for (final e in entries) e.track];
}

/// `HistoryEntry` (named apart from the player journal's).
class HistoryItem {
  final Track track;
  final DateTime playedAt;
  const HistoryItem(this.track, this.playedAt);
  factory HistoryItem.fromJson(Json j) =>
      HistoryItem(Track.fromJson(j['track'] as Json), _ms(j['playedAt']));
}

/// `Pin`.
class Pin {
  final PinKind kind;
  final String id;
  final String title;
  final String? thumbnail;
  final DateTime pinnedAt;
  const Pin(this.kind, this.id, this.title, this.thumbnail, this.pinnedAt);
  factory Pin.fromJson(Json j) => Pin(PinKind.parse(j['kind']), j['id'] as String,
      j['title'] as String? ?? '', j['thumbnail'] as String?, _ms(j['pinnedAt']));
}

/// `LegacyStatus`.
class LegacyStatus {
  final bool imported;
  final DateTime? importedAt;
  const LegacyStatus(this.imported, this.importedAt);
  factory LegacyStatus.fromJson(Json j) => LegacyStatus(
      j['imported'] as bool? ?? false, j['importedAt'] == null ? null : _ms(j['importedAt']));
}

class LibraryApi {
  final Transport _t;
  const LibraryApi(this._t);

  /// The event the daemon sends after every library write.
  static const changedEvent = 'library_changed';

  /// `kind` of each `library_changed` event.
  Stream<String> get changes => _t.events
      .where((e) => e.name == changedEvent)
      .map((e) => ((e.payload as Json?)?['kind'] as String?) ?? 'all');

  // ---- likes -----------------------------------------------------------------

  /// Liked items of [kind], newest first.
  Future<List<LikedItem>> liked(LikeKind kind) async => [
        for (final e in ((await _t.call('library_get', {'kind': kind.name})) as List? ?? const []))
          ?LikedItem.fromJson(e as Json),
      ];

  /// Likes or unlikes [item]; returns whether it is liked now.
  Future<bool> like(Item item, {bool on = true}) async =>
      (await _t.call('library_like', {
        'kind': LikeKind.of(item).name,
        'item': on ? item.toJson() : itemId(item),
        'on': on,
      })) as bool;

  /// Unlike by id (an unlike needs no full model).
  Future<bool> unlike(LikeKind kind, String id) async =>
      (await _t.call('library_like', {'kind': kind.name, 'item': id, 'on': false})) as bool;

  Future<Map<String, bool>> isLiked(LikeKind kind, List<String> ids) async {
    if (ids.isEmpty) return const {};
    final r = (await _t.call('library_is_liked', {'kind': kind.name, 'ids': ids})) as Json;
    return {for (final e in r.entries) e.key: e.value as bool};
  }

  // ---- user playlists ----------------------------------------------------------

  Future<List<UserPlaylistInfo>> playlists() async =>
      listOf(await _t.call('playlist_list'), UserPlaylistInfo.fromJson);

  Future<UserPlaylist> playlist(String id) async =>
      UserPlaylist.fromJson((await _t.call('playlist_get', {'id': id})) as Json);

  Future<UserPlaylistInfo> createPlaylist(String name,
          {String? description, List<Track> tracks = const []}) async =>
      UserPlaylistInfo.fromJson((await _t.call('playlist_create', {
        'name': name,
        'description': ?description,
        'tracks': [for (final t in tracks) t.toJson()],
      })) as Json);

  Future<UserPlaylistInfo> renamePlaylist(String id, {String? name, String? description}) async =>
      UserPlaylistInfo.fromJson((await _t.call('playlist_rename', {
        'id': id,
        'name': ?name,
        'description': ?description,
      })) as Json);

  Future<bool> deletePlaylist(String id) async =>
      (await _t.call('playlist_delete', {'id': id})) as bool;

  /// Returns how many were added (duplicates are skipped by the daemon).
  Future<int> addToPlaylist(String id, List<Track> tracks) async =>
      asInt(await _t.call('playlist_add', {
        'id': id,
        'tracks': [for (final t in tracks) t.toJson()],
      })) ??
      0;

  Future<int> removeFromPlaylist(String id, List<String> videoIds) async =>
      asInt(await _t.call('playlist_remove', {'id': id, 'videoIds': videoIds})) ?? 0;

  /// Moves the entry at [from] to [to] (indices after removal, as `Vec::insert`).
  Future<void> movePlaylistEntry(String id, int from, int to) =>
      _t.call('playlist_move', {'id': id, 'from': from, 'to': to});

  // ---- history -------------------------------------------------------------------

  /// Newest first.
  Future<List<HistoryItem>> history({int limit = 100, int offset = 0}) async =>
      listOf(await _t.call('history_get', {'limit': limit, 'offset': offset}),
          HistoryItem.fromJson);

  Future<void> clearHistory() => _t.call('history_clear');

  // ---- pins ----------------------------------------------------------------------

  /// [item]: an [AlbumSummary]/[ArtistSummary]/[PlaylistSummary] json, a
  /// [UserPlaylistInfo.toPinJson], or `{id, title, thumbnail}`. For
  /// `on: false` the id string is enough.
  Future<bool> setPin(PinKind kind, Object item, {bool on = true}) async =>
      (await _t.call('pin_set', {'kind': kind.wire, 'item': item, 'on': on})) as bool;

  Future<bool> pinItem(Item item, {bool on = true}) => switch (item) {
        AlbumItem(:final album) => setPin(PinKind.album, on ? album.toJson() : album.browseId, on: on),
        ArtistItem(:final artist) => setPin(PinKind.artist, on ? artist.toJson() : artist.id, on: on),
        PlaylistItem(:final playlist) =>
          setPin(PinKind.playlist, on ? playlist.toJson() : playlist.id, on: on),
        TrackItem() => throw ArgumentError('tracks cannot be pinned'),
      };

  Future<List<Pin>> pins() async => listOf(await _t.call('pin_list'), Pin.fromJson);

  // ---- legacy --------------------------------------------------------------------

  Future<LegacyStatus> legacyStatus() async =>
      LegacyStatus.fromJson((await _t.call('legacy_status')) as Json);
}
