/// The library: pinned items on top, then tabs of liked tracks, albums,
/// artists, playlists and the user's own playlists.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/models.dart';
import '../nav.dart';
import '../../state/library.dart';
import '../widgets/cards.dart';
import '../widgets/cover.dart';
import '../widgets/format.dart';
import '../widgets/states.dart';
import '../widgets/track_actions.dart';
import '../widgets/track_row.dart';

void openPin(BuildContext context, Pin p) => switch (p.kind) {
      PinKind.album => openAlbum(context, p.id),
      PinKind.artist => openArtist(context, p.id),
      PinKind.playlist => openPlaylist(context, p.id),
      PinKind.userPlaylist => openLibraryPlaylist(context, p.id),
    };

/// Creates a playlist after asking its name; opens it when [open].
Future<void> createPlaylistFlow(BuildContext context, WidgetRef ref, {bool open = true}) async {
  final name = await promptText(context, title: 'Новый плейлист', label: 'Название', action: 'Создать');
  if (name == null || !context.mounted) return;
  final info = await guarded(context, () => ref.read(libraryApiProvider).createPlaylist(name));
  if (info != null && open && context.mounted) openLibraryPlaylist(context, info.id);
}

class LibraryScreen extends ConsumerWidget {
  /// Initial tab (0–4).
  final int initialTab;
  const LibraryScreen({super.key, this.initialTab = 0});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final side = Gap.side(MediaQuery.sizeOf(context).width);
    final t = Theme.of(context).textTheme;
    final pins = ref.watch(pinsProvider).value ?? const <Pin>[];
    return DefaultTabController(
      length: 5,
      initialIndex: initialTab.clamp(0, 4),
      child: Scaffold(
        backgroundColor: Colors.transparent,
        body: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Padding(
            padding: EdgeInsets.fromLTRB(side, 28, side - 8, 4),
            child: Row(children: [
              Expanded(
                child: Text('Библиотека',
                    style: t.headlineMedium?.copyWith(fontWeight: FontWeight.w800, letterSpacing: -0.6)),
              ),
              TextButton.icon(
                onPressed: () => openHistory(context),
                icon: const Icon(Icons.history_rounded),
                label: const Text('История'),
              ),
            ]),
          ),
          if (pins.isNotEmpty) _Pins(pins: pins, side: side),
          TabBar(
            isScrollable: true,
            tabAlignment: TabAlignment.start,
            padding: EdgeInsets.symmetric(horizontal: side - 12),
            dividerColor: Colors.transparent,
            tabs: const [
              Tab(text: 'Треки'),
              Tab(text: 'Альбомы'),
              Tab(text: 'Исполнители'),
              Tab(text: 'Плейлисты'),
              Tab(text: 'Мои плейлисты'),
            ],
          ),
          Divider(height: 1, color: Theme.of(context).colorScheme.outlineVariant.withValues(alpha: 0.5)),
          const Expanded(
            child: TabBarView(children: [
              _LikedTracks(),
              _LikedGrid(kind: LikeKind.album),
              _LikedGrid(kind: LikeKind.artist),
              _LikedGrid(kind: LikeKind.playlist),
              _UserPlaylists(),
            ]),
          ),
        ]),
      ),
    );
  }
}

class _Pins extends ConsumerWidget {
  final List<Pin> pins;
  final double side;
  const _Pins({required this.pins, required this.side});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final cs = Theme.of(context).colorScheme;
    return SizedBox(
      height: 72,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        padding: EdgeInsets.fromLTRB(side, 8, side, 8),
        itemCount: pins.length,
        separatorBuilder: (_, _) => const SizedBox(width: 10),
        itemBuilder: (context, i) {
          final p = pins[i];
          final round = p.kind == PinKind.artist;
          return Material(
            color: cs.surfaceContainer,
            borderRadius: BorderRadius.circular(10),
            clipBehavior: Clip.antiAlias,
            child: InkWell(
              onTap: () => openPin(context, p),
              onSecondaryTapUp: (d) => _unpinMenu(context, ref, p, d.globalPosition),
              onLongPress: () => _unpinMenu(context, ref, p, null),
              child: Padding(
                padding: const EdgeInsets.fromLTRB(6, 6, 16, 6),
                child: Row(mainAxisSize: MainAxisSize.min, children: [
                  CoverImage(
                    url: p.thumbnail,
                    size: 44,
                    shape: round ? CoverShape.circle : CoverShape.rounded,
                    icon: switch (p.kind) {
                      PinKind.artist => Icons.person_rounded,
                      PinKind.album => Icons.album_rounded,
                      _ => Icons.queue_music_rounded,
                    },
                    seed: p.id,
                  ),
                  const SizedBox(width: 12),
                  ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 180),
                    child: Text(p.title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(fontWeight: FontWeight.w600)),
                  ),
                  const SizedBox(width: 6),
                  Icon(Icons.push_pin_rounded, size: 14, color: cs.onSurfaceVariant),
                ]),
              ),
            ),
          );
        },
      ),
    );
  }

  Future<void> _unpinMenu(BuildContext context, WidgetRef ref, Pin p, Offset? at) async {
    final ok = await confirm(context, title: 'Открепить «${p.title}»?', action: 'Открепить');
    if (!ok || !context.mounted) return;
    await guarded(context, () => ref.read(libraryApiProvider).setPin(p.kind, p.id, on: false));
  }
}

