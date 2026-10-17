/// Loading skeletons, error and empty states.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../ipc/transport.dart';

/// User-facing Russian text for a failed command.
String errorText(Object error) {
  if (error is DaemonException) {
    return switch (error.code) {
      DaemonException.unavailable => 'Нет связи с фоновой службой Volna',
      'not_found' => 'Ничего не найдено',
      'upstream' => 'YouTube Music не ответил. Проверьте сеть и прокси в настройках',
      'bad_args' => 'Неверный запрос',
      _ => 'Что-то пошло не так',
    };
  }
  return 'Что-то пошло не так';
}

/// Technical detail under the message (for bug reports).
String? errorDetail(Object error) => error is DaemonException ? error.message : error.toString();

class ErrorView extends StatelessWidget {
  final Object error;
  final VoidCallback? onRetry;
  final bool compact;
  const ErrorView({super.key, required this.error, this.onRetry, this.compact = false});

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    final cs = Theme.of(context).colorScheme;
    final detail = errorDetail(error);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 420),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (!compact) ...[
                Icon(Icons.cloud_off_rounded, size: 48, color: cs.onSurfaceVariant),
                const SizedBox(height: 16),
              ],
              Text(errorText(error), style: t.titleMedium, textAlign: TextAlign.center),
              if (detail != null && detail.isNotEmpty) ...[
                const SizedBox(height: 6),
                Text(
                  detail,
                  style: t.bodySmall?.copyWith(color: cs.onSurfaceVariant),
                  textAlign: TextAlign.center,
                  maxLines: 3,
                  overflow: TextOverflow.ellipsis,
                ),
              ],
              if (onRetry != null) ...[
                const SizedBox(height: 16),
                FilledButton.tonalIcon(
                  onPressed: onRetry,
                  icon: const Icon(Icons.refresh_rounded),
                  label: const Text('Повторить'),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class EmptyView extends StatelessWidget {
  final IconData icon;
  final String title;
  final String? subtitle;
  final Widget? action;
  const EmptyView({super.key, required this.icon, required this.title, this.subtitle, this.action});

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    final cs = Theme.of(context).colorScheme;
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 380),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                width: 72,
                height: 72,
                decoration: BoxDecoration(color: cs.surfaceContainerHighest, shape: BoxShape.circle),
                child: Icon(icon, size: 34, color: cs.onSurfaceVariant),
              ),
              const SizedBox(height: 16),
              Text(title, style: t.titleMedium, textAlign: TextAlign.center),
              if (subtitle != null) ...[
                const SizedBox(height: 6),
                Text(subtitle!,
                    style: t.bodyMedium?.copyWith(color: cs.onSurfaceVariant),
                    textAlign: TextAlign.center),
              ],
              if (action != null) ...[const SizedBox(height: 16), action!],
            ],
          ),
        ),
      ),
    );
  }
}

/// A gently pulsing block; children of [Skeleton] share one animation.
class Skeleton extends StatefulWidget {
  final Widget child;
  const Skeleton({super.key, required this.child});

  @override
  State<Skeleton> createState() => _SkeletonState();
}

class _SkeletonState extends State<Skeleton> with SingleTickerProviderStateMixin {
  late final _c = AnimationController(vsync: this, duration: const Duration(milliseconds: 1100))
    ..repeat(reverse: true);

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => FadeTransition(
        opacity: Tween(begin: 0.55, end: 1.0).animate(CurvedAnimation(parent: _c, curve: Curves.easeInOut)),
        child: widget.child,
      );
}

class SkeletonBox extends StatelessWidget {
  final double? width;
  final double height;
  final double radius;
  const SkeletonBox({super.key, this.width, required this.height, this.radius = 6});

  @override
  Widget build(BuildContext context) => Container(
        width: width,
        height: height,
        decoration: BoxDecoration(
          color: Theme.of(context).colorScheme.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(radius),
        ),
      );
}

/// Placeholder rows shaped like track rows.
class TrackListSkeleton extends StatelessWidget {
  final int count;
  const TrackListSkeleton({super.key, this.count = 8});

  @override
  Widget build(BuildContext context) => Skeleton(
        child: Column(
          children: [
            for (var i = 0; i < count; i++)
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
                child: Row(children: [
                  const SkeletonBox(width: 44, height: 44),
                  const SizedBox(width: 14),
                  Expanded(
                    child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                      FractionallySizedBox(widthFactor: 0.5 + (i * 37 % 30) / 100, child: const SkeletonBox(height: 12)),
                      const SizedBox(height: 8),
                      FractionallySizedBox(widthFactor: 0.25 + (i * 17 % 20) / 100, child: const SkeletonBox(height: 10)),
                    ]),
                  ),
                  const SizedBox(width: 24),
                  const SkeletonBox(width: 36, height: 10),
                ]),
              ),
          ],
        ),
      );
}

/// A page skeleton: header block and a few rows.
class PageSkeleton extends StatelessWidget {
  final bool round;
  const PageSkeleton({super.key, this.round = false});

  @override
  Widget build(BuildContext context) => SingleChildScrollView(
        physics: const NeverScrollableScrollPhysics(),
        child: Column(children: [
          Skeleton(
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Row(children: [
                SkeletonBox(width: 180, height: 180, radius: round ? 90 : 12),
                const SizedBox(width: 24),
                const Expanded(
                  child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                    SkeletonBox(width: 80, height: 12),
                    SizedBox(height: 14),
                    SkeletonBox(width: 280, height: 28),
                    SizedBox(height: 14),
                    SkeletonBox(width: 180, height: 14),
                  ]),
                ),
              ]),
            ),
          ),
          const TrackListSkeleton(count: 6),
        ]),
      );
}

/// Renders an [AsyncValue]: [data] once loaded, keeping old data while
/// refreshing; [loading] and an [ErrorView] with retry otherwise.
class AsyncBody<T> extends StatelessWidget {
  final AsyncValue<T> value;
  final Widget Function(T data) data;
  final Widget loading;
  final VoidCallback? onRetry;
  const AsyncBody({
    super.key,
    required this.value,
    required this.data,
    this.loading = const PageSkeleton(),
    this.onRetry,
  });

  @override
  Widget build(BuildContext context) => switch (value) {
        AsyncValue(:final value?, hasValue: true) => data(value),
        AsyncError(:final error) => ErrorView(error: error, onRetry: onRetry),
        _ => loading,
      };
}
