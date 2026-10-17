library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/models.dart';
import '../../state/library.dart';
import '../widgets/cards.dart';
import '../widgets/collection_header.dart';
import '../widgets/cover.dart';
import '../widgets/format.dart';
import '../widgets/states.dart';
import '../widgets/track_actions.dart';
import '../widgets/track_row.dart';
import 'providers.dart';

class PlaylistScreen extends ConsumerWidget {
  final String id;
  const PlaylistScreen({super.key, required this.id});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final p = ref.watch(playlistPagesProvider(id));
    return Scaffold(
      backgroundColor: Colors.transparent,
      body: AsyncBody(
        value: p,
        onRetry: () => ref.invalidate(playlistPagesProvider(id)),
        data: (s) => _PlaylistView(id: id, pages: s),
      ),
    );
  }
}

class _PlaylistView extends ConsumerStatefulWidget {
  final String id;
  final PlaylistPages pages;
  const _PlaylistView({required this.id, required this.pages});

  @override
  ConsumerState<_PlaylistView> createState() => _PlaylistViewState();
}

class _PlaylistViewState extends ConsumerState<_PlaylistView> {
  final _scroll = ScrollController();

  @override
  void initState() {
    super.initState();
    _scroll.addListener(_onScroll);
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  PlaylistPager get _pager => ref.read(playlistPagesProvider(widget.id).notifier);

  void _onScroll() {
    final p = _scroll.position;
    if (p.pixels > p.maxScrollExtent - 800) _pager.loadMore();
  }

  /// Every track, loading the remaining pages first.
  Future<List<Track>> _all() => _pager.loadAll(cap: 1000);

  @override
  Widget build(BuildContext context) {
    final s = widget.pages;
    final p = s.playlist;
    final width = MediaQuery.sizeOf(context).width;
    final side = Gap.side(width);
    final summary = PlaylistItem(p.toSummary());
    final cid = canonicalId(LikeKind.playlist, p.id);
    final liked = ref.watch(isLikedProvider((LikeKind.playlist, cid)));
    final pinned = ref.watch(isPinnedProvider((PinKind.playlist, cid)));
    final actions = TrackActions(context, ref);
    final count = p.trackCount ?? s.tracks.length;
    // A first page shorter than the window never scrolls: fetch on.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted && s.hasMore && _scroll.hasClients && _scroll.position.maxScrollExtent < 400) _pager.loadMore();
    });
    final meta = [
      if (p.author != null && p.author!.name.isNotEmpty) p.author!.name,
      if (p.year != null) '${p.year}',
      if (count > 0) tracksCount(count),
    ].join(' · ');

    return CustomScrollView(controller: _scroll, slivers: [
      SliverToBoxAdapter(
        child: CollectionHeader(
          cover: (sz) => CoverImage.thumbs(p.thumbnails, size: sz, radius: 12, icon: Icons.queue_music_rounded, seed: p.id),
          kind: 'Плейлист',
          title: p.title,
          meta: meta.isEmpty ? null : Text(meta),
          description: p.description,
          actions: ActionBar(children: [
            FilledButton.icon(
              onPressed: s.tracks.isEmpty ? null : () async => actions.playAll(await _all()),
              icon: const Icon(Icons.play_arrow_rounded),
              label: const Text('Слушать'),
              style: FilledButton.styleFrom(minimumSize: const Size(0, 44), padding: const EdgeInsets.symmetric(horizontal: 22)),
            ),
            ShuffleButton(onPressed: s.tracks.isEmpty ? null : () async => actions.shuffle(await _all())),
            LikeButton(
              liked: liked,
              size: 24,
              onPressed: () => guarded(context, () => ref.read(likesProvider.notifier).toggle(summary)),
            ),
            PopupMenuButton<VoidCallback>(
              tooltip: 'Ещё',
              icon: const Icon(Icons.more_horiz_rounded),
              onSelected: (f) => f(),
              itemBuilder: (_) => [
                PopupMenuItem(value: () async => actions.addToQueue(await _all()), child: const Text('Добавить в очередь')),
                PopupMenuItem(value: () async => actions.addToPlaylist(await _all()), child: const Text('Добавить в плейлист…')),
                PopupMenuItem(
                  value: () => guarded(context, () => ref.read(libraryApiProvider).pinItem(summary, on: !pinned)),
                  child: Text(pinned ? 'Открепить' : 'Закрепить в библиотеке'),
                ),
              ],
            ),
          ]),
        ),
      ),
      if (s.tracks.isEmpty)
        const SliverToBoxAdapter(child: EmptyView(icon: Icons.queue_music_rounded, title: 'Плейлист пуст'))
      else
        TrackListSliver(tracks: s.tracks, padding: EdgeInsets.symmetric(horizontal: side - 8)),
      if (s.loadingMore)
        const SliverToBoxAdapter(
          child: Padding(padding: EdgeInsets.all(24), child: Center(child: CircularProgressIndicator())),
        ),
      if (s.moreError != null)
        SliverToBoxAdapter(child: ErrorView(error: s.moreError!, compact: true, onRetry: _pager.loadMore)),
      const SliverToBoxAdapter(child: SizedBox(height: 40)),
    ]);
  }
}
