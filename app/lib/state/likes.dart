/// Track likes for the player and the wave, over `library_like` /
/// `library_is_liked` (`crates/daemon/src/api/library.rs`). A like feeds the
/// wave in the daemon. Refreshes on `library_changed{kind: track}`.
///
/// Private helper until `lib/api/library.dart` exists; unify at merge.
library;

import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/player_models.dart';
import '../ipc/transport.dart';
import 'connection.dart';
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

/// Whether the track with this video id is liked. `null` while unknown.
final trackLikedProvider =
    NotifierProvider.autoDispose.family<TrackLiked, bool?, String>(TrackLiked.new);

class TrackLiked extends Notifier<bool?> {
  final String videoId;
  TrackLiked(this.videoId);

  Transport get _t => ref.read(transportProvider);

  @override
  bool? build() {
    final sub = ref.watch(transportProvider).events.listen((e) {
      if (e.name == 'library_changed' &&
          (e.payload is! Map || (e.payload as Map)['kind'] == 'track')) {
        unawaited(_fetch());
      }
    });
    ref.onDispose(sub.cancel);
    unawaited(_fetch());
    return null;
  }

  Future<void> _fetch() async {
    try {
      final r = await _t.call('library_is_liked', {
        'kind': 'track',
        'ids': [videoId],
      });
      if (ref.mounted && r is Map) state = r[videoId] == true;
    } on DaemonException {
      // Unknown stays unknown.
    }
  }

  /// Likes or unlikes [track] (its id must be [videoId]).
  Future<void> toggle(PlayerTrack track) async {
    final on = !(state ?? false);
    state = on;
    try {
      final r = await _t.call('library_like', {
        'kind': 'track',
        'item': on ? ytmTrackJson(track) : track.id,
        'on': on,
      });
      if (ref.mounted && r is bool) state = r;
    } on DaemonException catch (e) {
      if (ref.mounted) state = !on;
      ref.read(playerMessagesProvider).add('Не удалось сохранить лайк: ${e.message}');
    }
  }
}
