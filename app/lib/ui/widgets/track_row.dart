/// A track row (cover or number, title, linked artists, album, like,
/// duration, menu) and lists of them.
library;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../../state/library.dart';
import '../../state/player.dart' show playerViewProvider;
import '../nav.dart';
import 'cover.dart';
import 'format.dart';
import 'track_actions.dart';

/// Id of the track the player is on (to highlight its row).
final nowPlayingIdProvider =
    Provider<String?>((ref) => ref.watch(playerViewProvider.select((v) => v.state.trackId)));

/// Width above which rows show the album column.
const wideRowBreakpoint = 760.0;

class TrackRow extends ConsumerStatefulWidget {
  final Track track;

  /// Shown instead of the cover (album track numbers).
  final int? number;
  final bool showCover;
  final bool showAlbum;
  final VoidCallback? onTap;

  /// Adds a "remove" entry to the menu.
  final VoidCallback? onRemove;
  final String removeLabel;

  /// Extra text under the title after the artists (e.g. a play time).
  final String? trailingNote;

  /// Drag handle for reorderable lists.
  final Widget? leading;

  const TrackRow({
    super.key,
    required this.track,
    this.number,
    this.showCover = true,
    this.showAlbum = true,
    this.onTap,
    this.onRemove,
    this.removeLabel = 'Убрать из плейлиста',
    this.trailingNote,
    this.leading,
  });

  @override
  ConsumerState<TrackRow> createState() => _TrackRowState();
}

class _TrackRowState extends ConsumerState<TrackRow> {
  bool _hover = false;

  void _menu(Offset at) => showTrackMenu(context, ref, widget.track, at,
      onRemove: widget.onRemove, removeLabel: widget.removeLabel);

