/// Catalog state of the browse screens.
library;

import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/catalog.dart';
import '../../api/models.dart';
import '../../state/connection.dart';
import '../../state/library.dart' show noRetry;

/// Pages stay cached for a while after leaving them (back navigation is instant).
void _keepFor(Ref ref, Duration d) {
  final link = ref.keepAlive();
  Timer? timer;
  ref.onCancel(() => timer = Timer(d, link.close));
  ref.onResume(() => timer?.cancel());
  ref.onDispose(() => timer?.cancel());
}

final albumProvider = FutureProvider.autoDispose.family<Album, String>((ref, id) {
  _keepFor(ref, const Duration(minutes: 5));
  return ref.watch(catalogProvider).album(id);
}, retry: noRetry);

final artistProvider = FutureProvider.autoDispose.family<Artist, String>((ref, id) {
  _keepFor(ref, const Duration(minutes: 5));
  return ref.watch(catalogProvider).artist(id);
}, retry: noRetry);

final homeProvider = FutureProvider.autoDispose<List<Section>>((ref) {
  _keepFor(ref, const Duration(minutes: 30));
  return ref.watch(catalogProvider).home();
}, retry: noRetry);

final moodsProvider = FutureProvider.autoDispose<List<MoodCategory>>((ref) {
  _keepFor(ref, const Duration(minutes: 30));
  return ref.watch(catalogProvider).moods();
}, retry: noRetry);

final moodProvider = FutureProvider.autoDispose.family<List<Section>, String>((ref, params) {
  _keepFor(ref, const Duration(minutes: 10));
  return ref.watch(catalogProvider).mood(params);
}, retry: noRetry);

/// A catalog playlist with its tracks loaded page by page.
class PlaylistPages {
  final Playlist playlist;
  final List<Track> tracks;
  final String? continuation;
  final bool loadingMore;
  final Object? moreError;
  const PlaylistPages(this.playlist, this.tracks, this.continuation,
      {this.loadingMore = false, this.moreError});

  bool get hasMore => continuation != null;

  PlaylistPages copyWith({
    List<Track>? tracks,
    String? Function()? continuation,
    bool? loadingMore,
    Object? Function()? moreError,
  }) =>
      PlaylistPages(
        playlist,
        tracks ?? this.tracks,
        continuation != null ? continuation() : this.continuation,
        loadingMore: loadingMore ?? this.loadingMore,
        moreError: moreError != null ? moreError() : this.moreError,
      );
}

class PlaylistPager extends AsyncNotifier<PlaylistPages> {
  final String id;
  PlaylistPager(this.id);

  @override
  Future<PlaylistPages> build() async {
    _keepFor(ref, const Duration(minutes: 5));
    final p = await ref.watch(catalogProvider).playlist(id);
    return PlaylistPages(p, p.tracks, p.continuation);
  }

  /// Fetches the next page (`catalog_playlist_more`); no-op while loading or at the end.
  Future<void> loadMore() async {
    final s = state.value;
    if (s == null || s.loadingMore || !s.hasMore) return;
    state = AsyncData(s.copyWith(loadingMore: true, moreError: () => null));
    try {
      final page = await ref.read(catalogProvider).playlistMore(s.continuation!);
      if (!ref.mounted) return;
      final cur = state.value!;
      state = AsyncData(cur.copyWith(
        tracks: [...cur.tracks, ...page.tracks],
        // An empty page with a token would loop forever.
        continuation: () => page.tracks.isEmpty ? null : page.continuation,
        loadingMore: false,
      ));
    } catch (e) {
      if (!ref.mounted) return;
      state = AsyncData(state.value!.copyWith(loadingMore: false, moreError: () => e));
    }
  }

  /// Loads every remaining page (before "play all" of a long playlist), up to [cap] tracks.
  Future<List<Track>> loadAll({int cap = 1000}) async {
    while (ref.mounted) {
      final s = state.value;
      if (s == null || !s.hasMore || s.tracks.length >= cap || s.moreError != null) break;
      await loadMore();
    }
    return state.value?.tracks ?? const [];
  }
}

final playlistPagesProvider =
    AsyncNotifierProvider.autoDispose.family<PlaylistPager, PlaylistPages, String>(PlaylistPager.new,
        retry: noRetry);

// ---- search ------------------------------------------------------------------

class SearchState {
  final String query;
  final SearchFilter filter;
  final Item? topResult;
  final List<Item> items;
  final String? continuation;
  final bool loading;
  final bool loadingMore;
  final Object? error;

  const SearchState({
    this.query = '',
    this.filter = SearchFilter.all,
    this.topResult,
    this.items = const [],
    this.continuation,
    this.loading = false,
    this.loadingMore = false,
    this.error,
  });

  bool get hasMore => continuation != null && filter != SearchFilter.all;
  bool get isEmpty => topResult == null && items.isEmpty;
}

/// The search screen's state: kept while navigating to results and back.
class SearchController extends Notifier<SearchState> {
  int _gen = 0;

  @override
  SearchState build() => const SearchState();

  CatalogApi get _api => ref.read(catalogProvider);

  Future<void> search(String query, {SearchFilter? filter}) async {
    final q = query.trim();
    final f = filter ?? state.filter;
    final gen = ++_gen;
    if (q.isEmpty) {
      state = SearchState(filter: f);
      return;
    }
    state = SearchState(query: q, filter: f, loading: true);
    try {
      final page = await _api.search(q, filter: f);
      if (gen != _gen) return;
      state = SearchState(
        query: q,
        filter: f,
        topResult: page.topResult,
        items: page.items,
        continuation: page.continuation,
      );
    } catch (e) {
      if (gen != _gen) return;
      state = SearchState(query: q, filter: f, error: e);
    }
  }

  Future<void> setFilter(SearchFilter f) => f == state.filter ? Future.value() : search(state.query, filter: f);

  Future<void> retry() => search(state.query);

  /// Next page with the same filter (`catalog_search_more`).
  Future<void> loadMore() async {
    final s = state;
    if (s.loading || s.loadingMore || !s.hasMore) return;
    final gen = _gen;
    state = SearchState(
      query: s.query,
      filter: s.filter,
      topResult: s.topResult,
      items: s.items,
      continuation: s.continuation,
      loadingMore: true,
    );
    try {
      final page = await _api.searchMore(s.continuation!, filter: s.filter);
      if (gen != _gen) return;
      state = SearchState(
        query: s.query,
        filter: s.filter,
        topResult: s.topResult,
        items: [...s.items, ...page.items],
        continuation: page.items.isEmpty ? null : page.continuation,
      );
    } catch (e) {
      if (gen != _gen) return;
      // Keep what we have; drop the token so the list stops asking.
      state = SearchState(
        query: s.query,
        filter: s.filter,
        topResult: s.topResult,
        items: s.items,
        error: s.items.isEmpty ? e : null,
      );
    }
  }
}

final searchProvider = NotifierProvider<SearchController, SearchState>(SearchController.new);

/// Suggestions for the text being typed; empty for an empty input.
final suggestionsProvider = FutureProvider.autoDispose.family<SearchSuggestions, String>((ref, input) async {
  if (input.trim().isEmpty) return const SearchSuggestions([], []);
  // Debounce: a newer input disposes this one before the timer fires.
  var cancelled = false;
  ref.onDispose(() => cancelled = true);
  await Future<void>.delayed(const Duration(milliseconds: 180));
  if (cancelled) throw StateError('cancelled');
  return ref.watch(catalogProvider).suggestions(input);
}, retry: noRetry);
