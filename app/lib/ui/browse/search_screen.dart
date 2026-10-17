/// Search: suggestions while typing, filter chips, results grouped by kind
/// for "all" and an infinite list for one kind.
library;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../widgets/cards.dart';
import '../widgets/cover.dart';
import '../widgets/format.dart';
import '../widgets/states.dart';
import '../widgets/track_actions.dart';
import '../widgets/track_row.dart';
import 'providers.dart';

const _filterLabels = {
  SearchFilter.all: 'Всё',
  SearchFilter.songs: 'Треки',
  SearchFilter.videos: 'Видео',
  SearchFilter.albums: 'Альбомы',
  SearchFilter.artists: 'Исполнители',
  SearchFilter.playlists: 'Плейлисты',
};

class SearchScreen extends ConsumerStatefulWidget {
  /// From `/search?q=`: searched right away.
  final String? initialQuery;
  const SearchScreen({super.key, this.initialQuery});

  @override
  ConsumerState<SearchScreen> createState() => _SearchScreenState();
}

class _SearchScreenState extends ConsumerState<SearchScreen> {
  late final TextEditingController _text;
  final _focus = FocusNode();
  final _scroll = ScrollController();

  /// The text differs from the searched query: show suggestions.
  bool _typing = false;

  @override
  void initState() {
    super.initState();
    final s = ref.read(searchProvider);
    final q = widget.initialQuery;
    _text = TextEditingController(text: q ?? s.query);
    if (q != null && q.trim().isNotEmpty && q.trim() != s.query) {
      Future.microtask(() => ref.read(searchProvider.notifier).search(q));
    }
    if (_text.text.isEmpty) _focus.requestFocus();
    _scroll.addListener(_onScroll);
  }

  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _onScroll() {
    final p = _scroll.position;
    if (p.pixels > p.maxScrollExtent - 600) ref.read(searchProvider.notifier).loadMore();
  }

  void _submit(String q) {
    _text.text = q;
    _text.selection = TextSelection.collapsed(offset: q.length);
    setState(() => _typing = false);
    _focus.unfocus();
    ref.read(searchProvider.notifier).search(q);
    if (_scroll.hasClients) _scroll.jumpTo(0);
  }

  void _filter(SearchFilter f) {
    ref.read(searchProvider.notifier).setFilter(f);
    if (_scroll.hasClients) _scroll.jumpTo(0);
  }

  @override
  Widget build(BuildContext context) {
    final s = ref.watch(searchProvider);
    final width = MediaQuery.sizeOf(context).width;
    final side = Gap.side(width);
    final showSuggestions = _typing && _text.text.trim().isNotEmpty;

    return Scaffold(
      backgroundColor: Colors.transparent,
      body: Column(children: [
        Padding(
          padding: EdgeInsets.fromLTRB(side, 20, side, 8),
          child: _SearchField(
            controller: _text,
            focus: _focus,
            onChanged: (_) => setState(() => _typing = true),
            onSubmitted: _submit,
            onClear: () {
              _text.clear();
              setState(() => _typing = false);
              _focus.requestFocus();
            },
            onEscape: () => setState(() => _typing = false),
          ),
        ),
        if (!showSuggestions && s.query.isNotEmpty)
          SizedBox(
            height: 48,
            child: ListView(
              scrollDirection: Axis.horizontal,
              padding: EdgeInsets.symmetric(horizontal: side, vertical: 6),
              children: [
                for (final f in SearchFilter.values)
                  Padding(
                    padding: const EdgeInsets.only(right: 8),
                    child: ChoiceChip(
                      label: Text(_filterLabels[f]!),
                      selected: s.filter == f,
                      showCheckmark: false,
                      onSelected: (_) => _filter(f),
                    ),
                  ),
              ],
            ),
          ),
        Expanded(
          child: AnimatedSwitcher(
            duration: const Duration(milliseconds: 150),
            child: showSuggestions
                ? _Suggestions(
                    key: const ValueKey('suggest'),
                    input: _text.text,
                    onQuery: _submit,
                    onItem: () => setState(() => _typing = false),
                  )
                : KeyedSubtree(key: const ValueKey('results'), child: _results(s, side)),
          ),
        ),
      ]),
    );
  }