  @override
  Widget build(BuildContext context) {
    final t = widget.track;
    final theme = Theme.of(context);
    final cs = theme.colorScheme;
    final tt = theme.textTheme;
    final playing = ref.watch(nowPlayingIdProvider.select((id) => id == t.videoId));
    final liked = ref.watch(isLikedProvider((LikeKind.track, t.videoId)));
    final dim = !t.playable;
    final accent = playing ? cs.primary : null;

    Widget lead;
    if (widget.showCover) {
      lead = Stack(alignment: Alignment.center, children: [
        CoverImage.thumbs(t.thumbnails, size: 44, seed: t.videoId),
        if (_hover || playing)
          Container(
            width: 44,
            height: 44,
            decoration: BoxDecoration(color: Colors.black.withValues(alpha: 0.45), borderRadius: BorderRadius.circular(6)),
            child: Icon(playing && !_hover ? Icons.graphic_eq_rounded : Icons.play_arrow_rounded, color: Colors.white, size: 24),
          ),
      ]);
    } else {
      lead = SizedBox(
        width: 32,
        child: Center(
          child: _hover
              ? Icon(Icons.play_arrow_rounded, size: 22, color: cs.onSurface)
              : playing
                  ? Icon(Icons.graphic_eq_rounded, size: 20, color: cs.primary)
                  : Text('${widget.number ?? ''}',
                      style: tt.bodyMedium?.copyWith(color: cs.onSurfaceVariant, fontFeatures: const [FontFeature.tabularFigures()])),
        ),
      );
    }

    final width = MediaQuery.sizeOf(context).width;
    final showAlbum = widget.showAlbum && t.album != null && width >= wideRowBreakpoint;
    final subtitle = [
      // Not on numbered album lists: YouTube marks many album entries as videos.
      if (t.kind != TrackKind.song && widget.showCover) 'Видео',
      if (widget.trailingNote != null) widget.trailingNote!,
    ];

    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        onSecondaryTapUp: (d) => _menu(d.globalPosition),
        onLongPressStart: (d) => _menu(d.globalPosition),
        child: Material(
          color: _hover ? cs.onSurface.withValues(alpha: 0.06) : Colors.transparent,
          borderRadius: BorderRadius.circular(8),
          child: InkWell(
            borderRadius: BorderRadius.circular(8),
            onTap: widget.onTap,
            child: Opacity(
              opacity: dim ? 0.45 : 1,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
                child: Row(children: [
                  ?widget.leading,
                  lead,
                  const SizedBox(width: 14),
                  Expanded(
                    flex: 5,
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Row(children: [
                          Flexible(
                            child: Text(t.title,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: tt.bodyLarge?.copyWith(fontWeight: FontWeight.w500, color: accent, height: 1.25)),
                          ),
                          if (t.explicit) const ExplicitBadge(),
                        ]),
                        const SizedBox(height: 2),
                        ArtistLinks(
                          artists: t.artists,
                          suffix: subtitle.isEmpty ? null : subtitle.join(' · '),
                          style: tt.bodyMedium?.copyWith(color: cs.onSurfaceVariant, height: 1.25),
                        ),
                      ],
                    ),
                  ),
                  if (showAlbum) ...[
                    const SizedBox(width: 16),
                    Expanded(
                      flex: 3,
                      child: _AlbumLink(album: t.album!, style: tt.bodyMedium?.copyWith(color: cs.onSurfaceVariant)),
                    ),
                  ],
                  const SizedBox(width: 8),
                  LikeButton(
                    liked: liked,
                    visible: liked || _hover,
                    onPressed: () => TrackActions(context, ref).toggleLike(t),
                  ),
                  SizedBox(
                    width: 52,
                    child: Text(formatDuration(t.durationS),
                        textAlign: TextAlign.right,
                        style: tt.bodyMedium?.copyWith(color: cs.onSurfaceVariant, fontFeatures: const [FontFeature.tabularFigures()])),
                  ),
                  const SizedBox(width: 4),
                  Builder(
                    builder: (bctx) => IconButton(
                      tooltip: 'Ещё',
                      visualDensity: VisualDensity.compact,
                      icon: Icon(Icons.more_vert_rounded, color: _hover ? cs.onSurface : cs.onSurfaceVariant),
                      onPressed: () {
                        final box = bctx.findRenderObject() as RenderBox;
                        _menu(box.localToGlobal(box.size.bottomLeft(Offset.zero)));
                      },
                    ),
                  ),
                ]),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class ExplicitBadge extends StatelessWidget {
  const ExplicitBadge({super.key});

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    return Container(
      margin: const EdgeInsets.only(left: 6),
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
      decoration: BoxDecoration(color: cs.onSurfaceVariant.withValues(alpha: 0.25), borderRadius: BorderRadius.circular(3)),
      child: Text('E', style: TextStyle(fontSize: 10, fontWeight: FontWeight.w700, color: cs.onSurface, height: 1.2)),
    );
  }
}

class LikeButton extends StatelessWidget {
  final bool liked;
  final bool visible;
  final VoidCallback onPressed;
  final double size;
  const LikeButton({super.key, required this.liked, required this.onPressed, this.visible = true, this.size = 20});

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    return Opacity(
      opacity: visible ? 1 : 0,
      child: IconButton(
        tooltip: liked ? 'Убрать из любимых' : 'В любимые',
        visualDensity: VisualDensity.compact,
        onPressed: onPressed,
        icon: AnimatedSwitcher(
          duration: const Duration(milliseconds: 180),
          transitionBuilder: (c, a) => ScaleTransition(scale: a, child: c),
          child: Icon(
            liked ? Icons.favorite_rounded : Icons.favorite_border_rounded,
            key: ValueKey(liked),
            size: size,
            color: liked ? cs.primary : cs.onSurfaceVariant,
          ),
        ),
      ),
    );
  }
}

/// Comma-separated artist names; linked ones open the artist page.
class ArtistLinks extends StatefulWidget {
  final List<ArtistRef> artists;
  final TextStyle? style;
  final String? suffix;
  final int maxLines;
  const ArtistLinks({super.key, required this.artists, this.style, this.suffix, this.maxLines = 1});

  @override
  State<ArtistLinks> createState() => _ArtistLinksState();
}

class _ArtistLinksState extends State<ArtistLinks> {
  final _recognizers = <TapGestureRecognizer>[];
  int? _hover;

  @override
  void dispose() {
    for (final r in _recognizers) {
      r.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    for (final r in _recognizers) {
      r.dispose();
    }
    _recognizers.clear();
    final router = GoRouter.maybeOf(context);
    final spans = <InlineSpan>[];
    for (var i = 0; i < widget.artists.length; i++) {
      final a = widget.artists[i];
      if (a.name.isEmpty) continue;
      if (spans.isNotEmpty) spans.add(const TextSpan(text: ', '));
      final id = a.id;
      if (id != null && router != null) {
        final r = TapGestureRecognizer()..onTap = () => openArtist(context, id);
        _recognizers.add(r);
        spans.add(TextSpan(
          text: a.name,
          recognizer: r,
          mouseCursor: SystemMouseCursors.click,
          onEnter: (_) => setState(() => _hover = i),
          onExit: (_) => setState(() => _hover = null),
          style: _hover == i ? const TextStyle(decoration: TextDecoration.underline) : null,
        ));
      } else {
        spans.add(TextSpan(text: a.name));
      }
    }
    if (widget.suffix != null) {
      spans.add(TextSpan(text: spans.isEmpty ? widget.suffix : ' · ${widget.suffix}'));
    }
    return Text.rich(TextSpan(children: spans),
        style: widget.style, maxLines: widget.maxLines, overflow: TextOverflow.ellipsis);
  }
}

class _AlbumLink extends StatefulWidget {
  final AlbumRef album;
  final TextStyle? style;
  const _AlbumLink({required this.album, this.style});

