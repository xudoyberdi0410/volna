/// Player state mirrored from the daemon (`docs/native-player.md` §10):
/// a `player_snapshot` on connect, then `player:*` events. A queue `rev` gap
/// or an unknown uid triggers a fresh snapshot.
///
/// - [playerViewProvider]: status, queue, settings, wave. Changes on state
///   and queue events only (not on position ticks).
/// - [positionProvider]: the latest position sample (250 ms ticks); widgets
///   interpolate between samples with [PositionSample.at].
/// - [playerMessagesProvider]: user-facing error texts (command failures,
///   track errors) for a snackbar.
library;

import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/player.dart';
import '../api/player_models.dart';
import '../ipc/transport.dart';
import 'connection.dart';

/// Wall clock in Unix ms; overridable so tests freeze time.
final clockProvider = Provider<int Function()>((ref) => () => DateTime.now().millisecondsSinceEpoch);

class PlayerView {
  /// A snapshot has been applied.
  final bool ready;
  final PlaybackState state;
  final List<PlayerTrack> items;
  final int index;
  final int rev;
  final bool shuffle;
  final Repeat repeat;
  final Continuation continuation;
  final bool waveActive;
  final PlayerSettings settings;

  const PlayerView({
    this.ready = false,
    this.state = const PlaybackState(),
    this.items = const [],
    this.index = -1,
    this.rev = 0,
    this.shuffle = false,
    this.repeat = Repeat.off,
    this.continuation = const Continuation(ContinuationKind.none, false),
    this.waveActive = false,
    this.settings = const PlayerSettings(),
  });

  /// The playing entry: the state's uid, else the queue index.
  PlayerTrack? get current {
    final uid = state.uid;
    if (uid != null) {
      for (final t in items) {
        if (t.uid == uid) return t;
      }
    }
    return index >= 0 && index < items.length ? items[index] : null;
  }

  int get currentIndex {
    final uid = state.uid;
    if (uid != null) {
      final i = items.indexWhere((t) => t.uid == uid);
      if (i >= 0) return i;
    }
    return index;
  }

  bool get playing => state.status == PlayStatus.playing;

  /// Loading or buffering: show a spinner.
  bool get busy => state.status == PlayStatus.loading || state.buffering;

  List<PlayerTrack> get upNext {
    final i = currentIndex;
    return i < 0 ? items : items.sublist(i + 1);
  }

  PlayerView copyWith({
    bool? ready,
    PlaybackState? state,
    List<PlayerTrack>? items,
    int? index,
    int? rev,
    bool? shuffle,
    Repeat? repeat,
    Continuation? continuation,
    bool? waveActive,
    PlayerSettings? settings,
  }) =>
      PlayerView(
        ready: ready ?? this.ready,
        state: state ?? this.state,
        items: items ?? this.items,
        index: index ?? this.index,
        rev: rev ?? this.rev,
        shuffle: shuffle ?? this.shuffle,
        repeat: repeat ?? this.repeat,
        continuation: continuation ?? this.continuation,
        waveActive: waveActive ?? this.waveActive,
        settings: settings ?? this.settings,
      );
}

/// One position report. [at] is the Unix ms it was taken.
class PositionSample {
  final String? uid;
  final double position;
  final double? duration;
  final double rate;
  final double at;
  final bool playing;

  const PositionSample({
    this.uid,
    this.position = 0,
    this.duration,
    this.rate = 1,
    this.at = 0,
    this.playing = false,
  });

  /// Seconds at wall time [nowMs], extrapolated while playing.
  double positionAt(int nowMs) {
    var p = position;
    if (playing && at > 0) {
      final dt = (nowMs - at) / 1000.0;
      // A stale sample (suspended machine, lost ticks) must not run away.
      if (dt > 0) p += (dt > 2 ? 2 : dt) * rate;
    }
    final d = duration;
    if (d != null && d > 0 && p > d) p = d;
    return p < 0 ? 0 : p;
  }

  factory PositionSample.fromState(PlaybackState s) => PositionSample(
        uid: s.uid,
        position: s.position,
        duration: s.duration,
        rate: s.rate,
        at: s.at,
        playing: s.status == PlayStatus.playing,
      );

  factory PositionSample.fromEvent(PositionEvent e) => PositionSample(
        uid: e.uid,
        position: e.position,
        duration: e.duration,
        rate: e.rate,
        at: e.at,
        playing: e.playing,
      );
}

