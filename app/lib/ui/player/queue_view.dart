/// The play queue: the current track, then what comes next, reorderable by
/// drag (`player_move`) and removable (`player_remove`). Tap plays an entry.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/player_models.dart';
import '../../state/player.dart';
import '../theme/app_theme.dart';
import 'cover.dart';

class QueueView extends ConsumerWidget {
  final EdgeInsets padding;
  const QueueView({super.key, this.padding = const EdgeInsets.fromLTRB(8, 0, 8, 24)});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final v = ref.watch(playerViewProvider);
    final ctl = ref.read(playerViewProvider.notifier);
    final t = Theme.of(context);
    final cur = v.current;
    final ci = v.currentIndex;
    final next = v.upNext;

    Widget header(String text, {Widget? trailing}) => Padding(
          padding: const EdgeInsets.fromLTRB(12, 18, 12, 8),
          child: Row(
            children: [
              Text(text.toUpperCase(),
                  style: t.textTheme.labelSmall?.copyWith(color: t.colorScheme.onSurfaceVariant)),
              const Spacer(),
              ?trailing,
            ],
          ),
        );

    if (cur == null && next.isEmpty) {
      return Center(
        child: Text('Очередь пуста',
            style: t.textTheme.bodyLarge?.copyWith(color: t.colorScheme.onSurfaceVariant)),
      );
    }

    return CustomScrollView(
      slivers: [
        SliverPadding(
          padding: EdgeInsets.only(left: padding.left, right: padding.right, top: padding.top),
          sliver: SliverList.list(children: [
            if (cur != null) ...[
              header('Сейчас играет'),
              QueueTile(track: cur, current: true, index: ci),
            ],
            header(
              v.waveActive ? 'Далее в волне' : 'Далее',
              trailing: Text(
                next.isEmpty ? '' : _count(next.length),
                style: t.textTheme.labelMedium?.copyWith(color: t.colorScheme.onSurfaceVariant),
              ),
            ),
          ]),
        ),
        SliverPadding(
          padding: EdgeInsets.only(left: padding.left, right: padding.right),
          sliver: SliverReorderableList(
            itemCount: next.length,
            onReorderItem: (from, to) {
              if (to == from) return;
              ctl.move(next[from].uid, ci + 1 + to);
            },
            proxyDecorator: (child, i, a) => Material(
              color: t.colorScheme.surface.withValues(alpha: 0.92),
              elevation: 8,
              shadowColor: Colors.black54,
              borderRadius: BorderRadius.circular(kRadiusM),
              child: child,
            ),
            itemBuilder: (context, i) => QueueTile(
              key: ValueKey(next[i].uid),
              track: next[i],
              index: ci + 1 + i,
              reorderIndex: i,
              onRemove: () => ctl.remove(next[i].uid),
            ),
          ),
        ),
        SliverPadding(
          padding: EdgeInsets.only(bottom: padding.bottom),
          sliver: SliverToBoxAdapter(
            child: v.continuation.fetching
                ? Padding(
                    padding: const EdgeInsets.all(20),
                    child: Row(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        SizedBox(
                          width: 14,
                          height: 14,
                          child: CircularProgressIndicator(
                              strokeWidth: 1.6, color: t.colorScheme.onSurfaceVariant),
                        ),
                        const SizedBox(width: 10),
                        Text('Подбираю следующие треки…',
                            style: t.textTheme.bodySmall
                                ?.copyWith(color: t.colorScheme.onSurfaceVariant)),
                      ],
                    ),
                  )
                : const SizedBox.shrink(),
          ),
        ),
      ],
    );
  }
}

String _count(int n) {
  final m10 = n % 10, m100 = n % 100;
  final w = m10 == 1 && m100 != 11
      ? 'трек'
      : m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)
          ? 'трека'
          : 'треков';
  return '$n $w';
}

class QueueTile extends ConsumerStatefulWidget {
  final PlayerTrack track;
  final bool current;
  final int index;

  /// Index inside the reorderable list; null for the current row.
  final int? reorderIndex;
  final VoidCallback? onRemove;

  const QueueTile({
    super.key,
    required this.track,
    required this.index,
    this.current = false,
    this.reorderIndex,
    this.onRemove,
  });

  @override
  ConsumerState<QueueTile> createState() => _QueueTileState();
}

class _QueueTileState extends ConsumerState<QueueTile> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    final tr = widget.track;
    final playing = widget.current && ref.watch(playerViewProvider.select((v) => v.playing));
    final dim = tr.unavailable;
    final row = MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: Material(
        color: widget.current
            ? t.colorScheme.primary.withValues(alpha: 0.12)
            : _hover
                ? t.colorScheme.onSurface.withValues(alpha: 0.05)
                : Colors.transparent,
        borderRadius: BorderRadius.circular(kRadiusM),
        child: InkWell(
          borderRadius: BorderRadius.circular(kRadiusM),
          onTap: widget.current || dim
              ? null
              : () => ref.read(playerViewProvider.notifier).playUid(tr.uid),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
            child: Opacity(
              opacity: dim ? 0.45 : 1,
              child: Row(
                children: [
                  Stack(
                    alignment: Alignment.center,
                    children: [
                      CoverArt(url: tr.cover, size: 44, radius: 6),
                      if (widget.current)
                        Container(
                          width: 44,
                          height: 44,
                          decoration: BoxDecoration(
                            color: Colors.black.withValues(alpha: 0.4),
                            borderRadius: BorderRadius.circular(6),
                          ),
                          child: Icon(
                            playing ? Icons.graphic_eq_rounded : Icons.pause_rounded,
                            color: Colors.white,
                            size: 20,
                          ),
                        ),
                    ],
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(tr.title,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: t.textTheme.titleSmall?.copyWith(
                                color: widget.current ? t.colorScheme.primary : null)),
                        const SizedBox(height: 2),
                        Text(tr.artistNames,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: t.textTheme.bodySmall
                                ?.copyWith(color: t.colorScheme.onSurfaceVariant)),
                      ],
                    ),
                  ),
                  const SizedBox(width: 8),
                  if (_hover && widget.onRemove != null)
                    IconButton(
                      tooltip: 'Убрать из очереди',
                      visualDensity: VisualDensity.compact,
                      iconSize: 18,
                      onPressed: widget.onRemove,
                      icon: Icon(Icons.close_rounded, color: t.colorScheme.onSurfaceVariant),
                    )
                  else
                    Text(formatDuration(tr.duration),
                        style: t.textTheme.labelMedium?.copyWith(
                          color: t.colorScheme.onSurfaceVariant,
                          fontFeatures: const [FontFeature.tabularFigures()],
                        )),
                  if (widget.reorderIndex != null)
                    ReorderableDragStartListener(
                      index: widget.reorderIndex!,
                      child: MouseRegion(
                        cursor: SystemMouseCursors.grab,
                        child: Padding(
                          padding: const EdgeInsets.only(left: 6),
                          child: Icon(Icons.drag_indicator_rounded,
                              size: 18,
                              color: t.colorScheme.onSurfaceVariant.withValues(alpha: _hover ? 1 : 0.35)),
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    return Padding(padding: const EdgeInsets.symmetric(vertical: 1), child: row);
  }
}