class _LikedTracks extends ConsumerWidget {
  const _LikedTracks();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final liked = ref.watch(likedItemsProvider(LikeKind.track));
    final side = Gap.side(MediaQuery.sizeOf(context).width);
    return AsyncBody(
      value: liked,
      loading: const TrackListSkeleton(),
      onRetry: () => ref.invalidate(likedItemsProvider(LikeKind.track)),
      data: (items) {
        final tracks = [for (final i in items) if (i.item case TrackItem(:final track)) track];
        if (tracks.isEmpty) {
          return const EmptyView(
            icon: Icons.favorite_border_rounded,
            title: 'Пока нет любимых треков',
            subtitle: 'Нажмите ♥ у трека, и он появится здесь. Лайки учитывает «Моя волна»',
          );
        }
        return CustomScrollView(slivers: [
          SliverToBoxAdapter(
            child: Padding(
              padding: EdgeInsets.fromLTRB(side, 16, side, 8),
              child: Row(children: [
                PlayShuffleButtons(tracks: tracks),
                const Spacer(),
                Text(tracksCount(tracks.length),
                    style: TextStyle(color: Theme.of(context).colorScheme.onSurfaceVariant)),
              ]),
            ),
          ),
          TrackListSliver(
            tracks: tracks,
            padding: EdgeInsets.symmetric(horizontal: side - 8),
            onRemove: (i) => () => TrackActions(context, ref).toggleLike(tracks[i]),
            removeLabel: 'Убрать из любимых',
          ),
          const SliverToBoxAdapter(child: SizedBox(height: 32)),
        ]);
      },
    );
  }
}

class _LikedGrid extends ConsumerWidget {
  final LikeKind kind;
  const _LikedGrid({required this.kind});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final liked = ref.watch(likedItemsProvider(kind));
    final side = Gap.side(MediaQuery.sizeOf(context).width);
    final (icon, empty) = switch (kind) {
      LikeKind.album => (Icons.album_rounded, 'Сохранённых альбомов пока нет'),
      LikeKind.artist => (Icons.person_rounded, 'Сохранённых исполнителей пока нет'),
      _ => (Icons.queue_music_rounded, 'Сохранённых плейлистов пока нет'),
    };
    return AsyncBody(
      value: liked,
      loading: const Center(child: CircularProgressIndicator()),
      onRetry: () => ref.invalidate(likedItemsProvider(kind)),
      data: (items) => items.isEmpty
          ? EmptyView(icon: icon, title: empty, subtitle: 'Добавляйте их кнопкой ♥ на странице')
          : CustomScrollView(slivers: [
              CardGridSliver(
                padding: EdgeInsets.fromLTRB(side, 20, side, 32),
                builder: (w) => [for (final i in items) ItemCard(item: i.item, width: w)],
              ),
            ]),
    );
  }
}

class _UserPlaylists extends ConsumerWidget {
  const _UserPlaylists();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final lists = ref.watch(userPlaylistsProvider);
    final side = Gap.side(MediaQuery.sizeOf(context).width);
    final cs = Theme.of(context).colorScheme;
    return AsyncBody(
      value: lists,
      loading: const Center(child: CircularProgressIndicator()),
      onRetry: () => ref.invalidate(userPlaylistsProvider),
      data: (items) => CustomScrollView(slivers: [
        CardGridSliver(
          padding: EdgeInsets.fromLTRB(side, 20, side, 32),
          builder: (w) => [
            MediaCard(
              width: w,
              cover: Container(
                width: w,
                height: w,
                decoration: BoxDecoration(
                  color: cs.surfaceContainerHigh,
                  borderRadius: BorderRadius.circular(10),
                  border: Border.all(color: cs.outlineVariant),
                ),
                child: Icon(Icons.add_rounded, size: w * 0.3, color: cs.primary),
              ),
              title: 'Новый плейлист',
              onTap: () => createPlaylistFlow(context, ref),
            ),
            for (final p in items)
              MediaCard(
                width: w,
                cover: CoverCollage(urls: p.cover != null ? [p.cover!] : p.thumbnails, size: w, radius: 10, seed: p.id),
                title: p.name,
                subtitle: tracksCount(p.trackCount),
                onTap: () => openLibraryPlaylist(context, p.id),
              ),
          ],
        ),
      ]),
    );
  }
}
