library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/models.dart';
import '../nav.dart';
import '../../state/library.dart';
import '../widgets/cards.dart';
import '../widgets/collection_header.dart';
import '../widgets/cover.dart';
import '../widgets/states.dart';
import '../widgets/track_actions.dart';
import '../widgets/track_row.dart';
import 'providers.dart';

class ArtistScreen extends ConsumerWidget {
  final String id;
  const ArtistScreen({super.key, required this.id});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final artist = ref.watch(artistProvider(id));
    return Scaffold(
      backgroundColor: Colors.transparent,
      body: AsyncBody(
        value: artist,
        loading: const PageSkeleton(round: true),
        onRetry: () => ref.invalidate(artistProvider(id)),
        data: (a) => _ArtistView(artist: a),
      ),
    );
  }
}

class _ArtistView extends ConsumerStatefulWidget {
  final Artist artist;
  const _ArtistView({required this.artist});

  @override
  ConsumerState<_ArtistView> createState() => _ArtistViewState();
}

class _ArtistViewState extends ConsumerState<_ArtistView> {
  bool _allTop = false;

  @override
  Widget build(BuildContext context) {
    final a = widget.artist;
    final width = MediaQuery.sizeOf(context).width;
    final side = Gap.side(width);
    final summary = ArtistItem(a.toSummary());
    final liked = ref.watch(isLikedProvider((LikeKind.artist, a.id)));
    final pinned = ref.watch(isPinnedProvider((PinKind.artist, canonicalId(LikeKind.artist, a.id))));
    final top = a.topSongs;
    final shown = _allTop ? top : top.take(5).toList();

    Widget shelf(String title, List<Item> items) => SliverToBoxAdapter(
          child: Padding(
            padding: const EdgeInsets.only(top: Gap.section),
            child: Shelf(
              title: title,
              padding: EdgeInsets.symmetric(horizontal: side),
              children: [for (final i in items) ItemCard(item: i, width: 160)],
            ),
          ),
        );

    return CustomScrollView(slivers: [
      SliverToBoxAdapter(
        child: CollectionHeader(
          cover: (s) => CoverImage.thumbs(a.thumbnails,
              size: s, shape: CoverShape.circle, icon: Icons.person_rounded, seed: a.id),
          round: true,
          kind: 'Исполнитель',
          title: a.name,
          meta: a.subscribers == null ? null : Text(a.subscribers!),
          actions: PlayShuffleButtons(tracks: top, extra: [
            OutlinedButton.icon(
              onPressed: () => guarded(context, () => ref.read(likesProvider.notifier).toggle(summary)),
              icon: Icon(liked ? Icons.check_rounded : Icons.add_rounded),
              label: Text(liked ? 'В библиотеке' : 'В библиотеку'),
              style: OutlinedButton.styleFrom(minimumSize: const Size(0, 44)),
            ),
            PopupMenuButton<VoidCallback>(
              tooltip: 'Ещё',
              icon: const Icon(Icons.more_horiz_rounded),
              onSelected: (f) => f(),
              itemBuilder: (_) => [
                PopupMenuItem(
                  value: () => guarded(context, () => ref.read(libraryApiProvider).pinItem(summary, on: !pinned)),
                  child: Text(pinned ? 'Открепить' : 'Закрепить в библиотеке'),
                ),
              ],
            ),
          ]),
        ),
      ),
      if (top.isNotEmpty) ...[
        SliverToBoxAdapter(
          child: SectionHeader(
            title: 'Популярные треки',
            padding: EdgeInsets.fromLTRB(side, 8, side - 8, 8),
            trailing: a.topSongsPlaylistId == null
                ? null
                : TextButton(
                    onPressed: () => openPlaylist(context, a.topSongsPlaylistId!),
                    child: const Text('Все треки'),
                  ),
          ),
        ),
        TrackListSliver(tracks: shown, padding: EdgeInsets.symmetric(horizontal: side - 8)),
        if (top.length > 5)
          SliverToBoxAdapter(
            child: Padding(
              padding: EdgeInsets.fromLTRB(side, 4, side, 0),
              child: Align(
                alignment: Alignment.centerLeft,
                child: TextButton(
                  onPressed: () => setState(() => _allTop = !_allTop),
                  child: Text(_allTop ? 'Свернуть' : 'Показать ещё'),
                ),
              ),
            ),
          ),
      ],
      if (a.albums.isNotEmpty) shelf('Альбомы', [for (final x in a.albums) AlbumItem(x)]),
      if (a.singles.isNotEmpty) shelf('Синглы и EP', [for (final x in a.singles) AlbumItem(x)]),
      if (a.videos.isNotEmpty) shelf('Видео', [for (final x in a.videos) TrackItem(x)]),
      if (a.playlists.isNotEmpty) shelf('Плейлисты', [for (final x in a.playlists) PlaylistItem(x)]),
      if (a.related.isNotEmpty) shelf('Похожие исполнители', [for (final x in a.related) ArtistItem(x)]),
      if (a.description != null && a.description!.trim().isNotEmpty)
        SliverToBoxAdapter(
          child: Padding(
            padding: EdgeInsets.fromLTRB(side, Gap.section, side, 0),
            child: _About(text: a.description!.trim()),
          ),
        ),
      const SliverToBoxAdapter(child: SizedBox(height: 40)),
    ]);
  }
}

class _About extends StatelessWidget {
  final String text;
  const _About({required this.text});

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    return Container(
      constraints: const BoxConstraints(maxWidth: 820),
      padding: const EdgeInsets.all(20),
      decoration: BoxDecoration(color: cs.surfaceContainer, borderRadius: BorderRadius.circular(16)),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text('Об исполнителе', style: t.titleMedium?.copyWith(fontWeight: FontWeight.w700)),
        const SizedBox(height: 8),
        Text(text, style: t.bodyMedium?.copyWith(color: cs.onSurfaceVariant, height: 1.45), maxLines: 8, overflow: TextOverflow.ellipsis),
      ]),
    );
  }
}
