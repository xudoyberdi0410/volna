import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import '../../api/player_models.dart';
import '../nav.dart';

/// Title over linked artist names; slides in when the track changes.
class TrackTitle extends StatelessWidget {
  final PlayerTrack track;

  /// Mini player sizes; otherwise [titleStyle]/[artistStyle] or large defaults.
  final bool dense;
  final TextStyle? titleStyle;
  final TextStyle? artistStyle;
  final CrossAxisAlignment align;
  final int titleLines;

  const TrackTitle({
    super.key,
    required this.track,
    this.dense = false,
    this.titleStyle,
    this.artistStyle,
    this.align = CrossAxisAlignment.start,
    this.titleLines = 1,
  });

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    final ts = titleStyle ?? (dense ? t.textTheme.titleSmall : t.textTheme.headlineSmall);
    final as = (artistStyle ?? (dense ? t.textTheme.bodySmall : t.textTheme.bodyLarge))
        ?.copyWith(color: t.colorScheme.onSurfaceVariant);
    final textAlign = switch (align) {
      CrossAxisAlignment.center => TextAlign.center,
      CrossAxisAlignment.end => TextAlign.end,
      _ => TextAlign.start,
    };
    return AnimatedSwitcher(
      duration: const Duration(milliseconds: 360),
      switchInCurve: Curves.easeOutCubic,
      switchOutCurve: Curves.easeInCubic,
      transitionBuilder: (child, a) => FadeTransition(
        opacity: a,
        child: SlideTransition(
          position: Tween(begin: const Offset(0, 0.25), end: Offset.zero).animate(a),
          child: child,
        ),
      ),
      layoutBuilder: (current, previous) => Stack(
        alignment: switch (align) {
          CrossAxisAlignment.center => Alignment.center,
          CrossAxisAlignment.end => Alignment.centerRight,
          _ => Alignment.centerLeft,
        },
        children: [...previous, ?current],
      ),
      child: Column(
        key: ValueKey(track.uid),
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: align,
        children: [
          Text.rich(
            TextSpan(children: [
              TextSpan(text: track.title),
              if (track.explicit)
                WidgetSpan(
                  alignment: PlaceholderAlignment.middle,
                  child: Padding(
                    padding: const EdgeInsets.only(left: 6),
                    child: _ExplicitBadge(color: t.colorScheme.onSurfaceVariant),
                  ),
                ),
            ]),
            style: ts,
            maxLines: titleLines,
            overflow: TextOverflow.ellipsis,
            textAlign: textAlign,
          ),
          SizedBox(height: dense ? 2 : 4),
          _Artists(track: track, style: as, align: textAlign),
        ],
      ),
    );
  }
}

class _Artists extends StatelessWidget {
  final PlayerTrack track;
  final TextStyle? style;
  final TextAlign align;
  const _Artists({required this.track, required this.style, required this.align});

  @override
  Widget build(BuildContext context) {
    final spans = <InlineSpan>[];
    for (final (i, a) in track.artists.indexed) {
      if (i > 0) spans.add(const TextSpan(text: ', '));
      final id = a.id;
      spans.add(TextSpan(
        text: a.name,
        mouseCursor: id == null ? null : SystemMouseCursors.click,
        recognizer: id == null ? null : (TapGestureRecognizer()..onTap = () => openArtist(context, id)),
      ));
    }
    if (spans.isEmpty) spans.add(const TextSpan(text: 'Неизвестный исполнитель'));
    return Text.rich(
      TextSpan(children: spans),
      style: style,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      textAlign: align,
    );
  }
}

class _ExplicitBadge extends StatelessWidget {
  final Color color;
  const _ExplicitBadge({required this.color});

  @override
  Widget build(BuildContext context) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 3.5, vertical: 0.5),
        decoration: BoxDecoration(
          color: color.withValues(alpha: 0.22),
          borderRadius: BorderRadius.circular(3),
        ),
        child: Text('E',
            style: TextStyle(
                fontSize: 9.5, fontWeight: FontWeight.w700, color: color, height: 1.2)),
      );
}
