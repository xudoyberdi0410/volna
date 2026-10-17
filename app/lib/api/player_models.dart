/// Player models: the serde shapes of `crates/player/src/model.rs`
/// (camelCase JSON, lowercase enums). Dropped features (DSP) stay raw JSON.
library;

import '_json.dart';
import 'models.dart' as ytm;

enum Origin { user, wave, radio, autoplay }

enum Repeat { off, all, one }

enum PlayStatus { idle, loading, playing, paused, ended }

/// `model::TrackKind` (not the ytm one).
enum PlayerTrackKind { track, video }

enum ContinuationKind { none, wave, radio, ui }

/// `model::ArtistRef`.
class PArtist {
  final String? id;
  final String name;
  const PArtist(this.id, this.name);
  factory PArtist.fromJson(Json j) => PArtist(j['id'] as String?, j['name'] as String? ?? '');
  Json toJson() => {'id': id, 'name': name};
}

/// `model::AlbumRef`.
class PAlbum {
  final String? id;
  final String title;
  const PAlbum(this.id, this.title);
  factory PAlbum.fromJson(Json j) => PAlbum(j['id'] as String?, j['title'] as String? ?? '');
  Json toJson() => {'id': id, 'title': title};
}

/// `TrackInput`: what `player_set_queue` / `player_enqueue` take.
class TrackInput {
  final String id;
  final String title;
  final List<PArtist> artists;
  final PAlbum? album;
  final double? duration;
  final String? cover;
  final bool explicit;
  final PlayerTrackKind kind;
  final Origin? origin;

  /// Opaque, stored and returned verbatim in [PlayerTrack.ui].
  final Object? ui;

  const TrackInput({
    required this.id,
    required this.title,
    this.artists = const [],
    this.album,
    this.duration,
    this.cover,
    this.explicit = false,
    this.kind = PlayerTrackKind.track,
    this.origin,
    this.ui,
  });

  /// From a catalog track; [cover] is the largest thumbnail URL.
  factory TrackInput.fromTrack(ytm.Track t, {Origin? origin}) => TrackInput(
        id: t.videoId,
        title: t.title,
        artists: [for (final a in t.artists) PArtist(a.id, a.name)],
        album: t.album == null ? null : PAlbum(t.album!.id, t.album!.name),
        duration: t.durationS?.toDouble(),
        cover: ytm.bestThumbnail(t.thumbnails)?.url,
        explicit: t.explicit,
        kind: t.kind == ytm.TrackKind.song ? PlayerTrackKind.track : PlayerTrackKind.video,
        origin: origin,
      );

  Json toJson() => {
        'id': id,
        'title': title,
        'artists': [for (final a in artists) a.toJson()],
        'album': album?.toJson(),
        'duration': duration,
        'cover': cover,
        'explicit': explicit,
        'kind': kind.name,
        'origin': origin?.name,
        'ui': ui,
      };
}

/// `PlayerTrack`: a queue entry.
class PlayerTrack {
  final String uid;
  final String id;
  final String title;
  final List<PArtist> artists;
  final PAlbum? album;
  final double? duration;
  final String? cover;
  final bool explicit;
  final PlayerTrackKind kind;
  final Origin origin;
  final Object? ui;
  final bool unavailable;

  const PlayerTrack({
    required this.uid,
    required this.id,
    required this.title,
    this.artists = const [],
    this.album,
    this.duration,
    this.cover,
    this.explicit = false,
    this.kind = PlayerTrackKind.track,
    this.origin = Origin.user,
    this.ui,
    this.unavailable = false,
  });

  factory PlayerTrack.fromJson(Json j) => PlayerTrack(
        uid: j['uid'] as String,
        id: j['id'] as String,
        title: j['title'] as String? ?? '',
        artists: listOf(j['artists'], PArtist.fromJson),
        album: objOf(j['album'], PAlbum.fromJson),
        duration: asDouble(j['duration']),
        cover: j['cover'] as String?,
        explicit: j['explicit'] as bool? ?? false,
        kind: enumOf(PlayerTrackKind.values, j['kind'], PlayerTrackKind.track),
        origin: enumOf(Origin.values, j['origin'], Origin.user),
        ui: j['ui'],
        unavailable: j['unavailable'] as bool? ?? false,
      );

