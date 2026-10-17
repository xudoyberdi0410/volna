/// Track likes for the player and the wave: a view of [likesProvider]
/// (`state/library.dart`), the one source of truth for likes. A like feeds
/// the wave in the daemon.
library;

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/models.dart';
import '../api/player_models.dart';
import 'library.dart';
import 'player.dart';

/// The ytm `Track` JSON (snake_case) the daemon stores for a liked entry.
Map<String, Object?> ytmTrackJson(PlayerTrack t) => {
      'video_id': t.id,
      'title': t.title,
      'artists': [for (final a in t.artists) a.toJson()],
      'album': t.album == null ? null : {'id': t.album!.id, 'name': t.album!.title},
      'duration_s': t.duration?.round(),
      'thumbnails': [
        if (t.cover != null) {'url': t.cover, 'width': 0, 'height': 0},
      ],
      'explicit': t.explicit,
      'kind': t.kind == PlayerTrackKind.track ? 'song' : 'video',
      'year': null,
      'views': null,
      'playable': !t.unavailable,
    };

/// The catalog track behind a queue entry: the full model the browse
/// screens put in `ui` when they queued it, else rebuilt from the entry.
Track ytmTrack(PlayerTrack t) {
  final ui = t.ui;
  if (ui is Map && ui['video_id'] == t.id) {
    try {
      return Track.fromJson(ui.cast<String, Object?>());
    } catch (_) {
      // Not a ytm track after all.
    }
  }
  return Track.fromJson(ytmTrackJson(t));
}

/// Whether the track with this video id is liked. `null` until the likes are loaded.
final trackLikedProvider =
    NotifierProvider.autoDispose.family<TrackLiked, bool?, String>(TrackLiked.new);

class TrackLiked extends Notifier<bool?> {
  final String videoId;
  TrackLiked(this.videoId);

  @override
  bool? build() {
    final likes = ref.watch(likesProvider);
    final on = likes.contains(LikeKind.track, videoId);
    return on || likes.loaded ? on : null;
  }

  /// Likes or unlikes [track] (its id must be [videoId]).
  Future<void> toggle(PlayerTrack track) async {
    final on = !(state ?? false);
    try {
      await ref.read(likesProvider.notifier).set(TrackItem(ytmTrack(track)), on);
    } catch (e) {
      final msg = e is Exception ? e.toString() : '$e';
      ref.read(playerMessagesProvider).add('Не удалось сохранить лайк: $msg');
    }
  }
}
