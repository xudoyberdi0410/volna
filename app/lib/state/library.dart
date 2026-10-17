/// Library state: liked ids for the like buttons, and the library lists,
/// all refreshed on the daemon's `library_changed` events.
library;

import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/library.dart';
import '../api/models.dart';
import 'connection.dart';

export '../api/library.dart';

final libraryApiProvider = Provider((ref) => LibraryApi(ref.watch(transportProvider)));

/// Daemon errors are not worth retrying (a `not_found` stays one).
Duration? noRetry(int count, Object error) => null;

/// Re-runs the provider when the daemon reports a change of a kind in [kinds]
/// (`all` always matches).
void refreshOnLibraryChange(Ref ref, Set<String> kinds) {
  final sub = ref
      .watch(libraryApiProvider)
      .changes
      .where((k) => k == 'all' || kinds.contains(k))
      .listen((_) => ref.invalidateSelf());
  ref.onDispose(sub.cancel);
}

/// Ids of everything liked, per kind. Loaded once, then kept in sync with
/// `library_changed`; [Likes.toggle] updates optimistically.
class LikedIds {
  final Map<LikeKind, Set<String>> _ids;
  final bool loaded;
  const LikedIds(this._ids, {this.loaded = false});

  bool contains(LikeKind kind, String id) =>
      _ids[kind]?.contains(canonicalId(kind, id)) ?? false;

  LikedIds withKind(LikeKind kind, Set<String> ids) =>
      LikedIds({..._ids, kind: ids}, loaded: loaded);

  LikedIds withLoaded() => LikedIds(_ids, loaded: true);

  Set<String> of(LikeKind kind) => _ids[kind] ?? const {};
}

class Likes extends Notifier<LikedIds> {
  /// Writes in flight per id: a reload must not undo an optimistic toggle.
  final _pending = <String>{};

  @override
  LikedIds build() {
    final api = ref.watch(libraryApiProvider);
    final sub = api.changes.listen((k) {
      if (k == 'all') {
        unawaited(_loadAll());
      } else {
        final kind = LikeKind.values.where((v) => v.name == k).firstOrNull;
        if (kind != null) unawaited(_load(kind));
      }
    });
    ref.onDispose(sub.cancel);
    unawaited(_loadAll());
    return const LikedIds({});
  }

  Future<void> _loadAll() async {
    await Future.wait([for (final k in LikeKind.values) _load(k)]);
    if (ref.mounted) state = state.withLoaded();
  }

  Future<void> _load(LikeKind kind) async {
    try {
      final items = await ref.read(libraryApiProvider).liked(kind);
      if (!ref.mounted) return;
      final ids = {for (final i in items) canonicalId(kind, itemId(i.item))};
      // Keep the optimistic value of ids being written right now.
      final cur = state.of(kind);
      for (final p in _pending) {
        final id = p.substring(p.indexOf(':') + 1);
        if (!p.startsWith('${kind.name}:')) continue;
        if (cur.contains(id)) {
          ids.add(id);
        } else {
          ids.remove(id);
        }
      }
      state = state.withKind(kind, ids);
    } catch (_) {
      // Daemon not there yet: the next `library_changed` or reconnect reloads.
    }
  }

  bool isLiked(Item item) => state.contains(LikeKind.of(item), itemId(item));

  /// Flips the like of [item]; returns the new value. Reverts on failure and rethrows.
  Future<bool> toggle(Item item) => set(item, !isLiked(item));

  Future<bool> set(Item item, bool on) async {
    final kind = LikeKind.of(item);
    final id = canonicalId(kind, itemId(item));
    final key = '${kind.name}:$id';
    final before = state.of(kind);
    state = state.withKind(kind, on ? ({...before, id}) : ({...before}..remove(id)));
    _pending.add(key);
    try {
      return await ref.read(libraryApiProvider).like(item, on: on);
    } catch (_) {
      if (ref.mounted) {
        final now = state.of(kind);
        state = state.withKind(kind, on ? ({...now}..remove(id)) : ({...now, id}));
      }
      rethrow;
    } finally {
      _pending.remove(key);
    }
  }

  /// Reloads everything (after a reconnect).
  Future<void> reload() => _loadAll();
}

final likesProvider = NotifierProvider<Likes, LikedIds>(Likes.new);

/// Whether one item is liked; rebuilds only when that changes.
final isLikedProvider = Provider.family<bool, (LikeKind, String)>(
  (ref, k) => ref.watch(likesProvider.select((s) => s.contains(k.$1, k.$2))),
);

/// Liked items of a kind, newest first.
final likedItemsProvider = FutureProvider.autoDispose.family<List<LikedItem>, LikeKind>(
  (ref, kind) {
    refreshOnLibraryChange(ref, {kind.name});
    return ref.watch(libraryApiProvider).liked(kind);
  },
  retry: noRetry,
);

final userPlaylistsProvider = FutureProvider.autoDispose<List<UserPlaylistInfo>>(
  (ref) {
    refreshOnLibraryChange(ref, {'user_playlist'});
    return ref.watch(libraryApiProvider).playlists();
  },
  retry: noRetry,
);

final userPlaylistProvider = FutureProvider.autoDispose.family<UserPlaylist, String>(
  (ref, id) {
    refreshOnLibraryChange(ref, {'user_playlist'});
    return ref.watch(libraryApiProvider).playlist(id);
  },
  retry: noRetry,
);

final historyProvider = FutureProvider.autoDispose<List<HistoryItem>>(
  (ref) {
    refreshOnLibraryChange(ref, {'history'});
    return ref.watch(libraryApiProvider).history(limit: 500);
  },
  retry: noRetry,
);

final pinsProvider = FutureProvider.autoDispose<List<Pin>>(
  (ref) {
    refreshOnLibraryChange(ref, {'pinned', 'user_playlist'});
    return ref.watch(libraryApiProvider).pins();
  },
  retry: noRetry,
);

/// Whether an album/artist/playlist is pinned.
final isPinnedProvider = Provider.autoDispose.family<bool, (PinKind, String)>((ref, k) {
  final pins = ref.watch(pinsProvider).value ?? const [];
  return pins.any((p) => p.kind == k.$1 && p.id == k.$2);
});

/// Liked state for a [Track] (convenience for rows).
bool watchTrackLiked(WidgetRef ref, Track t) =>
    ref.watch(isLikedProvider((LikeKind.track, t.videoId)));
