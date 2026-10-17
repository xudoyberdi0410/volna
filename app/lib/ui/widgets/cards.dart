/// Album / artist / playlist cards, horizontal shelves and section headers.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../../api/models.dart';
import '../../state/library.dart';
import '../nav.dart';
import 'cover.dart';
import 'format.dart';
import 'track_actions.dart';

/// Standard spacing of the browse screens.
abstract final class Gap {
  static const page = 24.0;
  static const pageNarrow = 16.0;
  static const section = 32.0;

  /// Horizontal page padding for [width].
  static double side(double width) => width < 600 ? pageNarrow : page;
}

/// Whether [openItem] can open [item] (a track needs an album).
bool canOpenItem(Item item) => item is! TrackItem || item.track.album?.id != null;

/// Opens the page of [item] (a track opens its album).
void openItem(BuildContext context, Item item) => switch (item) {
      AlbumItem(:final album) => openAlbum(context, album.browseId),
      ArtistItem(:final artist) => openArtist(context, artist.id),
      PlaylistItem(:final playlist) => openPlaylist(context, playlist.id),
      TrackItem(:final track) => track.album?.id == null ? null : openAlbum(context, track.album!.id!),
    };

/// Subtitle line for an item card.
String itemSubtitle(Item item) => switch (item) {
      AlbumItem(:final album) => [
          albumKindLabel(album.kind),
          if (album.year != null) '${album.year}',
          if (album.artists.isNotEmpty) artistNames(album.artists),
        ].join(' · '),
      ArtistItem(:final artist) => artist.audience ?? 'Исполнитель',
      PlaylistItem(:final playlist) => playlist.subtitle ??
          [
            'Плейлист',
            if (playlist.author != null) playlist.author!.name,
            if (playlist.trackCount != null) tracksCount(playlist.trackCount!),
          ].join(' · '),
      TrackItem(:final track) => artistNames(track.artists),
    };

class SectionHeader extends StatelessWidget {
  final String title;
  final String? strapline;
  final Widget? trailing;
  final EdgeInsets padding;
  const SectionHeader({
    super.key,
    required this.title,
    this.strapline,
    this.trailing,
    this.padding = const EdgeInsets.fromLTRB(24, 0, 16, 12),
  });

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    final cs = Theme.of(context).colorScheme;
    return Padding(
      padding: padding,
      child: Row(crossAxisAlignment: CrossAxisAlignment.end, children: [
        Expanded(
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
            if (strapline != null && strapline!.isNotEmpty)
              Text(strapline!.toUpperCase(),
                  style: t.labelSmall?.copyWith(color: cs.onSurfaceVariant, letterSpacing: 0.8)),
            Text(title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: t.titleLarge?.copyWith(fontWeight: FontWeight.w700, letterSpacing: -0.2)),
          ]),
        ),
        ?trailing,
      ]),
    );
  }
}

/// Height of [MediaCard]'s text under the cover (two title and two subtitle lines).
double mediaCardTextHeight(BuildContext context) {
  final t = Theme.of(context).textTheme;
  final s = MediaQuery.textScalerOf(context);
  return 10 + 2 * s.scale(t.titleSmall?.fontSize ?? 14) * 1.25 + 2 + 2 * s.scale(t.bodySmall?.fontSize ?? 12) * 1.3 + 2;
}

/// A square (or round, for artists) cover with two lines of text.
class MediaCard extends StatefulWidget {
  final Widget cover;
  final String title;
  final String? subtitle;
  final double width;
  final bool round;
  final VoidCallback? onTap;
  final VoidCallback? onPlay;
  final void Function(Offset at)? onMenu;

  const MediaCard({
    super.key,
    required this.cover,
    required this.title,
    this.subtitle,
    this.width = 168,
    this.round = false,
    this.onTap,
    this.onPlay,
    this.onMenu,
  });

  @override
  State<MediaCard> createState() => _MediaCardState();
}

