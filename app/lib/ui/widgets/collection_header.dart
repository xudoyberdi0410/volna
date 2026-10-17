/// The top of an album / playlist / artist page: big cover, kind, title,
/// meta line, description and actions. Side by side when wide, stacked when narrow.
library;

import 'package:flutter/material.dart';

class CollectionHeader extends StatelessWidget {
  final Widget Function(double size) cover;
  final String? kind;
  final String title;

  /// The meta line (artists, year, track count…), usually [ArtistLinks] + text.
  final Widget? meta;
  final String? description;
  final Widget? actions;

  /// A round cover (artists): the shadow follows it.
  final bool round;

  const CollectionHeader({
    super.key,
    required this.cover,
    this.round = false,
    this.kind,
    required this.title,
    this.meta,
    this.description,
    this.actions,
  });

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    final cs = Theme.of(context).colorScheme;
    return LayoutBuilder(builder: (context, c) {
      final narrow = c.maxWidth < 640;
      final coverSize = narrow ? (c.maxWidth * 0.62).clamp(160.0, 260.0) : (c.maxWidth < 1000 ? 196.0 : 232.0);
      final align = narrow ? CrossAxisAlignment.center : CrossAxisAlignment.start;
      final textAlign = narrow ? TextAlign.center : TextAlign.start;
      final titleStyle = (narrow || title.length > 40 ? t.headlineMedium : t.displaySmall)
          ?.copyWith(fontWeight: FontWeight.w800, letterSpacing: -0.8, height: 1.1);
      final text = Column(
        crossAxisAlignment: align,
        mainAxisSize: MainAxisSize.min,
        children: [
          if (kind != null)
            Text(kind!.toUpperCase(),
                style: t.labelMedium?.copyWith(color: cs.onSurfaceVariant, letterSpacing: 1.2, fontWeight: FontWeight.w600)),
          const SizedBox(height: 6),
          Text(title, style: titleStyle, textAlign: textAlign, maxLines: 3, overflow: TextOverflow.ellipsis),
          if (meta != null) ...[
            const SizedBox(height: 10),
            DefaultTextStyle.merge(
              style: t.bodyMedium?.copyWith(color: cs.onSurfaceVariant),
              textAlign: textAlign,
              child: meta!,
            ),
          ],
          if (description != null && description!.trim().isNotEmpty) ...[
            const SizedBox(height: 10),
            _Description(text: description!.trim(), textAlign: textAlign),
          ],
          if (actions != null) ...[const SizedBox(height: 18), actions!],
        ],
      );
      final shadowed = DecoratedBox(
        decoration: BoxDecoration(
          shape: round ? BoxShape.circle : BoxShape.rectangle,
          borderRadius: round ? null : BorderRadius.circular(12),
          boxShadow: [BoxShadow(color: Colors.black.withValues(alpha: 0.28), blurRadius: 28, offset: const Offset(0, 10))],
        ),
        child: cover(coverSize),
      );
      return Container(
        decoration: BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: [cs.primary.withValues(alpha: 0.10), cs.surface.withValues(alpha: 0)],
          ),
        ),
        padding: EdgeInsets.fromLTRB(narrow ? 16 : 24, narrow ? 16 : 32, narrow ? 16 : 24, 20),
        child: narrow
            ? Column(children: [shadowed, const SizedBox(height: 20), text])
            : Row(crossAxisAlignment: CrossAxisAlignment.end, children: [
                shadowed,
                const SizedBox(width: 28),
                Expanded(child: text),
              ]),
      );
    });
  }
}

class _Description extends StatefulWidget {
  final String text;
  final TextAlign textAlign;
  const _Description({required this.text, required this.textAlign});

  @override
  State<_Description> createState() => _DescriptionState();
}

class _DescriptionState extends State<_Description> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    final cs = Theme.of(context).colorScheme;
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 720),
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          onTap: () => setState(() => _open = !_open),
          child: AnimatedSize(
            duration: const Duration(milliseconds: 200),
            alignment: Alignment.topCenter,
            child: Text(
              widget.text,
              maxLines: _open ? null : 2,
              overflow: _open ? null : TextOverflow.ellipsis,
              textAlign: widget.textAlign,
              style: t.bodyMedium?.copyWith(color: cs.onSurfaceVariant, height: 1.4),
            ),
          ),
        ),
      ),
    );
  }
}