final positionProvider =
    NotifierProvider<PositionController, PositionSample>(PositionController.new);

class PositionController extends Notifier<PositionSample> {
  @override
  PositionSample build() => const PositionSample();

  void set(PositionSample s) => state = s;
}

final playerMessagesProvider = Provider<StreamController<String>>((ref) {
  final c = StreamController<String>.broadcast();
  ref.onDispose(c.close);
  return c;
});

final playerViewProvider =
    NotifierProvider<PlayerController, PlayerView>(PlayerController.new);

/// Russian text for a track failure.
String describePlayerError(PlayerError e) {
  final what = switch (e.kind) {
    'bot_check' => 'YouTube просит подтвердить, что вы не робот. Проверьте cookies в настройках сети',
    'unavailable' => 'Трек недоступен',
    'no_yt_dlp' => 'Не найден yt-dlp',
    'decode' => 'Не удалось декодировать поток',
    'timeout' => 'Сеть не ответила вовремя',
    _ => 'Ошибка воспроизведения',
  };
  final then = switch (e.action) {
    'skipped' => ', трек пропущен',
    'retrying' => ', пробую ещё раз',
    'stopped' => ', воспроизведение остановлено',
    _ => '',
  };
  return '$what$then';
}

class PlayerController extends Notifier<PlayerView> {
  PlayerApi get _api => ref.read(playerProvider);
  bool _syncing = false;
  bool _resyncAgain = false;

  @override
  PlayerView build() {
    final api = ref.watch(playerProvider);
    final t = ref.watch(transportProvider);
    final subs = <StreamSubscription<Object?>>[
      api.events.listen(_onEvent),
      t.statusChanges.listen((s) {
        if (s == TransportStatus.connected) unawaited(resync());
      }),
    ];
    ref.onDispose(() {
      for (final s in subs) {
        s.cancel();
      }
    });
    if (t.status == TransportStatus.connected) {
      Future.microtask(resync);
    }
    return const PlayerView();
  }

  /// Replaces the mirror with a fresh `player_snapshot`.
  Future<void> resync() async {
    if (_syncing) {
      _resyncAgain = true;
      return;
    }
    _syncing = true;
    try {
      do {
        _resyncAgain = false;
        final s = await _api.snapshot();
        if (!ref.mounted) return;
        state = PlayerView(
          ready: true,
          state: s.state,
          items: s.queue.items,
          index: s.queue.index,
          rev: s.queue.rev,
          shuffle: s.queue.shuffle,
          repeat: s.queue.repeat,
          continuation: s.queue.continuation,
          waveActive: s.waveActive,
          settings: s.settings,
        );
        ref.read(positionProvider.notifier).set(PositionSample.fromState(s.state));
      } while (_resyncAgain);
    } on DaemonException {
      // Disconnected or the engine is down: the next `connected` retries.
    } finally {
      _syncing = false;
    }
  }

  void _onEvent(PlayerEvent e) {
    switch (e) {
      case StateEvent(:final state):
        this.state = this.state.copyWith(state: state);
        ref.read(positionProvider.notifier).set(PositionSample.fromState(state));
      case PositionChanged(:final position):
        ref.read(positionProvider.notifier).set(PositionSample.fromEvent(position));
      case QueueEvent(:final queue):
        _applyQueue(queue);
      case ErrorEvent(:final error):
        ref.read(playerMessagesProvider).add(describePlayerError(error));
      case SettingsEvent(:final settings):
        state = state.copyWith(settings: settings);
      case WaveEvent(:final active):
        state = state.copyWith(waveActive: active);
      case SleepEvent() || QueueLowEvent() || JournalEvent():
        break;
    }
  }

  void _applyQueue(QueueChanged q) {
    final v = state;
    if (v.ready && q.rev <= v.rev) return; // Stale or duplicate.
    final known = {for (final t in v.items) t.uid: t};
    for (final t in q.added) {
      known[t.uid] = t;
    }
    for (final t in q.updated) {
      known[t.uid] = t;
    }
    final items = <PlayerTrack>[];
    var missing = false;
    for (final uid in q.order) {
      final t = known[uid];
      if (t == null) {
        missing = true;
        break;
      }
      items.add(t);
    }
    if (missing || !v.ready) {
      unawaited(resync());
      return;
    }
    state = v.copyWith(
      items: items,
      index: q.index,
      rev: q.rev,
      shuffle: q.shuffle,
      repeat: q.repeat,
      continuation: q.continuation,
    );
    // A gap means an earlier `updated` list may be lost: refetch quietly.
    if (q.rev != v.rev + 1) unawaited(resync());
  }

