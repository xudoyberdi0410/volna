/// What a track can do: play a list from a track, play next, add to the
/// queue or a playlist, go to its album or artist. Playback is the daemon's
/// (`crates/daemon/src/player/commands.rs`); the window only sends commands.
library;

import 'dart:math';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../api/player.dart';
import '../../api/player_models.dart';
import '../../state/connection.dart';
import '../../state/library.dart';
import '../nav.dart';
import 'cover.dart';
import 'format.dart';
import 'states.dart';

/// A queue entry for [t]. `ui` carries the full ytm track, so the player
/// screens can like it or open its album without another lookup.
TrackInput trackInput(Track t) {
  final base = TrackInput.fromTrack(t, origin: Origin.user);
  return TrackInput(
    id: base.id,
    title: base.title,
    artists: base.artists,
    album: base.album,
    duration: base.duration,
    cover: base.cover,
    explicit: base.explicit,
    kind: base.kind,
    origin: base.origin,
    ui: t.toJson(),
  );
}

void showSnack(BuildContext context, String text) {
  final m = ScaffoldMessenger.maybeOf(context);
  m?.hideCurrentSnackBar();
  m?.showSnackBar(SnackBar(content: Text(text), behavior: SnackBarBehavior.floating, width: 420));
}

/// Runs [f]; a failure becomes a snack bar.
Future<T?> guarded<T>(BuildContext context, Future<T> Function() f) async {
  try {
    return await f();
  } catch (e) {
    if (context.mounted) showSnack(context, errorText(e));
    return null;
  }
}

class TrackActions {
  final WidgetRef ref;
  final BuildContext context;
  const TrackActions(this.context, this.ref);

  PlayerApi get _player => ref.read(playerProvider);

  /// Replaces the queue with [tracks] and plays from [index]. Unplayable
  /// rows are skipped.
  Future<void> play(List<Track> tracks, int index) async {
    final start = tracks[index];
    final list = [for (final t in tracks) if (t.playable || identical(t, start)) t];
    final at = list.indexOf(start);
    await guarded(context, () async {
      await _player.setQueue([for (final t in list) trackInput(t)], startIndex: max(at, 0), play: true);
    });
  }

  Future<void> playAll(List<Track> tracks) async {
    final list = [for (final t in tracks) if (t.playable) t];
    if (list.isEmpty) return;
    await guarded(context, () async {
      await _player.setQueue([for (final t in list) trackInput(t)], startIndex: 0, play: true);
      await _player.setShuffle(false);
    });
  }

  Future<void> shuffle(List<Track> tracks) async {
    final list = [for (final t in tracks) if (t.playable) t];
    if (list.isEmpty) return;
    await guarded(context, () async {
      await _player.setQueue([for (final t in list) trackInput(t)],
          startIndex: Random().nextInt(list.length), play: true);
      await _player.setShuffle(true);
    });
  }

  Future<void> playNext(List<Track> tracks) async {
    final ok = await guarded(context, () => _player.enqueue([for (final t in tracks) trackInput(t)], EnqueueAt.next));
    if (ok != null && context.mounted) {
      showSnack(context, tracks.length == 1 ? 'Сыграет следующим' : 'Сыграют следующими');
    }
  }

  Future<void> addToQueue(List<Track> tracks) async {
    final ok = await guarded(context, () => _player.enqueue([for (final t in tracks) trackInput(t)], EnqueueAt.end));
    if (ok != null && context.mounted) showSnack(context, 'Добавлено в очередь');
  }

  Future<void> toggleLike(Track t) => guarded(context, () => ref.read(likesProvider.notifier).toggle(TrackItem(t)));

  Future<void> addToPlaylist(List<Track> tracks) => showAddToPlaylist(context, ref, tracks);
}

/// The ⋮ menu of a track row. [userPlaylistId] adds "remove from this playlist".
List<PopupMenuEntry<VoidCallback>> trackMenuItems(
  BuildContext context,
  WidgetRef ref,
  Track t, {
  VoidCallback? onRemove,
  String removeLabel = 'Убрать из плейлиста',
}) {
  final a = TrackActions(context, ref);
  final liked = ref.read(likesProvider).contains(LikeKind.track, t.videoId);
  final router = GoRouter.maybeOf(context);
  final albumId = t.album?.id;
  PopupMenuItem<VoidCallback> item(IconData icon, String label, VoidCallback f) => PopupMenuItem(
        value: f,
        height: 40,
        child: Row(children: [
          Icon(icon, size: 20),
          const SizedBox(width: 14),
          Flexible(child: Text(label, overflow: TextOverflow.ellipsis)),
        ]),
      );
  final artist = t.artists.where((x) => x.id != null).firstOrNull;
  return [
    item(Icons.playlist_play_rounded, 'Играть следующим', () => a.playNext([t])),
    item(Icons.queue_music_rounded, 'Добавить в очередь', () => a.addToQueue([t])),
    item(Icons.playlist_add_rounded, 'Добавить в плейлист…', () => a.addToPlaylist([t])),
    item(liked ? Icons.favorite_rounded : Icons.favorite_border_rounded,
        liked ? 'Убрать из любимых' : 'В любимые', () => a.toggleLike(t)),
    if (router != null && (t.album?.id != null || artist != null)) const PopupMenuDivider(height: 8),
    if (router != null && t.album?.id != null)
      item(Icons.album_rounded, 'Перейти к альбому', () => openAlbum(context, albumId!)),
    if (router != null && artist != null)
      item(Icons.person_rounded, 'Перейти к исполнителю', () => openArtist(context, artist.id!)),
    if (onRemove != null) ...[
      const PopupMenuDivider(height: 8),
      item(Icons.remove_circle_outline_rounded, removeLabel, onRemove),
    ],
  ];
}

