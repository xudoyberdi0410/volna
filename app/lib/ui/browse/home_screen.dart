/// Home: the logged-out YouTube Music feed and "moods & genres".
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../api/models.dart';
import '../widgets/cards.dart';
import '../widgets/states.dart';
import 'providers.dart';

class HomeScreen extends ConsumerWidget {
  const HomeScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final home = ref.watch(homeProvider);
    final moods = ref.watch(moodsProvider);
    final side = Gap.side(MediaQuery.sizeOf(context).width);
    final t = Theme.of(context).textTheme;
    return Scaffold(
      backgroundColor: Colors.transparent,
      body: RefreshIndicator(
        onRefresh: () async {
          ref.invalidate(homeProvider);
          await ref.read(homeProvider.future).catchError((_) => const <Section>[]);
        },
        child: CustomScrollView(slivers: [
          SliverToBoxAdapter(
            child: Padding(
              padding: EdgeInsets.fromLTRB(side, 28, side, 8),
              child: Text('Главная', style: t.headlineMedium?.copyWith(fontWeight: FontWeight.w800, letterSpacing: -0.6)),
            ),
          ),
          if (moods.value case final cats? when cats.isNotEmpty)
            SliverToBoxAdapter(child: _MoodChips(categories: cats, side: side)),
          ...switch (home) {
            AsyncValue(:final value?, hasValue: true) => [
                for (final s in value)
                  if (s.items.isNotEmpty) SliverToBoxAdapter(child: _SectionShelf(section: s, side: side)),
              ],
            AsyncError(:final error) => [
                SliverFillRemaining(
                  hasScrollBody: false,
                  child: ErrorView(error: error, onRetry: () => ref.invalidate(homeProvider)),
                ),
              ],
            _ => [const SliverToBoxAdapter(child: _ShelvesSkeleton())],
          },
          const SliverToBoxAdapter(child: SizedBox(height: 40)),
        ]),
      ),
    );
  }
}

class _SectionShelf extends StatelessWidget {
  final Section section;
  final double side;
  const _SectionShelf({required this.section, required this.side});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: 28),
      child: Shelf(
        title: section.title,
        strapline: section.strapline,
        padding: EdgeInsets.symmetric(horizontal: side),
        children: [for (final i in section.items) ItemCard(item: i, width: 168)],
      ),
    );
  }
}

class _MoodChips extends StatelessWidget {
  final List<MoodCategory> categories;
  final double side;
  const _MoodChips({required this.categories, required this.side});

  @override
  Widget build(BuildContext context) {
    final moods = [for (final c in categories) ...c.moods];
    final cs = Theme.of(context).colorScheme;
    return SizedBox(
      height: 52,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        padding: EdgeInsets.symmetric(horizontal: side, vertical: 8),
        itemCount: moods.length,
        separatorBuilder: (_, _) => const SizedBox(width: 8),
        itemBuilder: (context, i) {
          final m = moods[i];
          final accent = m.color == null ? cs.primary : Color(m.color!);
          return ActionChip(
            avatar: Container(width: 4, height: 18, decoration: BoxDecoration(color: accent, borderRadius: BorderRadius.circular(2))),
            label: Text(m.title),
            onPressed: () => context.push('/mood/${Uri.encodeComponent(m.params)}?title=${Uri.encodeComponent(m.title)}'),
          );
        },
      ),
    );
  }
}

class _ShelvesSkeleton extends StatelessWidget {
  const _ShelvesSkeleton();

  @override
  Widget build(BuildContext context) {
    final side = Gap.side(MediaQuery.sizeOf(context).width);
    return Skeleton(
      child: Column(children: [
        for (var s = 0; s < 3; s++)
          Padding(
            padding: EdgeInsets.fromLTRB(side, 28, 0, 0),
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              const SkeletonBox(width: 200, height: 20),
              const SizedBox(height: 14),
              SizedBox(
                height: 210,
                child: ListView(
                  scrollDirection: Axis.horizontal,
                  physics: const NeverScrollableScrollPhysics(),
                  children: [
                    for (var i = 0; i < 8; i++)
                      const Padding(
                        padding: EdgeInsets.only(right: 18),
                        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                          SkeletonBox(width: 168, height: 168, radius: 10),
                          SizedBox(height: 10),
                          SkeletonBox(width: 120, height: 12),
                          SizedBox(height: 6),
                          SkeletonBox(width: 80, height: 10),
                        ]),
                      ),
                  ],
                ),
              ),
            ]),
          ),
      ]),
    );
  }
}

/// Playlists of one mood or genre.
class MoodScreen extends ConsumerWidget {
  final String params;
  final String title;
  const MoodScreen({super.key, required this.params, required this.title});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sections = ref.watch(moodProvider(params));
    final side = Gap.side(MediaQuery.sizeOf(context).width);
    final t = Theme.of(context).textTheme;
    return Scaffold(
      backgroundColor: Colors.transparent,
      body: AsyncBody(
        value: sections,
        loading: const _ShelvesSkeleton(),
        onRetry: () => ref.invalidate(moodProvider(params)),
        data: (list) => CustomScrollView(slivers: [
          SliverToBoxAdapter(
            child: Padding(
              padding: EdgeInsets.fromLTRB(side, 28, side, 0),
              child: Text(title, style: t.headlineMedium?.copyWith(fontWeight: FontWeight.w800, letterSpacing: -0.6)),
            ),
          ),
          if (list.every((s) => s.items.isEmpty))
            const SliverFillRemaining(
              hasScrollBody: false,
              child: EmptyView(icon: Icons.queue_music_rounded, title: 'Здесь пока пусто'),
            ),
          for (final s in list)
            if (s.items.isNotEmpty) SliverToBoxAdapter(child: _SectionShelf(section: s, side: side)),
          const SliverToBoxAdapter(child: SizedBox(height: 40)),
        ]),
      ),
    );
  }
}
