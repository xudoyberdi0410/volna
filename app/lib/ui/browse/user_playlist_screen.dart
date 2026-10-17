/// A user playlist: play, rename, delete, pin, reorder by drag, remove tracks.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../nav.dart';
import '../../state/library.dart';
import '../widgets/cards.dart';
import '../widgets/collection_header.dart';
import '../widgets/cover.dart';
import '../widgets/format.dart';
import '../widgets/states.dart';
import '../widgets/track_actions.dart';
import '../widgets/track_row.dart';

class UserPlaylistScreen extends ConsumerWidget {
  final String id;
  const UserPlaylistScreen({super.key, required this.id});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final p = ref.watch(userPlaylistProvider(id));
    return Scaffold(
      backgroundColor: Colors.transparent,
      body: AsyncBody(
        value: p,
        onRetry: () => ref.invalidate(userPlaylistProvider(id)),
        data: (pl) => _UserPlaylistView(playlist: pl),
      ),
    );
  }
}

class _UserPlaylistView extends ConsumerStatefulWidget {
  final UserPlaylist playlist;
  const _UserPlaylistView({required this.playlist});

  @override
  ConsumerState<_UserPlaylistView> createState() => _UserPlaylistViewState();
}

class _UserPlaylistViewState extends ConsumerState<_UserPlaylistView> {
  /// Local order, so a drag shows at once; replaced by the daemon's on refresh.
  late List<Track> _tracks = widget.playlist.tracks;

  @override
  void didUpdateWidget(covariant _UserPlaylistView old) {
    super.didUpdateWidget(old);
    if (!identical(old.playlist, widget.playlist)) _tracks = widget.playlist.tracks;
  }

  LibraryApi get _api => ref.read(libraryApiProvider);
  String get _id => widget.playlist.info.id;

  /// [to] is already adjusted for the removal (as the daemon's `playlist_move`).
  Future<void> _reorder(int from, int to) async {
    if (from == to) return;
    setState(() {
      final t = _tracks.removeAt(from);
      _tracks.insert(to, t);
    });
    final ok = await guarded(context, () async {
      await _api.movePlaylistEntry(_id, from, to);
      return true;
    });
    if (ok == null && mounted) setState(() => _tracks = widget.playlist.tracks);
  }

  Future<void> _remove(int i) async {
    final t = _tracks[i];
    setState(() => _tracks = [..._tracks]..removeAt(i));
    final n = await guarded(context, () => _api.removeFromPlaylist(_id, [t.videoId]));
    if (n == null && mounted) setState(() => _tracks = widget.playlist.tracks);
  }

  Future<void> _rename() async {
    final info = widget.playlist.info;
    final name = await promptText(context,
        title: 'Переименовать плейлист', label: 'Название', action: 'Сохранить', initial: info.name);
    if (name == null || name == info.name || !mounted) return;
    await guarded(context, () => _api.renamePlaylist(_id, name: name));
  }

  Future<void> _describe() async {
    final info = widget.playlist.info;
    final d = await promptText(context,
        title: 'Описание', label: 'Описание плейлиста', action: 'Сохранить', initial: info.description);
    if (d == null || !mounted) return;
    await guarded(context, () => _api.renamePlaylist(_id, description: d));
  }

  Future<void> _delete() async {
    final info = widget.playlist.info;
    final ok = await confirm(context,
        title: 'Удалить «${info.name}»?',
        body: 'Плейлист удалится вместе со списком треков. Это нельзя отменить.',
        action: 'Удалить',
        destructive: true);
    if (!ok || !mounted) return;
    final gone = await guarded(context, () => _api.deletePlaylist(_id));
    if (gone == true && mounted) {
      final router = GoRouter.of(context);
      router.canPop() ? router.pop() : openLibrary(context);
    }
  }

  @override
  Widget build(BuildContext context) {
    final info = widget.playlist.info;
    final width = MediaQuery.sizeOf(context).width;
    final side = Gap.side(width);
    final pinned = ref.watch(isPinnedProvider((PinKind.userPlaylist, info.id)));
    final total = _tracks.fold<int>(0, (s, t) => s + (t.durationS ?? 0));
    final meta = [
      tracksCount(_tracks.length),
      if (total > 0) formatLongDuration(total),
    ].join(' · ');
    final actions = TrackActions(context, ref);
    final cs = Theme.of(context).colorScheme;

    return CustomScrollView(slivers: [
      SliverToBoxAdapter(
        child: CollectionHeader(
          cover: (s) => CoverCollage(
              urls: info.cover != null ? [info.cover!] : info.thumbnails, size: s, radius: 12, seed: info.id),
          kind: 'Мой плейлист',
          title: info.name,
          meta: Text(meta),
          description: info.description,
          actions: PlayShuffleButtons(tracks: _tracks, extra: [
            IconButton(tooltip: 'Переименовать', onPressed: _rename, icon: const Icon(Icons.edit_rounded)),
            PopupMenuButton<VoidCallback>(
              tooltip: 'Ещё',
              icon: const Icon(Icons.more_horiz_rounded),
              onSelected: (f) => f(),
              itemBuilder: (_) => [
                PopupMenuItem(value: () => actions.addToQueue(_tracks), child: const Text('Добавить в очередь')),
                PopupMenuItem(value: _rename, child: const Text('Переименовать')),
                PopupMenuItem(value: _describe, child: const Text('Изменить описание')),
                PopupMenuItem(
                  value: () => guarded(context, () => _api.setPin(PinKind.userPlaylist, pinned ? info.id : info.toPinJson(), on: !pinned)),
                  child: Text(pinned ? 'Открепить' : 'Закрепить в библиотеке'),
                ),
                const PopupMenuDivider(),
                PopupMenuItem(
                  value: _delete,
                  child: Text('Удалить плейлист', style: TextStyle(color: cs.error)),
                ),
              ],
            ),
          ]),
        ),
      ),
      if (_tracks.isEmpty)
        SliverToBoxAdapter(
          child: EmptyView(
            icon: Icons.playlist_add_rounded,
            title: 'В плейлисте пока пусто',
            subtitle: 'Добавляйте треки через меню ⋮ → «Добавить в плейлист…»',
            action: FilledButton.tonalIcon(
              onPressed: () => openSearch(context),
              icon: const Icon(Icons.search_rounded),
              label: const Text('Найти музыку'),
            ),
          ),
        )
      else
        SliverPadding(
          padding: EdgeInsets.symmetric(horizontal: side - 8),
          sliver: SliverReorderableList(
            itemCount: _tracks.length,
            onReorderItem: _reorder,
            proxyDecorator: (child, _, anim) => Material(
              elevation: 6,
              color: cs.surfaceContainerHigh,
              borderRadius: BorderRadius.circular(8),
              child: child,
            ),
            itemBuilder: (context, i) => TrackRow(
              key: ValueKey(_tracks[i].videoId),
              track: _tracks[i],
              leading: ReorderableDragStartListener(
                index: i,
                child: MouseRegion(
                  cursor: SystemMouseCursors.grab,
                  child: Padding(
                    padding: const EdgeInsets.only(right: 6),
                    child: Icon(Icons.drag_indicator_rounded, size: 20, color: cs.onSurfaceVariant),
                  ),
                ),
              ),
              onTap: () => actions.play(_tracks, i),
              onRemove: () => _remove(i),
            ),
          ),
        ),
      const SliverToBoxAdapter(child: SizedBox(height: 40)),
    ]);
  }
}