  String get artistNames =>
      artists.map((a) => a.name).where((n) => n.isNotEmpty).join(', ');
}

/// `StreamInfo`. `quality` is `HIGH` / `LOW`.
class StreamInfo {
  final String? codec;
  final double? bitrateKbps;
  final String quality;
  final String? mime;
  const StreamInfo(this.codec, this.bitrateKbps, this.quality, this.mime);
  factory StreamInfo.fromJson(Json j) => StreamInfo(j['codec'] as String?,
      asDouble(j['bitrateKbps']), j['quality'] as String? ?? 'HIGH', j['mime'] as String?);
}

/// `PlayerError`. `kind`: bot_check, unavailable, no_yt_dlp, decode,
/// timeout, other; `action`: skipped, stopped, retrying, none.
class PlayerError {
  final String? uid;
  final String? trackId;
  final String kind;
  final String message;
  final String action;
  final int streak;
  const PlayerError(this.uid, this.trackId, this.kind, this.message, this.action, this.streak);
  factory PlayerError.fromJson(Json j) => PlayerError(
        j['uid'] as String?,
        j['trackId'] as String?,
        j['kind'] as String? ?? 'other',
        j['message'] as String? ?? '',
        j['action'] as String? ?? 'none',
        asInt(j['streak']) ?? 0,
      );
}

/// `PlaybackState` (also the `player:state` payload).
class PlaybackState {
  final PlayStatus status;
  final String? uid;
  final String? trackId;
  final double position;

  /// Unix ms of the [position] sample.
  final double at;
  final double? duration;
  final double rate;
  final bool buffering;
  final bool crossfading;
  final double volume;
  final bool muted;
  final StreamInfo? stream;
  final PlayerError? error;

  const PlaybackState({
    this.status = PlayStatus.idle,
    this.uid,
    this.trackId,
    this.position = 0,
    this.at = 0,
    this.duration,
    this.rate = 1,
    this.buffering = false,
    this.crossfading = false,
    this.volume = 0.7,
    this.muted = false,
    this.stream,
    this.error,
  });

  factory PlaybackState.fromJson(Json j) => PlaybackState(
        status: enumOf(PlayStatus.values, j['status'], PlayStatus.idle),
        uid: j['uid'] as String?,
        trackId: j['trackId'] as String?,
        position: asDouble(j['position']) ?? 0,
        at: asDouble(j['at']) ?? 0,
        duration: asDouble(j['duration']),
        rate: asDouble(j['rate']) ?? 1,
        buffering: j['buffering'] as bool? ?? false,
        crossfading: j['crossfading'] as bool? ?? false,
        volume: asDouble(j['volume']) ?? 0.7,
        muted: j['muted'] as bool? ?? false,
        stream: objOf(j['stream'], StreamInfo.fromJson),
        error: objOf(j['error'], PlayerError.fromJson),
      );
}

/// `Continuation`.
class Continuation {
  final ContinuationKind kind;
  final bool fetching;
  const Continuation(this.kind, this.fetching);
  factory Continuation.fromJson(Json j) => Continuation(
      enumOf(ContinuationKind.values, j['kind'], ContinuationKind.none),
      j['fetching'] as bool? ?? false);
}

List<String>? _strings(Object? v) => v == null ? null : [for (final s in v as List) s as String];

/// `QueueSnapshot`.
class QueueSnapshot {
  final int rev;
  final List<PlayerTrack> items;
  final List<String>? original;
  final int index;
  final bool shuffle;
  final Repeat repeat;
  final Continuation continuation;
  const QueueSnapshot(this.rev, this.items, this.original, this.index, this.shuffle, this.repeat,
      this.continuation);
  factory QueueSnapshot.fromJson(Json j) => QueueSnapshot(
        asInt(j['rev']) ?? 0,
        listOf(j['items'], PlayerTrack.fromJson),
        _strings(j['original']),
        asInt(j['index']) ?? -1,
        j['shuffle'] as bool? ?? false,
        enumOf(Repeat.values, j['repeat'], Repeat.off),
        Continuation.fromJson(j['continuation'] as Json? ?? const {}),
      );
}