  Widget _results(SearchState s, double side) {
    if (s.query.isEmpty) {
      return const EmptyView(
        icon: Icons.search_rounded,
        title: 'Что послушаем?',
        subtitle: 'Ищите треки, альбомы, исполнителей и плейлисты',
      );
    }
    if (s.loading) return const TrackListSkeleton();
    if (s.error != null && s.isEmpty) {
      return ErrorView(error: s.error!, onRetry: () => ref.read(searchProvider.notifier).retry());
    }
    if (s.isEmpty) {
      return EmptyView(
        icon: Icons.search_off_rounded,
        title: 'Ничего не нашлось',
        subtitle: 'По запросу «${s.query}» в разделе «${_filterLabels[s.filter]}» пусто',
      );
    }
    final pad = EdgeInsets.symmetric(horizontal: side - 8);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted && s.hasMore && _scroll.hasClients && _scroll.position.maxScrollExtent < 400) {
        ref.read(searchProvider.notifier).loadMore();
      }
    });
    final slivers = <Widget>[
      if (s.filter == SearchFilter.all) ..._allSlivers(s, side) else ..._filteredSlivers(s, side, pad),
      if (s.loadingMore)
        const SliverToBoxAdapter(
          child: Padding(padding: EdgeInsets.all(24), child: Center(child: CircularProgressIndicator())),
        ),
      const SliverToBoxAdapter(child: SizedBox(height: 32)),
    ];
    return CustomScrollView(controller: _scroll, slivers: slivers);
  }

  List<Widget> _allSlivers(SearchState s, double side) {
    final tracks = <Track>[], videos = <Track>[];
    final albums = <Item>[], artists = <Item>[], playlists = <Item>[];
    for (final i in s.items) {
      switch (i) {
        case TrackItem(:final track):
          (track.kind == TrackKind.song ? tracks : videos).add(track);
        case AlbumItem():
          albums.add(i);
        case ArtistItem():
          artists.add(i);
        case PlaylistItem():
          playlists.add(i);
      }
    }
    Widget more(SearchFilter f) => TextButton(onPressed: () => _filter(f), child: const Text('Все'));
    Widget header(String title, SearchFilter f) => SliverToBoxAdapter(
          child: SectionHeader(
            title: title,
            padding: EdgeInsets.fromLTRB(side, 24, side - 8, 8),
            trailing: more(f),
          ),
        );
    Widget shelf(String title, SearchFilter f, List<Item> items) => SliverToBoxAdapter(
          child: Padding(
            padding: const EdgeInsets.only(top: 24),
            child: Shelf(
              title: title,
              padding: EdgeInsets.symmetric(horizontal: side),
              children: [for (final i in items) ItemCard(item: i, width: 160)],
            ),
          ),
        );
    return [
      if (s.topResult != null) SliverToBoxAdapter(child: _TopResult(item: s.topResult!, side: side)),
      if (tracks.isNotEmpty) ...[
        header('Треки', SearchFilter.songs),
        TrackListSliver(tracks: tracks, padding: EdgeInsets.symmetric(horizontal: side - 8)),
      ],
      if (artists.isNotEmpty) shelf('Исполнители', SearchFilter.artists, artists),
      if (albums.isNotEmpty) shelf('Альбомы', SearchFilter.albums, albums),
      if (videos.isNotEmpty) ...[
        header('Видео', SearchFilter.videos),
        TrackListSliver(tracks: videos, padding: EdgeInsets.symmetric(horizontal: side - 8)),
      ],
      if (playlists.isNotEmpty) shelf('Плейлисты', SearchFilter.playlists, playlists),
    ];
  }

  List<Widget> _filteredSlivers(SearchState s, double side, EdgeInsets pad) {
    switch (s.filter) {
      case SearchFilter.songs || SearchFilter.videos:
        final tracks = [for (final i in s.items) if (i is TrackItem) i.track];
        return [
          const SliverToBoxAdapter(child: SizedBox(height: 8)),
          TrackListSliver(tracks: tracks, padding: pad),
        ];
      case SearchFilter.artists:
        return [
          SliverPadding(
            padding: EdgeInsets.symmetric(horizontal: side - 8, vertical: 8),
            sliver: SliverList.builder(
              itemCount: s.items.length,
              itemBuilder: (_, i) => switch (s.items[i]) {
                ArtistItem(:final artist) => _ArtistTile(artist: artist),
                _ => const SizedBox.shrink(),
              },
            ),
          ),
        ];
      default:
        return [
          CardGridSliver(
            padding: EdgeInsets.fromLTRB(side, 12, side, 0),
            builder: (w) => [for (final i in s.items) ItemCard(item: i, width: w)],
          ),
        ];
    }
  }
}

