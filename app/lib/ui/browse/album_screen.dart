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

/// Album tracks often come without covers and album links; fill them from the album.
List<Track> albumTracks(Album a) => [
      for (final t in a.tracks)
        Track(
          videoId: t.videoId,
          title: t.title,
          artists: t.artists.isEmpty ? a.artists : t.artists,
          album: t.album ?? AlbumRef(a.browseId, a.title),
          durationS: t.durationS,
          thumbnails: t.thumbnails.isEmpty ? a.thumbnails : t.thumbnails,
          explicit: t.explicit,
          kind: t.kind,
          year: t.year ?? a.year,
          views: t.views,
          playable: t.playable,
        ),
    ];

class AlbumScreen extends ConsumerWidget {
  final String id;
  const AlbumScreen({super.key, required this.id});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final album = ref.watch(albumProvider(id));
    return Scaffold(
      backgroundColor: Colors.transparent,
      body: AsyncBody(
        value: album,
        onRetry: () => ref.invalidate(albumProvider(id)),
        data: (a) => _AlbumView(album: a),
      ),
    );
  }
}

class _AlbumView extends ConsumerWidget {
  final Album album;
  const _AlbumView({required this.album});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final a = album;
    final tracks = albumTracks(a);
    final summary = AlbumItem(a.toSummary());
    final liked = ref.watch(isLikedProvider((LikeKind.album, a.browseId)));
    final pinned = ref.watch(isPinnedProvider((PinKind.album, a.browseId)));
    final width = MediaQuery.sizeOf(context).width;
    final side = Gap.side(width);
    final total = a.durationS > 0 ? a.durationS : tracks.fold<int>(0, (s, t) => s + (t.durationS ?? 0));
    final meta = [
      if (a.year != null) '${a.year}',
      tracksCount(a.trackCount ?? tracks.length),
      if (total > 0) formatLongDuration(total),
    ].join(' · ');
    final actions = TrackActions(context, ref);

    return CustomScrollView(slivers: [
      SliverToBoxAdapter(
        child: CollectionHeader(
          cover: (s) => CoverImage.thumbs(a.thumbnails, size: s, radius: 12, icon: Icons.album_rounded, seed: a.browseId),
          kind: albumKindLabel(a.kind),
          title: a.title,
          meta: Column(
            crossAxisAlignment: width < 640 ? CrossAxisAlignment.center : CrossAxisAlignment.start,
            children: [
              ArtistLinks(
                artists: a.artists,
                style: Theme.of(context).textTheme.titleSmall?.copyWith(fontWeight: FontWeight.w600),
              ),
              const SizedBox(height: 4),
              Text(meta),
            ],
          ),
          description: a.description,
          actions: PlayShuffleButtons(tracks: tracks, extra: [
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
                PopupMenuItem(value: () => actions.playNext(tracks), child: const Text('Играть следующим')),
                PopupMenuItem(value: () => actions.addToQueue(tracks), child: const Text('Добавить в очередь')),
                PopupMenuItem(value: () => actions.addToPlaylist(tracks), child: const Text('Добавить в плейлист…')),
                PopupMenuItem(
                  value: () => guarded(context, () => ref.read(libraryApiProvider).pinItem(summary, on: !pinned)),
                  child: Text(pinned ? 'Открепить' : 'Закрепить в библиотеке'),
                ),
              ],
            ),
          ]),
        ),
      ),
      if (tracks.isEmpty)
        const SliverToBoxAdapter(child: EmptyView(icon: Icons.album_rounded, title: 'В альбоме нет треков'))
      else
        TrackListSliver(
          tracks: tracks,
          numbered: true,
          showCover: false,
          showAlbum: false,
          padding: EdgeInsets.symmetric(horizontal: side - 8),
        ),
      if (a.related.isNotEmpty)
        SliverToBoxAdapter(
          child: Padding(
            padding: const EdgeInsets.only(top: Gap.section),
            child: Shelf(
              title: 'Ещё от исполнителя',
              padding: EdgeInsets.symmetric(horizontal: side),
              children: [for (final r in a.related) ItemCard(item: AlbumItem(r), width: 160)],
            ),
          ),
        ),
      const SliverToBoxAdapter(child: SizedBox(height: 40)),
    ]);
  }
}
