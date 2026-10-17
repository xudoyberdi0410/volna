/// Smooth playback position: the daemon reports every 250 ms, widgets here
/// extrapolate per frame while playing and stop ticking when paused.
library;

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../state/player.dart';

class PositionBuilder extends ConsumerStatefulWidget {
  /// `(position, duration)` in seconds; duration null when unknown.
  final Widget Function(BuildContext context, double position, double? duration) builder;

  const PositionBuilder({super.key, required this.builder});

  @override
  ConsumerState<PositionBuilder> createState() => _PositionBuilderState();
}

class _PositionBuilderState extends ConsumerState<PositionBuilder>
    with SingleTickerProviderStateMixin {
  late final Ticker _ticker = createTicker((_) => setState(() {}));

  @override
  void dispose() {
    _ticker.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final sample = ref.watch(positionProvider);
    final duration = sample.duration ??
        ref.watch(playerViewProvider.select((v) => v.state.duration ?? v.current?.duration));
    final animate = sample.playing; // The mixin mutes it under TickerMode(false).
    if (animate && !_ticker.isActive) {
      _ticker.start();
    } else if (!animate && _ticker.isActive) {
      _ticker.stop();
    }
    final now = ref.read(clockProvider)();
    return widget.builder(context, sample.positionAt(now), duration);
  }
}
