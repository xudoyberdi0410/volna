/// Catalog models: the serde shapes of `crates/ytm/src/models.rs`.
///
/// Field names are snake_case on the wire (no `rename_all` on the structs);
/// enums are snake_case strings; [Item] is internally tagged by `type`.
library;

import '_json.dart';

/// `Thumbnail`. Width/height are 0 when YouTube did not report them.
class Thumbnail {
  final String url;
  final int width;
  final int height;
  const Thumbnail(this.url, this.width, this.height);

  factory Thumbnail.fromJson(Json j) =>
      Thumbnail(j['url'] as String, asInt(j['width']) ?? 0, asInt(j['height']) ?? 0);
}

/// The largest thumbnail, or null.
Thumbnail? bestThumbnail(List<Thumbnail> t) {
  if (t.isEmpty) return null;
  return t.reduce((a, b) => b.width * b.height >= a.width * a.height ? b : a);
}

List<Thumbnail> _thumbs(Object? v) => listOf(v, Thumbnail.fromJson);

/// `ArtistRef`.
class ArtistRef {
  /// Channel id (`UC…`); null for unlinked names.
  final String? id;
  final String name;
  const ArtistRef(this.id, this.name);
  factory ArtistRef.fromJson(Json j) => ArtistRef(j['id'] as String?, j['name'] as String);
}

/// `AlbumRef`.
class AlbumRef {
  final String? id;
  final String name;
  const AlbumRef(this.id, this.name);
  factory AlbumRef.fromJson(Json j) => AlbumRef(j['id'] as String?, j['name'] as String);
}

/// `TrackKind` (ytm).
enum TrackKind {
  song('song'),
  officialVideo('official_video'),
  video('video');

  final String wire;
  const TrackKind(this.wire);
  static TrackKind parse(Object? s) =>
      values.firstWhere((v) => v.wire == s, orElse: () => TrackKind.song);
}

/// `Track`.
class Track {
  final String videoId;
  final String title;
  final List<ArtistRef> artists;
  final AlbumRef? album;
  final int? durationS;
  final List<Thumbnail> thumbnails;
  final bool explicit;
  final TrackKind kind;
  final int? year;
  final String? views;
  final bool playable;

  const Track({
    required this.videoId,
    required this.title,
    this.artists = const [],
    this.album,
    this.durationS,
    this.thumbnails = const [],
    this.explicit = false,
    this.kind = TrackKind.song,
    this.year,
    this.views,
    this.playable = true,
  });

  factory Track.fromJson(Json j) => Track(
        videoId: j['video_id'] as String,
        title: j['title'] as String,
        artists: listOf(j['artists'], ArtistRef.fromJson),
        album: objOf(j['album'], AlbumRef.fromJson),
        durationS: asInt(j['duration_s']),
        thumbnails: _thumbs(j['thumbnails']),
        explicit: j['explicit'] as bool? ?? false,
        kind: TrackKind.parse(j['kind']),
        year: asInt(j['year']),
        views: j['views'] as String?,
        playable: j['playable'] as bool? ?? true,
      );
}

/// `AlbumKind`.
enum AlbumKind {
  album,
  single,
  ep;

  static AlbumKind parse(Object? s) =>
      values.firstWhere((v) => v.name == s, orElse: () => AlbumKind.album);
}

/// `AlbumSummary`.
class AlbumSummary {
  final String browseId;
  final String? playlistId;
  final String title;
  final AlbumKind kind;
  final List<ArtistRef> artists;
  final int? year;
  final List<Thumbnail> thumbnails;
  final bool explicit;

  const AlbumSummary({
    required this.browseId,
    this.playlistId,
    required this.title,
    this.kind = AlbumKind.album,
    this.artists = const [],
    this.year,
    this.thumbnails = const [],
    this.explicit = false,
  });

  factory AlbumSummary.fromJson(Json j) => AlbumSummary(
        browseId: j['browse_id'] as String,
        playlistId: j['playlist_id'] as String?,
        title: j['title'] as String,
        kind: AlbumKind.parse(j['kind']),
        artists: listOf(j['artists'], ArtistRef.fromJson),
        year: asInt(j['year']),
        thumbnails: _thumbs(j['thumbnails']),
        explicit: j['explicit'] as bool? ?? false,
      );
}

/// `ArtistSummary`.
class ArtistSummary {
  final String id;
  final String name;
  final List<Thumbnail> thumbnails;
  final String? audience;
  const ArtistSummary({
    required this.id,
    required this.name,
    this.thumbnails = const [],
    this.audience,
  });

  factory ArtistSummary.fromJson(Json j) => ArtistSummary(
        id: j['id'] as String,
        name: j['name'] as String,
        thumbnails: _thumbs(j['thumbnails']),
        audience: j['audience'] as String?,
      );
}

/// `PlaylistSummary`.
class PlaylistSummary {
  /// Without the `VL` prefix.
  final String id;
  final String title;
  final ArtistRef? author;
  final List<Thumbnail> thumbnails;
  final int? trackCount;
  final String? subtitle;
  const PlaylistSummary({
    required this.id,
    required this.title,
    this.author,
    this.thumbnails = const [],
    this.trackCount,
    this.subtitle,
  });