/// `QueueChanged` (the `player:queue` payload): the new order by uid plus
/// the entries that were added or updated since the previous rev.
class QueueChanged {
  final int rev;
  final List<String> order;
  final List<String>? original;
  final int index;
  final bool shuffle;
  final Repeat repeat;
  final Continuation continuation;
  final List<PlayerTrack> added;
  final List<PlayerTrack> updated;
  const QueueChanged(this.rev, this.order, this.original, this.index, this.shuffle, this.repeat,
      this.continuation, this.added, this.updated);
  factory QueueChanged.fromJson(Json j) => QueueChanged(
        asInt(j['rev']) ?? 0,
        _strings(j['order']) ?? const [],
        _strings(j['original']),
        asInt(j['index']) ?? -1,
        j['shuffle'] as bool? ?? false,
        enumOf(Repeat.values, j['repeat'], Repeat.off),
        Continuation.fromJson(j['continuation'] as Json? ?? const {}),
        listOf(j['added'], PlayerTrack.fromJson),
        listOf(j['updated'], PlayerTrack.fromJson),
      );
}

/// `PlayerSettings`. [raw] keeps every key for patches built from it.
class PlayerSettings {
  final double volume;
  final bool muted;

  /// `linear` / `cubic`.
  final String volumeCurve;

  /// `high` / `low`.
  final String quality;
  final bool crossfadeEnabled;
  final double crossfadeSeconds;
  final bool gapless;
  final double speed;
  final bool autoplay;
  final Json raw;

  const PlayerSettings({
    this.volume = 0.7,
    this.muted = false,
    this.volumeCurve = 'linear',
    this.quality = 'high',
    this.crossfadeEnabled = false,
    this.crossfadeSeconds = 5,
    this.gapless = true,
    this.speed = 1,
    this.autoplay = false,
    this.raw = const {},
  });

  factory PlayerSettings.fromJson(Json j) {
    final cf = j['crossfade'] as Json? ?? const {};
    return PlayerSettings(
      volume: asDouble(j['volume']) ?? 0.7,
      muted: j['muted'] as bool? ?? false,
      volumeCurve: j['volumeCurve'] as String? ?? 'linear',
      quality: j['quality'] as String? ?? 'high',
      crossfadeEnabled: cf['enabled'] as bool? ?? false,
      crossfadeSeconds: asDouble(cf['seconds']) ?? 5,
      gapless: j['gapless'] as bool? ?? true,
      speed: asDouble(j['speed']) ?? 1,
      autoplay: j['autoplay'] as bool? ?? false,
      raw: j,
    );
  }
}

/// `PlayerSnapshot` (`player_snapshot`).
class PlayerSnapshot {
  final int protocol;
  final PlaybackState state;
  final QueueSnapshot queue;
  final PlayerSettings settings;

  /// DSP is not ported (plan: dropped); kept raw.
  final Json dsp;

  /// Unix ms, when a sleep timer runs.
  final double? sleepEndsAt;
  final bool waveActive;
  final int journalSeq;

  const PlayerSnapshot(this.protocol, this.state, this.queue, this.settings, this.dsp,
      this.sleepEndsAt, this.waveActive, this.journalSeq);

  factory PlayerSnapshot.fromJson(Json j) => PlayerSnapshot(
        asInt(j['protocol']) ?? 0,
        PlaybackState.fromJson(j['state'] as Json),
        QueueSnapshot.fromJson(j['queue'] as Json),
        PlayerSettings.fromJson(j['settings'] as Json? ?? const {}),
        j['dsp'] as Json? ?? const {},
        asDouble((j['sleepTimer'] as Json?)?['endsAt']),
        (j['wave'] as Json?)?['active'] as bool? ?? false,
        asInt(j['journalSeq']) ?? 0,
      );
}