  @override
  State<_AlbumLink> createState() => _AlbumLinkState();
}

class _AlbumLinkState extends State<_AlbumLink> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final id = widget.album.id;
    final text = Text(widget.album.name,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: widget.style?.copyWith(decoration: _hover ? TextDecoration.underline : null));
    if (id == null || GoRouter.maybeOf(context) == null) return text;
    return Align(
      alignment: Alignment.centerLeft,
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(onTap: () => openAlbum(context, id), child: text),
      ),
    );
  }
}

/// A sliver of [TrackRow]s; tapping one plays [tracks] from it.
class TrackListSliver extends ConsumerWidget {
  final List<Track> tracks;
  final bool numbered;
  final bool showCover;
  final bool showAlbum;
  final EdgeInsets padding;

  /// Builds the per-row "remove" action, if any.
  final VoidCallback? Function(int index)? onRemove;
  final String removeLabel;
  final String? Function(int index)? note;

  const TrackListSliver({
    super.key,
    required this.tracks,
    this.numbered = false,
    this.showCover = true,
    this.showAlbum = true,
    this.padding = const EdgeInsets.symmetric(horizontal: 16),
    this.onRemove,
    this.removeLabel = 'Убрать из плейлиста',
    this.note,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) => SliverPadding(
        padding: padding,
        sliver: SliverList.builder(
          itemCount: tracks.length,
          itemBuilder: (context, i) => TrackRow(
            key: ValueKey('${tracks[i].videoId}#$i'),
            track: tracks[i],
            number: numbered ? i + 1 : null,
            showCover: showCover,
            showAlbum: showAlbum,
            onTap: () => TrackActions(context, ref).play(tracks, i),
            onRemove: onRemove?.call(i),
            removeLabel: removeLabel,
            trailingNote: note?.call(i),
          ),
        ),
      );
}

/// "Слушать" and "Перемешать" for a list of tracks.
class PlayShuffleButtons extends ConsumerWidget {
  final List<Track> tracks;
  final List<Widget> extra;
  const PlayShuffleButtons({super.key, required this.tracks, this.extra = const []});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final a = TrackActions(context, ref);
    final empty = tracks.every((t) => !t.playable);
    return ActionBar(children: [
      FilledButton.icon(
        onPressed: empty ? null : () => a.playAll(tracks),
        icon: const Icon(Icons.play_arrow_rounded),
        label: const Text('Слушать'),
        style: FilledButton.styleFrom(minimumSize: const Size(0, 44), padding: const EdgeInsets.symmetric(horizontal: 22)),
      ),
      ShuffleButton(onPressed: empty ? null : () => a.shuffle(tracks)),
      ...extra,
    ]);
  }
}

/// "Перемешать": icon-only on phones, so a header's buttons fit one line.
class ShuffleButton extends StatelessWidget {
  final VoidCallback? onPressed;
  const ShuffleButton({super.key, this.onPressed});

  @override
  Widget build(BuildContext context) {
    if (MediaQuery.sizeOf(context).width < 480) {
      return IconButton.filledTonal(
        tooltip: 'Перемешать',
        onPressed: onPressed,
        icon: const Icon(Icons.shuffle_rounded),
        style: IconButton.styleFrom(minimumSize: const Size(44, 44)),
      );
    }
    return FilledButton.tonalIcon(
      onPressed: onPressed,
      icon: const Icon(Icons.shuffle_rounded),
      label: const Text('Перемешать'),
      style: FilledButton.styleFrom(minimumSize: const Size(0, 44), padding: const EdgeInsets.symmetric(horizontal: 18)),
    );
  }
}

/// A row of header actions; centred under a stacked (narrow) header.
class ActionBar extends StatelessWidget {
  final List<Widget> children;
  const ActionBar({super.key, required this.children});

  @override
  Widget build(BuildContext context) => Wrap(
        spacing: 8,
        runSpacing: 8,
        alignment: MediaQuery.sizeOf(context).width < 640 ? WrapAlignment.center : WrapAlignment.start,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: children,
      );
}