  factory PlaylistSummary.fromJson(Json j) => PlaylistSummary(
        id: j['id'] as String,
        title: j['title'] as String,
        author: objOf(j['author'], ArtistRef.fromJson),
        thumbnails: _thumbs(j['thumbnails']),
        trackCount: asInt(j['track_count']),
        subtitle: j['subtitle'] as String?,
      );
}

/// `Item`: `{"type": "track" | "album" | "artist" | "playlist", ...fields}`.
sealed class Item {
  const Item();

  /// Null for an unknown `type` (a newer daemon).
  static Item? fromJson(Json j) => switch (j['type']) {
        'track' => TrackItem(Track.fromJson(j)),
        'album' => AlbumItem(AlbumSummary.fromJson(j)),
        'artist' => ArtistItem(ArtistSummary.fromJson(j)),
        'playlist' => PlaylistItem(PlaylistSummary.fromJson(j)),
        _ => null,
      };

  List<Thumbnail> get thumbnails;
  String get title;
}

final class TrackItem extends Item {
  final Track track;
  const TrackItem(this.track);
  @override
  List<Thumbnail> get thumbnails => track.thumbnails;
  @override
  String get title => track.title;
}

final class AlbumItem extends Item {
  final AlbumSummary album;
  const AlbumItem(this.album);
  @override
  List<Thumbnail> get thumbnails => album.thumbnails;
  @override
  String get title => album.title;
}

final class ArtistItem extends Item {
  final ArtistSummary artist;
  const ArtistItem(this.artist);
  @override
  List<Thumbnail> get thumbnails => artist.thumbnails;
  @override
  String get title => artist.name;
}

final class PlaylistItem extends Item {
  final PlaylistSummary playlist;
  const PlaylistItem(this.playlist);
  @override
  List<Thumbnail> get thumbnails => playlist.thumbnails;
  @override
  String get title => playlist.title;
}

List<Item> _items(Object? v) => [
      for (final e in (v as List? ?? const []))
        ?Item.fromJson(e as Json),
    ];

/// `Section`.
class Section {
  final String title;
  final String? strapline;
  final List<Item> items;
  const Section(this.title, this.strapline, this.items);
  factory Section.fromJson(Json j) =>
      Section(j['title'] as String, j['strapline'] as String?, _items(j['items']));
}

/// `SearchFilter`.
enum SearchFilter { all, songs, videos, albums, artists, playlists }

/// `SearchPage`.
class SearchPage {
  final Item? topResult;
  final List<Item> items;
  final String? continuation;
  const SearchPage(this.topResult, this.items, this.continuation);
  factory SearchPage.fromJson(Json j) => SearchPage(
        objOf(j['top_result'], Item.fromJson),
        _items(j['items']),
        j['continuation'] as String?,
      );
}

/// `SearchSuggestions`.
class SearchSuggestions {
  final List<String> queries;
  final List<Item> items;
  const SearchSuggestions(this.queries, this.items);
  factory SearchSuggestions.fromJson(Json j) => SearchSuggestions(
        [for (final q in (j['queries'] as List? ?? const [])) q as String],
        _items(j['items']),
      );
}

/// `Album`.
class Album {
  final String browseId;
  final String? playlistId;
  final String title;
  final AlbumKind kind;
  final List<ArtistRef> artists;
  final int? year;
  final String? description;
  final List<Thumbnail> thumbnails;
  final bool explicit;
  final int? trackCount;
  final int durationS;
  final List<Track> tracks;
  final List<AlbumSummary> related;

  const Album({
    required this.browseId,
    this.playlistId,
    required this.title,
    this.kind = AlbumKind.album,
    this.artists = const [],
    this.year,
    this.description,
    this.thumbnails = const [],
    this.explicit = false,
    this.trackCount,
    this.durationS = 0,
    this.tracks = const [],
    this.related = const [],
  });

  factory Album.fromJson(Json j) => Album(
        browseId: j['browse_id'] as String,
        playlistId: j['playlist_id'] as String?,
        title: j['title'] as String,
        kind: AlbumKind.parse(j['kind']),
        artists: listOf(j['artists'], ArtistRef.fromJson),
        year: asInt(j['year']),
        description: j['description'] as String?,
        thumbnails: _thumbs(j['thumbnails']),
        explicit: j['explicit'] as bool? ?? false,
        trackCount: asInt(j['track_count']),
        durationS: asInt(j['duration_s']) ?? 0,
        tracks: listOf(j['tracks'], Track.fromJson),
        related: listOf(j['related'], AlbumSummary.fromJson),
      );
}

/// `Artist`.
class Artist {
  final String id;
  final String name;
  final String? description;
  final List<Thumbnail> thumbnails;
  final String? subscribers;
  final String? shufflePlaylistId;
  final String? radioPlaylistId;
  final List<Track> topSongs;
  final String? topSongsPlaylistId;
  final List<AlbumSummary> albums;
  final List<AlbumSummary> singles;
  final List<Track> videos;
  final List<PlaylistSummary> playlists;
  final List<ArtistSummary> related;