/// `PositionEvent` (`player:position`).
class PositionEvent {
  final String? uid;
  final double position;
  final double? duration;
  final double rate;
  final double at;
  final bool playing;
  const PositionEvent(this.uid, this.position, this.duration, this.rate, this.at, this.playing);
  factory PositionEvent.fromJson(Json j) => PositionEvent(
        j['uid'] as String?,
        asDouble(j['position']) ?? 0,
        asDouble(j['duration']),
        asDouble(j['rate']) ?? 1,
        asDouble(j['at']) ?? 0,
        j['playing'] as bool? ?? false,
      );
}

/// `JournalEntry` (tag `kind`: `history` | `end`).
sealed class JournalEntry {
  final int seq;
  final double at;
  final PlayerTrack track;
  const JournalEntry(this.seq, this.at, this.track);

  static JournalEntry? fromJson(Json j) {
    final seq = asInt(j['seq']) ?? 0;
    final at = asDouble(j['at']) ?? 0;
    final track = PlayerTrack.fromJson(j['track'] as Json);
    return switch (j['kind']) {
      'history' => HistoryEntry(seq, at, track),
      'end' => EndEntry(
          seq,
          at,
          track,
          startedAt: asDouble(j['startedAt']) ?? 0,
          position: asDouble(j['position']) ?? 0,
          duration: asDouble(j['duration']),
          ended: j['ended'] as bool? ?? false,
          inWave: j['inWave'] as bool? ?? false,
        ),
      _ => null,
    };
  }
}

/// Played ≥ 10 s.
final class HistoryEntry extends JournalEntry {
  const HistoryEntry(super.seq, super.at, super.track);
}

/// A started play closed.
final class EndEntry extends JournalEntry {
  final double startedAt;
  final double position;
  final double? duration;
  final bool ended;
  final bool inWave;
  const EndEntry(super.seq, super.at, super.track,
      {required this.startedAt,
      required this.position,
      this.duration,
      required this.ended,
      required this.inWave});
}

/// Typed player events (`player/forward.rs`).
sealed class PlayerEvent {
  const PlayerEvent();

  /// Null for events that are not the player's or are unknown.
  static PlayerEvent? parse(String name, Object? payload) {
    final j = payload is Json ? payload : const <String, Object?>{};
    return switch (name) {
      'player:state' => StateEvent(PlaybackState.fromJson(j)),
      'player:position' => PositionChanged(PositionEvent.fromJson(j)),
      'player:queue' => QueueEvent(QueueChanged.fromJson(j)),
      'player:error' => ErrorEvent(PlayerError.fromJson(j)),
      'player:settings' => SettingsEvent(PlayerSettings.fromJson(j['settings'] as Json? ?? const {})),
      'player:sleep' => SleepEvent(asDouble(j['endsAt'])),
      'player:wave' => WaveEvent(j['active'] as bool? ?? false),
      'player:queue-low' => QueueLowEvent(asInt(j['remaining']) ?? 0),
      'player:journal' => switch (JournalEntry.fromJson(j)) {
          final e? => JournalEvent(e),
          null => null,
        },
      _ => null,
    };
  }
}

final class StateEvent extends PlayerEvent {
  final PlaybackState state;
  const StateEvent(this.state);
}

final class PositionChanged extends PlayerEvent {
  final PositionEvent position;
  const PositionChanged(this.position);
}

final class QueueEvent extends PlayerEvent {
  final QueueChanged queue;
  const QueueEvent(this.queue);
}

final class ErrorEvent extends PlayerEvent {
  final PlayerError error;
  const ErrorEvent(this.error);
}

/// Settings changed outside the window (MPRIS, tray).
final class SettingsEvent extends PlayerEvent {
  final PlayerSettings settings;
  const SettingsEvent(this.settings);
}

final class SleepEvent extends PlayerEvent {
  final double? endsAt;
  const SleepEvent(this.endsAt);
}

final class WaveEvent extends PlayerEvent {
  final bool active;
  const WaveEvent(this.active);
}

final class QueueLowEvent extends PlayerEvent {
  final int remaining;
  const QueueLowEvent(this.remaining);
}

final class JournalEvent extends PlayerEvent {
  final JournalEntry entry;
  const JournalEvent(this.entry);
}