/// Opens [trackMenuItems] at [position] (a right click) or under a button.
Future<void> showTrackMenu(BuildContext context, WidgetRef ref, Track t, Offset position,
    {VoidCallback? onRemove, String removeLabel = 'Убрать из плейлиста'}) async {
  final overlay = Overlay.of(context).context.findRenderObject() as RenderBox;
  final f = await showMenu<VoidCallback>(
    context: context,
    position: RelativeRect.fromRect(position & const Size(1, 1), Offset.zero & overlay.size),
    items: trackMenuItems(context, ref, t, onRemove: onRemove, removeLabel: removeLabel),
  );
  f?.call();
}

/// Picks a user playlist (or creates one) and adds [tracks] to it.
Future<void> showAddToPlaylist(BuildContext context, WidgetRef ref, List<Track> tracks) async {
  final api = ref.read(libraryApiProvider);
  final picked = await showDialog<_Pick>(
    context: context,
    builder: (_) => const _PlaylistPicker(),
  );
  if (picked == null || !context.mounted) return;
  final String? name;
  final int? added;
  switch (picked) {
    case _New():
      final title = await promptText(context, title: 'Новый плейлист', label: 'Название', action: 'Создать');
      if (title == null || !context.mounted) return;
      final info = await guarded(context, () => api.createPlaylist(title, tracks: tracks));
      name = info?.name;
      added = info == null ? null : tracks.length;
    case _Existing(:final info):
      name = info.name;
      added = await guarded(context, () => api.addToPlaylist(info.id, tracks));
  }
  if (added == null || !context.mounted) return;
  showSnack(
    context,
    added == 0 ? 'Уже есть в «$name»' : 'Добавлено в «$name»',
  );
}

sealed class _Pick {
  const _Pick();
}

class _New extends _Pick {
  const _New();
}

class _Existing extends _Pick {
  final UserPlaylistInfo info;
  const _Existing(this.info);
}

class _PlaylistPicker extends ConsumerWidget {
  const _PlaylistPicker();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final lists = ref.watch(userPlaylistsProvider);
    final cs = Theme.of(context).colorScheme;
    return Dialog(
      clipBehavior: Clip.antiAlias,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 400, maxHeight: 520),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(24, 22, 24, 12),
              child: Text('Добавить в плейлист', style: Theme.of(context).textTheme.titleLarge),
            ),
            ListTile(
              contentPadding: const EdgeInsets.symmetric(horizontal: 24),
              leading: Container(
                width: 44,
                height: 44,
                decoration: BoxDecoration(color: cs.primaryContainer, borderRadius: BorderRadius.circular(6)),
                child: Icon(Icons.add_rounded, color: cs.onPrimaryContainer),
              ),
              title: const Text('Новый плейлист'),
              onTap: () => Navigator.pop(context, const _New()),
            ),
            Flexible(
              child: switch (lists) {
                AsyncData(:final value) => ListView(
                    shrinkWrap: true,
                    children: [
                      for (final p in value)
                        ListTile(
                          contentPadding: const EdgeInsets.symmetric(horizontal: 24),
                          leading: CoverCollage(urls: p.cover != null ? [p.cover!] : p.thumbnails, size: 44, seed: p.id),
                          title: Text(p.name, maxLines: 1, overflow: TextOverflow.ellipsis),
                          subtitle: Text(tracksCount(p.trackCount)),
                          onTap: () => Navigator.pop(context, _Existing(p)),
                        ),
                    ],
                  ),
                AsyncError(:final error) => ErrorView(error: error, compact: true),
                _ => const Padding(
                    padding: EdgeInsets.all(24),
                    child: Center(child: CircularProgressIndicator()),
                  ),
              },
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 4, 16, 12),
              child: Align(
                alignment: Alignment.centerRight,
                child: TextButton(onPressed: () => Navigator.pop(context), child: const Text('Отмена')),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// A one-field dialog; null when cancelled or empty.
Future<String?> promptText(
  BuildContext context, {
  required String title,
  required String label,
  required String action,
  String initial = '',
}) async {
  final r = await showDialog<String>(
    context: context,
    builder: (_) => _PromptDialog(title: title, label: label, action: action, initial: initial),
  );
  final v = r?.trim();
  return v == null || v.isEmpty ? null : v;
}

/// Owns its controller: the field is still on screen during the exit animation.
class _PromptDialog extends StatefulWidget {
  final String title, label, action, initial;
  const _PromptDialog({required this.title, required this.label, required this.action, required this.initial});

  @override
  State<_PromptDialog> createState() => _PromptDialogState();
}

class _PromptDialogState extends State<_PromptDialog> {
  late final _c = TextEditingController(text: widget.initial)
    ..selection = TextSelection(baseOffset: 0, extentOffset: widget.initial.length);

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
        title: Text(widget.title),
        content: SizedBox(
          width: 360,
          child: TextField(
            controller: _c,
            autofocus: true,
            decoration: InputDecoration(labelText: widget.label),
            onSubmitted: (v) => Navigator.pop(context, v),
          ),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context), child: const Text('Отмена')),
          FilledButton(onPressed: () => Navigator.pop(context, _c.text), child: Text(widget.action)),
        ],
      );
}

/// A yes/no dialog.
Future<bool> confirm(BuildContext context,
    {required String title, String? body, required String action, bool destructive = false}) async {
  final cs = Theme.of(context).colorScheme;
  final r = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(title),
      content: body == null ? null : Text(body),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Отмена')),
        FilledButton(
          style: destructive ? FilledButton.styleFrom(backgroundColor: cs.error, foregroundColor: cs.onError) : null,
          onPressed: () => Navigator.pop(context, true),
          child: Text(action),
        ),
      ],
    ),
  );
  return r ?? false;
}