  // ---- commands --------------------------------------------------------------

  Future<T?> _run<T>(Future<T> Function(PlayerApi api) f) async {
    try {
      return await f(_api);
    } on DaemonException catch (e) {
      ref.read(playerMessagesProvider).add(e.code == DaemonException.unavailable
          ? 'Нет связи с плеером'
          : 'Не получилось: ${e.message}');
      return null;
    }
  }

  Future<void> toggle() => _run((a) => a.toggle());
  Future<void> play() => _run((a) => a.play());
  Future<void> pause() => _run((a) => a.pause());
  Future<void> next() => _run((a) => a.next());
  Future<void> prev() => _run((a) => a.prev());

  Future<void> seek(double seconds) async {
    final p = ref.read(positionProvider);
    final d = p.duration ?? state.state.duration;
    var s = seconds < 0 ? 0.0 : seconds;
    if (d != null && d > 0 && s > d - 0.5) s = d - 0.5;
    // Optimistic: the bar jumps now, the daemon's tick confirms.
    ref.read(positionProvider.notifier).set(PositionSample(
          uid: p.uid,
          position: s,
          duration: p.duration,
          rate: p.rate,
          at: ref.read(clockProvider)().toDouble(),
          playing: p.playing,
        ));
    await _run((a) => a.seek(s));
  }

  Future<void> seekBy(double delta) =>
      seek(ref.read(positionProvider).positionAt(ref.read(clockProvider)()) + delta);

  Future<void> playUid(String uid) => _run((a) => a.playUid(uid));

  Future<void> setShuffle(bool on) => _run((a) => a.setShuffle(on));

  Future<void> cycleRepeat() => _run((a) => a.setRepeat(switch (state.repeat) {
        Repeat.off => Repeat.all,
        Repeat.all => Repeat.one,
        Repeat.one => Repeat.off,
      }));

  Future<void> setVolume(double v) async {
    final vol = v.clamp(0.0, 1.0);
    state = state.copyWith(
      state: _withVolume(state.state, vol),
    );
    final s = await _run((a) => a.setSettings({'volume': vol, if (vol > 0) 'muted': false}));
    if (s != null && ref.mounted) state = state.copyWith(settings: s);
  }

  Future<void> toggleMute() async {
    final muted = !state.state.muted;
    final s = await _run((a) => a.setSettings({'muted': muted}));
    if (s != null && ref.mounted) state = state.copyWith(settings: s);
  }

  /// Moves [uid] to absolute queue position [to], optimistically.
  Future<void> move(String uid, int to) async {
    final items = [...state.items];
    final from = items.indexWhere((t) => t.uid == uid);
    if (from < 0) return;
    final t = items.removeAt(from);
    items.insert(to.clamp(0, items.length), t);
    state = state.copyWith(items: items);
    await _run((a) => a.move(uid, to));
  }

  Future<void> remove(String uid) async {
    state = state.copyWith(items: [
      for (final t in state.items)
        if (t.uid != uid) t
    ]);
    await _run((a) => a.remove(uid));
  }

  Future<void> waveStart() => _run((a) => a.waveStart());
  Future<void> waveStop() => _run((a) => a.waveStop());
  Future<void> waveDislike() => _run((a) => a.waveDislike());

  /// Replaces the queue and plays from [startIndex].
  Future<void> playTracks(List<TrackInput> tracks, {int startIndex = 0}) =>
      _run((a) => a.setQueue(tracks, startIndex: startIndex, play: true));
}

PlaybackState _withVolume(PlaybackState s, double v) => PlaybackState(
      status: s.status,
      uid: s.uid,
      trackId: s.trackId,
      position: s.position,
      at: s.at,
      duration: s.duration,
      rate: s.rate,
      buffering: s.buffering,
      crossfading: s.crossfading,
      volume: v,
      muted: v > 0 ? false : s.muted,
      stream: s.stream,
      error: s.error,
    );