class _SearchField extends StatelessWidget {
  final TextEditingController controller;
  final FocusNode focus;
  final ValueChanged<String> onChanged;
  final ValueChanged<String> onSubmitted;
  final VoidCallback onClear;
  final VoidCallback onEscape;
  const _SearchField({
    required this.controller,
    required this.focus,
    required this.onChanged,
    required this.onSubmitted,
    required this.onClear,
    required this.onEscape,
  });

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    return CallbackShortcuts(
      bindings: {const SingleActivator(LogicalKeyboardKey.escape): onEscape},
      child: TextField(
        controller: controller,
        focusNode: focus,
        onChanged: onChanged,
        onSubmitted: onSubmitted,
        textInputAction: TextInputAction.search,
        style: Theme.of(context).textTheme.titleMedium,
        decoration: InputDecoration(
          hintText: 'Исполнители, треки, альбомы',
          prefixIcon: const Icon(Icons.search_rounded),
          suffixIcon: ListenableBuilder(
            listenable: controller,
            builder: (_, _) => controller.text.isEmpty
                ? const SizedBox.shrink()
                : IconButton(tooltip: 'Очистить', onPressed: onClear, icon: const Icon(Icons.close_rounded)),
          ),
          filled: true,
          fillColor: cs.surfaceContainerHigh,
          contentPadding: const EdgeInsets.symmetric(vertical: 14),
          border: OutlineInputBorder(borderRadius: BorderRadius.circular(28), borderSide: BorderSide.none),
          focusedBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(28),
            borderSide: BorderSide(color: cs.primary.withValues(alpha: 0.6), width: 1.5),
          ),
        ),
      ),
    );
  }
}

class _Suggestions extends ConsumerStatefulWidget {
  final String input;
  final ValueChanged<String> onQuery;
  final VoidCallback onItem;
  const _Suggestions({super.key, required this.input, required this.onQuery, required this.onItem});

  @override
  ConsumerState<_Suggestions> createState() => _SuggestionsState();
}

class _SuggestionsState extends ConsumerState<_Suggestions> {
  /// The last answer, shown while the next keystroke's loads (no flicker).
  SearchSuggestions? _last;

  @override
  Widget build(BuildContext context) {
    final s = ref.watch(suggestionsProvider(widget.input.trim()));
    final cs = Theme.of(context).colorScheme;
    final side = Gap.side(MediaQuery.sizeOf(context).width) - 8;
    final onQuery = widget.onQuery;
    final onItem = widget.onItem;
    final data = s.value ?? _last;
    if (s.value != null) _last = s.value;
    if (data == null) {
      return s.hasError
          ? const SizedBox.shrink()
          : const Align(alignment: Alignment.topCenter, child: LinearProgressIndicator(minHeight: 2));
    }
    return ListView(
      padding: EdgeInsets.symmetric(horizontal: side, vertical: 4),
      children: [
        for (final q in data.queries)
          ListTile(
            shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
            leading: Icon(Icons.search_rounded, color: cs.onSurfaceVariant),
            title: Text(q),
            trailing: IconButton(
              tooltip: 'Подставить',
              icon: const Icon(Icons.north_west_rounded, size: 18),
              onPressed: () => onQuery(q),
            ),
            onTap: () => onQuery(q),
          ),
        if (data.queries.isNotEmpty && data.items.isNotEmpty) const Divider(height: 16, indent: 16, endIndent: 16),
        for (final i in data.items) _SuggestionItem(item: i, onOpen: onItem),
      ],
    );
  }
}