  const Artist({
    required this.id,
    required this.name,
    this.description,
    this.thumbnails = const [],
    this.subscribers,
    this.shufflePlaylistId,
    this.radioPlaylistId,
    this.topSongs = const [],
    this.topSongsPlaylistId,
    this.albums = const [],
    this.singles = const [],
    this.videos = const [],
    this.playlists = const [],
    this.related = const [],
  });

  factory Artist.fromJson(Json j) => Artist(
        id: j['id'] as String,
        name: j['name'] as String,
        description: j['description'] as String?,
        thumbnails: _thumbs(j['thumbnails']),
        subscribers: j['subscribers'] as String?,
        shufflePlaylistId: j['shuffle_playlist_id'] as String?,
        radioPlaylistId: j['radio_playlist_id'] as String?,
        topSongs: listOf(j['top_songs'], Track.fromJson),
        topSongsPlaylistId: j['top_songs_playlist_id'] as String?,
        albums: listOf(j['albums'], AlbumSummary.fromJson),
        singles: listOf(j['singles'], AlbumSummary.fromJson),
        videos: listOf(j['videos'], Track.fromJson),
        playlists: listOf(j['playlists'], PlaylistSummary.fromJson),
        related: listOf(j['related'], ArtistSummary.fromJson),
      );
}

/// `Playlist`.
class Playlist {
  final String id;
  final String title;
  final ArtistRef? author;
  final String? description;
  final List<Thumbnail> thumbnails;
  final int? year;
  final int? trackCount;
  final List<Track> tracks;
  final String? continuation;

  const Playlist({
    required this.id,
    required this.title,
    this.author,
    this.description,
    this.thumbnails = const [],
    this.year,
    this.trackCount,
    this.tracks = const [],
    this.continuation,
  });

  factory Playlist.fromJson(Json j) => Playlist(
        id: j['id'] as String,
        title: j['title'] as String,
        author: objOf(j['author'], ArtistRef.fromJson),
        description: j['description'] as String?,
        thumbnails: _thumbs(j['thumbnails']),
        year: asInt(j['year']),
        trackCount: asInt(j['track_count']),
        tracks: listOf(j['tracks'], Track.fromJson),
        continuation: j['continuation'] as String?,
      );
}

/// `TrackPage`.
class TrackPage {
  final List<Track> tracks;
  final String? continuation;
  const TrackPage(this.tracks, this.continuation);
  factory TrackPage.fromJson(Json j) =>
      TrackPage(listOf(j['tracks'], Track.fromJson), j['continuation'] as String?);
}

/// `WatchQueue`.
class WatchQueue {
  final String? playlistId;
  final List<Track> tracks;
  final String? continuation;
  final String? lyricsBrowseId;
  final String? relatedBrowseId;
  const WatchQueue(this.playlistId, this.tracks, this.continuation, this.lyricsBrowseId,
      this.relatedBrowseId);
  factory WatchQueue.fromJson(Json j) => WatchQueue(
        j['playlist_id'] as String?,
        listOf(j['tracks'], Track.fromJson),
        j['continuation'] as String?,
        j['lyrics_browse_id'] as String?,
        j['related_browse_id'] as String?,
      );
}

/// `TrackDetails`.
class TrackDetails {
  final Track track;
  final String? lyricsBrowseId;
  final String? relatedBrowseId;
  const TrackDetails(this.track, this.lyricsBrowseId, this.relatedBrowseId);
  factory TrackDetails.fromJson(Json j) => TrackDetails(
        Track.fromJson(j['track'] as Json),
        j['lyrics_browse_id'] as String?,
        j['related_browse_id'] as String?,
      );
}

/// `Related`.
class Related {
  final List<Track> tracks;
  final List<ArtistSummary> artists;
  final List<AlbumSummary> albums;
  final List<PlaylistSummary> playlists;
  final String? artistDescription;
  const Related(this.tracks, this.artists, this.albums, this.playlists, this.artistDescription);
  factory Related.fromJson(Json j) => Related(
        listOf(j['tracks'], Track.fromJson),
        listOf(j['artists'], ArtistSummary.fromJson),
        listOf(j['albums'], AlbumSummary.fromJson),
        listOf(j['playlists'], PlaylistSummary.fromJson),
        j['artist_description'] as String?,
      );
}

/// `Lyrics` (ytm, plain text).
class PlainLyrics {
  final String text;
  final String? source;
  const PlainLyrics(this.text, this.source);
  factory PlainLyrics.fromJson(Json j) => PlainLyrics(j['text'] as String, j['source'] as String?);
}

/// `Mood`.
class Mood {
  final String title;
  final String params;

  /// ARGB.
  final int? color;
  const Mood(this.title, this.params, this.color);
  factory Mood.fromJson(Json j) =>
      Mood(j['title'] as String, j['params'] as String, asInt(j['color']));
}

/// `MoodCategory`.
class MoodCategory {
  final String title;
  final List<Mood> moods;
  const MoodCategory(this.title, this.moods);
  factory MoodCategory.fromJson(Json j) =>
      MoodCategory(j['title'] as String, listOf(j['moods'], Mood.fromJson));
}