class _MediaCardState extends State<MediaCard> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    final cs = Theme.of(context).colorScheme;
    return SizedBox(
      width: widget.width,
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          onTap: widget.onTap,
          onSecondaryTapUp: widget.onMenu == null ? null : (d) => widget.onMenu!(d.globalPosition),
          onLongPressStart: widget.onMenu == null ? null : (d) => widget.onMenu!(d.globalPosition),
          child: Column(
            crossAxisAlignment: widget.round ? CrossAxisAlignment.center : CrossAxisAlignment.start,
            children: [
              AnimatedScale(
                scale: _hover ? 1.025 : 1,
                duration: const Duration(milliseconds: 160),
                curve: Curves.easeOut,
                child: Stack(children: [
                  DecoratedBox(
                    decoration: BoxDecoration(
                      shape: widget.round ? BoxShape.circle : BoxShape.rectangle,
                      borderRadius: widget.round ? null : BorderRadius.circular(10),
                      boxShadow: [
                        BoxShadow(
                          color: Colors.black.withValues(alpha: _hover ? 0.28 : 0.14),
                          blurRadius: _hover ? 18 : 10,
                          offset: const Offset(0, 4),
                        ),
                      ],
                    ),
                    child: widget.cover,
                  ),
                  if (widget.onPlay != null)
                    Positioned(
                      right: 8,
                      bottom: 8,
                      child: AnimatedOpacity(
                        opacity: _hover ? 1 : 0,
                        duration: const Duration(milliseconds: 160),
                        child: IgnorePointer(
                          ignoring: !_hover,
                          child: IconButton.filled(
                            tooltip: 'Слушать',
                            onPressed: widget.onPlay,
                            icon: const Icon(Icons.play_arrow_rounded),
                          ),
                        ),
                      ),
                    ),
                ]),
              ),
              const SizedBox(height: 10),
              Text(widget.title,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  textAlign: widget.round ? TextAlign.center : TextAlign.start,
                  style: t.titleSmall?.copyWith(fontWeight: FontWeight.w600, height: 1.25)),
              if (widget.subtitle != null && widget.subtitle!.isNotEmpty) ...[
                const SizedBox(height: 2),
                Text(widget.subtitle!,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    textAlign: widget.round ? TextAlign.center : TextAlign.start,
                    style: t.bodySmall?.copyWith(color: cs.onSurfaceVariant, height: 1.3)),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// A card for any catalog [Item]; tracks play on tap.
class ItemCard extends ConsumerWidget {
  final Item item;
  final double width;
  const ItemCard({super.key, required this.item, this.width = 168});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final round = item is ArtistItem;
    final id = switch (item) {
      TrackItem(:final track) => track.videoId,
      AlbumItem(:final album) => album.browseId,
      ArtistItem(:final artist) => artist.id,
      PlaylistItem(:final playlist) => playlist.id,
    };
    final icon = switch (item) {
      ArtistItem() => Icons.person_rounded,
      PlaylistItem() => Icons.queue_music_rounded,
      AlbumItem() => Icons.album_rounded,
      TrackItem() => Icons.music_note_rounded,
    };
    final it = item;
    return MediaCard(
      width: width,
      round: round,
      cover: CoverImage.thumbs(
        item.thumbnails,
        size: width,
        radius: 10,
        shape: round ? CoverShape.circle : CoverShape.rounded,
        icon: icon,
        seed: id,
      ),
      title: item.title,
      subtitle: itemSubtitle(item),
      onTap: switch (it) {
        TrackItem(:final track) => () => TrackActions(context, ref).play([track], 0),
        _ => () => openItem(context, it),
      },
      onMenu: it is TrackItem
          ? (at) => showTrackMenu(context, ref, it.track, at)
          : (at) => showItemMenu(context, ref, it, at),
    );
  }
}

/// Like / pin menu for albums, artists and playlists.
Future<void> showItemMenu(BuildContext context, WidgetRef ref, Item item, Offset at) async {
  final liked = ref.read(likesProvider.notifier).isLiked(item);
  final overlay = Overlay.of(context).context.findRenderObject() as RenderBox;
  final f = await showMenu<VoidCallback>(
    context: context,
    position: RelativeRect.fromRect(at & const Size(1, 1), Offset.zero & overlay.size),
    items: [
      if (canOpenItem(item))
        PopupMenuItem(
          value: () => openItem(context, item),
          height: 40,
          child: const Row(children: [Icon(Icons.open_in_new_rounded, size: 20), SizedBox(width: 14), Text('Открыть')]),
        ),
      PopupMenuItem(
        value: () => guarded(context, () => ref.read(likesProvider.notifier).toggle(item)),
        height: 40,
        child: Row(children: [
          Icon(liked ? Icons.favorite_rounded : Icons.favorite_border_rounded, size: 20),
          const SizedBox(width: 14),
          Flexible(child: Text(liked ? 'Убрать из библиотеки' : 'В библиотеку', overflow: TextOverflow.ellipsis)),
        ]),
      ),
    ],
  );
  f?.call();
}

/// A titled horizontal row of cards, with scroll arrows on wide screens.
class Shelf extends StatefulWidget {
  final String title;
  final String? strapline;
  final List<Widget> children;
  final EdgeInsets padding;
  const Shelf({
    super.key,
    required this.title,
    this.strapline,
    required this.children,
    this.padding = const EdgeInsets.symmetric(horizontal: 24),
  });

  @override
  State<Shelf> createState() => _ShelfState();
}

class _ShelfState extends State<Shelf> {
  final _c = ScrollController();

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  void _page(int dir) {
    if (!_c.hasClients) return;
    final w = _c.position.viewportDimension * 0.8;
    _c.animateTo((_c.offset + dir * w).clamp(0, _c.position.maxScrollExtent),
        duration: const Duration(milliseconds: 380), curve: Curves.easeOutCubic);
  }

  @override
  Widget build(BuildContext context) {
    final wide = MediaQuery.sizeOf(context).width >= 600;
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      SectionHeader(
        title: widget.title,
        strapline: widget.strapline,
        padding: EdgeInsets.fromLTRB(widget.padding.left, 0, widget.padding.right - 8, 12),
        trailing: wide && widget.children.length > 3
            ? Row(mainAxisSize: MainAxisSize.min, children: [
                IconButton(
                    tooltip: 'Назад',
                    visualDensity: VisualDensity.compact,
                    onPressed: () => _page(-1),
                    icon: const Icon(Icons.chevron_left_rounded)),
                IconButton(
                    tooltip: 'Вперёд',
                    visualDensity: VisualDensity.compact,
                    onPressed: () => _page(1),
                    icon: const Icon(Icons.chevron_right_rounded)),
              ])
            : null,
      ),
      // A Row, not a ListView: the shelf is as tall as its tallest card
      // (text size varies), and shelves hold a few dozen cards at most.
      SingleChildScrollView(
        controller: _c,
        scrollDirection: Axis.horizontal,
        padding: widget.padding,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          spacing: 18,
          children: widget.children,
        ),
      ),
    ]);
  }
}

/// A responsive grid of cards as a sliver.
class CardGridSliver extends StatelessWidget {
  final List<Widget> Function(double cardWidth) builder;
  final EdgeInsets padding;
  const CardGridSliver({super.key, required this.builder, this.padding = const EdgeInsets.all(24)});

  @override
  Widget build(BuildContext context) => SliverPadding(
        padding: padding,
        sliver: SliverLayoutBuilder(builder: (context, c) {
          const spacing = 18.0;
          final w = c.crossAxisExtent;
          final cols = (w / 190).floor().clamp(2, 8);
          final cardW = (w - spacing * (cols - 1)) / cols;
          final children = builder(cardW);
          return SliverGrid(
            gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
              crossAxisCount: cols,
              crossAxisSpacing: spacing,
              mainAxisSpacing: 20,
              mainAxisExtent: cardW + mediaCardTextHeight(context),
            ),
            delegate: SliverChildBuilderDelegate((_, i) => children[i], childCount: children.length),
          );
        }),
      );
}
