/// Lyrics of a track (`lyrics_get`) and the user's source choice per track.
library;

import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/lyrics.dart';
import '../api/player_models.dart';
import 'connection.dart';

final lyricsApiProvider = Provider((ref) => LyricsApi(ref.watch(transportProvider)));

/// A family key: what to ask for, with value equality.
typedef LyricsKey = ({
  String videoId,
  String? title,
  String? artist,
  String? album,
  int? durationMs,
  String? source,
});

LyricsKey lyricsKeyFor(PlayerTrack t, {String? source}) => (
      videoId: t.id,
      title: t.title.isEmpty ? null : t.title,
      artist: t.artistNames.isEmpty ? null : t.artistNames,
      album: t.album?.title.isEmpty ?? true ? null : t.album!.title,
      durationMs: t.duration == null ? null : (t.duration! * 1000).round(),
      source: source,
    );

LyricsQuery _query(LyricsKey k) => LyricsQuery(k.videoId,
    title: k.title, artist: k.artist, album: k.album, durationMs: k.durationMs);

final lyricsProvider = FutureProvider.autoDispose.family<Lyrics?, LyricsKey>((ref, k) {
  // Keep a fetched text for a while after the last listener goes away
  // (closing and reopening the player, switching tabs).
  final link = ref.keepAlive();
  Timer? timer;
  ref.onCancel(() => timer = Timer(const Duration(minutes: 10), link.close));
  ref.onResume(() => timer?.cancel());
  ref.onDispose(() => timer?.cancel());
  return ref.watch(lyricsApiProvider).get(_query(k), source: k.source);
});

final lyricsSourcesProvider =
    FutureProvider.autoDispose.family<List<LyricsSource>, LyricsKey>((ref, k) {
  return ref.watch(lyricsApiProvider).sources(_query((
        videoId: k.videoId,
        title: k.title,
        artist: k.artist,
        album: k.album,
        durationMs: k.durationMs,
        source: null,
      )));
});

/// The source the user picked, per video id (session only).
final lyricsSourceChoiceProvider =
    NotifierProvider<LyricsSourceChoice, Map<String, String>>(LyricsSourceChoice.new);

class LyricsSourceChoice extends Notifier<Map<String, String>> {
  @override
  Map<String, String> build() => const {};

  void choose(String videoId, String source) => state = {...state, videoId: source};
}
