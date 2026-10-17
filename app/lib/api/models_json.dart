/// Encoders for the catalog models: the exact serde shapes of
/// `crates/ytm/src/models.rs`, for commands that take a model back
/// (`library_like`, `playlist_add`, `pin_set`, …). Every field is written,
/// nulls included: the Rust structs have no `#[serde(default)]`.
library;

import '_json.dart';
import 'models.dart';

extension ThumbnailJson on Thumbnail {
  Json toJson() => {'url': url, 'width': width, 'height': height};
}

List<Json> _thumbs(List<Thumbnail> t) => [for (final x in t) x.toJson()];

extension ArtistRefJson on ArtistRef {
  Json toJson() => {'id': id, 'name': name};
}

extension AlbumRefJson on AlbumRef {
  Json toJson() => {'id': id, 'name': name};
}

extension TrackJson on Track {
  Json toJson() => {
        'video_id': videoId,
        'title': title,
        'artists': [for (final a in artists) a.toJson()],
        'album': album?.toJson(),
        'duration_s': durationS,
        'thumbnails': _thumbs(thumbnails),
        'explicit': explicit,
        'kind': kind.wire,
        'year': year,
        'views': views,
        'playable': playable,
      };
}

extension AlbumSummaryJson on AlbumSummary {
  Json toJson() => {
        'browse_id': browseId,
        'playlist_id': playlistId,
        'title': title,
        'kind': kind.name,
        'artists': [for (final a in artists) a.toJson()],
        'year': year,
        'thumbnails': _thumbs(thumbnails),
        'explicit': explicit,
      };
}

extension ArtistSummaryJson on ArtistSummary {
  Json toJson() => {
        'id': id,
        'name': name,
        'thumbnails': _thumbs(thumbnails),
        'audience': audience,
      };
}

extension PlaylistSummaryJson on PlaylistSummary {
  Json toJson() => {
        'id': id,
        'title': title,
        'author': author?.toJson(),
        'thumbnails': _thumbs(thumbnails),
        'track_count': trackCount,
        'subtitle': subtitle,
      };
}

extension ItemJson on Item {
  /// Tagged by `type`, as `Item` deserialises.
  Json toJson() => switch (this) {
        TrackItem(:final track) => {'type': 'track', ...track.toJson()},
        AlbumItem(:final album) => {'type': 'album', ...album.toJson()},
        ArtistItem(:final artist) => {'type': 'artist', ...artist.toJson()},
        PlaylistItem(:final playlist) => {'type': 'playlist', ...playlist.toJson()},
      };
}

/// Summaries of full pages, for liking/pinning from an album/artist/playlist screen.
extension AlbumToSummary on Album {
  AlbumSummary toSummary() => AlbumSummary(
        browseId: browseId,
        playlistId: playlistId,
        title: title,
        kind: kind,
        artists: artists,
        year: year,
        thumbnails: thumbnails,
        explicit: explicit,
      );
}

extension ArtistToSummary on Artist {
  ArtistSummary toSummary() =>
      ArtistSummary(id: id, name: name, thumbnails: thumbnails, audience: subscribers);
}

extension PlaylistToSummary on Playlist {
  PlaylistSummary toSummary() => PlaylistSummary(
        id: id,
        title: title,
        author: author,
        thumbnails: thumbnails,
        trackCount: trackCount,
      );
}
