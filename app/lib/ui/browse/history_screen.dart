/// Listening history, newest first, grouped by day.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../state/library.dart';
import '../widgets/cards.dart';
import '../widgets/format.dart';
import '../widgets/states.dart';
import '../widgets/track_actions.dart';
import '../widgets/track_row.dart';

/// "Now" for the day headers; tests pin it.
final clockProvider = Provider<DateTime Function()>((_) => DateTime.now);

class HistoryScreen extends ConsumerWidget {
  const HistoryScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final h = ref.watch(historyProvider);
    final side = Gap.side(MediaQuery.sizeOf(context).width);
    final t = Theme.of(context).textTheme;
    final cs = Theme.of(context).colorScheme;
    final empty = h.value?.isEmpty ?? true;
    final now = ref.watch(clockProvider)();

    Future<void> clear() async {
      final ok = await confirm(context,
          title: 'Очистить историю?',
          body: 'Список прослушанного удалится. Лайки и «Моя волна» не изменятся.',
          action: 'Очистить',
          destructive: true);
      if (ok && context.mounted) await guarded(context, () => ref.read(libraryApiProvider).clearHistory());
    }

    return Scaffold(
      backgroundColor: Colors.transparent,
      body: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Padding(
          padding: EdgeInsets.fromLTRB(side, 28, side - 8, 8),
          child: Row(children: [
            Expanded(
              child: Text('История', style: t.headlineMedium?.copyWith(fontWeight: FontWeight.w800, letterSpacing: -0.6)),
            ),
            if (!empty)
              TextButton.icon(
                onPressed: clear,
                icon: const Icon(Icons.delete_sweep_rounded),
                label: const Text('Очистить'),
                style: TextButton.styleFrom(foregroundColor: cs.onSurfaceVariant),
              ),
          ]),
        ),
        Expanded(
          child: AsyncBody(
            value: h,
            loading: const TrackListSkeleton(),
            onRetry: () => ref.invalidate(historyProvider),
            data: (items) {
              if (items.isEmpty) {
                return const EmptyView(
                  icon: Icons.history_rounded,
                  title: 'История пуста',
                  subtitle: 'Здесь появятся треки, которые вы слушали',
                );
              }
              final tracks = [for (final e in items) e.track];
              // Day headers between rows.
              final rows = <Object>[];
              String? day;
              for (var i = 0; i < items.length; i++) {
                final d = formatDay(items[i].playedAt, now: now);
                if (d != day) rows.add(day = d);
                rows.add(i);
              }
              return CustomScrollView(slivers: [
                SliverPadding(
                  padding: EdgeInsets.symmetric(horizontal: side - 8),
                  sliver: SliverList.builder(
                    itemCount: rows.length,
                    itemBuilder: (context, r) {
                      final row = rows[r];
                      if (row is String) {
                        return Padding(
                          padding: EdgeInsets.fromLTRB(8, r == 0 ? 8 : 20, 8, 6),
                          child: Text(row,
                              style: t.titleSmall?.copyWith(color: cs.onSurfaceVariant, fontWeight: FontWeight.w700)),
                        );
                      }
                      final i = row as int;
                      return TrackRow(
                        key: ValueKey('h$i'),
                        track: tracks[i],
                        trailingNote: formatTime(items[i].playedAt),
                        onTap: () => TrackActions(context, ref).play(tracks, i),
                      );
                    },
                  ),
                ),
                const SliverToBoxAdapter(child: SizedBox(height: 32)),
              ]);
            },
          ),
        ),
      ]),
    );
  }
}