class _SuggestionItem extends ConsumerWidget {
  final Item item;
  final VoidCallback onOpen;
  const _SuggestionItem({required this.item, required this.onOpen});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final it = item;
    final round = it is ArtistItem;
    return ListTile(
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
      leading: CoverImage.thumbs(it.thumbnails,
          size: 44, shape: round ? CoverShape.circle : CoverShape.rounded, seed: it.title),
      title: Text(it.title, maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Text(
        switch (it) {
          TrackItem() => 'Трек · ${itemSubtitle(it)}',
          _ => itemSubtitle(it),
        },
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
      ),
      onTap: () {
        onOpen();
        if (it is TrackItem) {
          TrackActions(context, ref).play([it.track], 0);
        } else if (itemRoute(it) case final r?) {
          context.push(r);
        }
      },
    );
  }
}

class _TopResult extends ConsumerWidget {
  final Item item;
  final double side;
  const _TopResult({required this.item, required this.side});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final cs = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    final it = item;
    final round = it is ArtistItem;
    final kind = switch (it) {
      TrackItem(:final track) => track.kind == TrackKind.song ? 'Трек' : 'Видео',
      AlbumItem() => 'Альбом',
      ArtistItem() => 'Исполнитель',
      PlaylistItem() => 'Плейлист',
    };
    void open() {
      if (it is TrackItem) {
        TrackActions(context, ref).play([it.track], 0);
      } else if (itemRoute(it) case final r?) {
        context.push(r);
      }
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(side, 16, side, 0),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        SectionHeader(title: 'Лучший результат', padding: const EdgeInsets.only(bottom: 12)),
        Material(
          color: cs.surfaceContainer,
          borderRadius: BorderRadius.circular(16),
          clipBehavior: Clip.antiAlias,
          child: InkWell(
            onTap: open,
            onSecondaryTapUp: (d) => it is TrackItem
                ? showTrackMenu(context, ref, it.track, d.globalPosition)
                : showItemMenu(context, ref, it, d.globalPosition),
            child: Padding(
              padding: const EdgeInsets.all(18),
              child: Row(children: [
                CoverImage.thumbs(it.thumbnails,
                    size: 112, radius: 10, shape: round ? CoverShape.circle : CoverShape.rounded, seed: it.title),
                const SizedBox(width: 20),
                Expanded(
                  child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                    Text(it.title,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: t.headlineSmall?.copyWith(fontWeight: FontWeight.w800, letterSpacing: -0.4)),
                    const SizedBox(height: 6),
                    Row(children: [
                      Container(
                        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                        decoration: BoxDecoration(color: cs.secondaryContainer, borderRadius: BorderRadius.circular(20)),
                        child: Text(kind, style: t.labelSmall?.copyWith(color: cs.onSecondaryContainer)),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                            switch (it) {
                              // The badge already says it is an album.
                              AlbumItem(:final album) => [
                                  if (album.year != null) '${album.year}',
                                  artistNames(album.artists),
                                ].where((s) => s.isNotEmpty).join(' · '),
                              _ => itemSubtitle(it),
                            },
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: t.bodyMedium?.copyWith(color: cs.onSurfaceVariant)),
                      ),
                    ]),
                  ]),
                ),
                const SizedBox(width: 12),
                IconButton.filled(
                  iconSize: 28,
                  tooltip: it is TrackItem ? 'Слушать' : 'Открыть',
                  onPressed: open,
                  icon: Icon(it is TrackItem ? Icons.play_arrow_rounded : Icons.arrow_forward_rounded),
                ),
              ]),
            ),
          ),
        ),
      ]),
    );
  }
}

class _ArtistTile extends StatelessWidget {
  final ArtistSummary artist;
  const _ArtistTile({required this.artist});

  @override
  Widget build(BuildContext context) => ListTile(
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
        contentPadding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
        leading: CoverImage.thumbs(artist.thumbnails,
            size: 56, shape: CoverShape.circle, icon: Icons.person_rounded, seed: artist.id),
        title: Text(artist.name, style: const TextStyle(fontWeight: FontWeight.w600)),
        subtitle: artist.audience == null ? null : Text(artist.audience!),
        trailing: const Icon(Icons.chevron_right_rounded),
        onTap: () => context.push('/artist/${artist.id}'),
      );
}
