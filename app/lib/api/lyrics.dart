/// `lyrics_get` / `lyrics_sources` (`crates/daemon/src/api/lyrics.rs`,
/// model in `crates/lyrics/src/model.rs`). Times are milliseconds.
library;

import '../ipc/transport.dart';
import '_json.dart';

enum Synced { none, line, word }

class LyricWord {
  final int startMs;
  final int endMs;

  /// Keeps its trailing space when the next word is separate.
  final String text;
  const LyricWord(this.startMs, this.endMs, this.text);
  factory LyricWord.fromJson(Json j) =>
      LyricWord(asInt(j['startMs']) ?? 0, asInt(j['endMs']) ?? 0, j['text'] as String? ?? '');
}

class LyricLine {
  final int startMs;
  final int endMs;
  final String text;
  final List<LyricWord>? words;

  /// Background vocals, word-timed.
  final List<LyricWord>? background;
  const LyricLine(this.startMs, this.endMs, this.text, {this.words, this.background});
  factory LyricLine.fromJson(Json j) => LyricLine(
        asInt(j['startMs']) ?? 0,
        asInt(j['endMs']) ?? 0,
        j['text'] as String? ?? '',
        words: j['words'] == null ? null : listOf(j['words'], LyricWord.fromJson),
        background: j['background'] == null ? null : listOf(j['background'], LyricWord.fromJson),
      );
}

class Lyrics {
  /// Provider label ("lrc.red", "Apple", "LRCLIB", …).
  final String source;
  final Synced synced;
  final List<LyricLine> lines;
  final String? plain;
  final String? songwriters;
  const Lyrics(this.source, this.synced, this.lines, {this.plain, this.songwriters});
  factory Lyrics.fromJson(Json j) => Lyrics(
        j['source'] as String? ?? '',
        enumOf(Synced.values, j['synced'], Synced.none),
        listOf(j['lines'], LyricLine.fromJson),
        plain: j['plain'] as String?,
        songwriters: j['songwriters'] as String?,
      );
}

/// One source of `lyrics_sources`.
class LyricsSource {
  final String source;
  final Synced synced;
  final int lines;
  const LyricsSource(this.source, this.synced, this.lines);
  factory LyricsSource.fromJson(Json j) => LyricsSource(j['source'] as String? ?? '',
      enumOf(Synced.values, j['synced'], Synced.none), asInt(j['lines']) ?? 0);
}

/// What the providers search by. Title/artist are looked up by the daemon
/// when missing, so passing them only saves a request.
class LyricsQuery {
  final String videoId;
  final String? title;
  final String? artist;
  final String? album;
  final int? durationMs;
  final String? isrc;
  const LyricsQuery(this.videoId,
      {this.title, this.artist, this.album, this.durationMs, this.isrc});

  Map<String, Object?> toArgs({String? source}) => {
        'videoId': videoId,
        'source': ?source,
        'title': ?title,
        'artist': ?artist,
        'album': ?album,
        'durationMs': ?durationMs,
        'isrc': ?isrc,
      };
}

class LyricsApi {
  final Transport _t;
  const LyricsApi(this._t);

  /// The best source, or [source] when given. Null when nothing was found.
  Future<Lyrics?> get(LyricsQuery q, {String? source}) async {
    final r = await _t.call('lyrics_get', q.toArgs(source: source));
    return r == null ? null : Lyrics.fromJson(r as Json);
  }

  /// Every source that has lyrics for the track, best first.
  Future<List<LyricsSource>> sources(LyricsQuery q) async =>
      listOf(await _t.call('lyrics_sources', q.toArgs()), LyricsSource.fromJson);
}
