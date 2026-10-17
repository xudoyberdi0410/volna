/// Building blocks shared by the settings sections: grouped cards, status
/// pills, inline messages, key/value rows.
library;

import 'package:flutter/material.dart';

/// How a status reads: fine, worth a look, broken, or just information.
enum Tone { good, warn, bad, neutral, busy }

extension ToneColors on Tone {
  Color color(ColorScheme c) => switch (this) {
    Tone.good => c.brightness == Brightness.dark ? const Color(0xFF7BD88F) : const Color(0xFF1E8E3E),
    Tone.warn => c.brightness == Brightness.dark ? const Color(0xFFF2C14E) : const Color(0xFFB26A00),
    Tone.bad => c.error,
    Tone.neutral => c.onSurfaceVariant,
    Tone.busy => c.primary,
  };

  IconData get icon => switch (this) {
    Tone.good => Icons.check_circle_rounded,
    Tone.warn => Icons.warning_amber_rounded,
    Tone.bad => Icons.error_rounded,
    Tone.neutral => Icons.info_outline_rounded,
    Tone.busy => Icons.more_horiz_rounded,
  };
}

/// The page title of a section.
class SectionHeader extends StatelessWidget {
  final String title;
  final String? subtitle;
  const SectionHeader(this.title, {super.key, this.subtitle});

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    return Padding(
      padding: const EdgeInsets.fromLTRB(4, 4, 4, 20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(title, style: t.headlineSmall?.copyWith(fontWeight: FontWeight.w600)),
          if (subtitle != null) ...[
            const SizedBox(height: 6),
            Text(
              subtitle!,
              style: t.bodyMedium?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant),
            ),
          ],
        ],
      ),
    );
  }
}

/// A group of related settings with an icon, a title and optional trailing
/// status.
class SettingsCard extends StatelessWidget {
  final IconData? icon;
  final String title;
  final String? subtitle;
  final Widget? trailing;
  final List<Widget> children;
  final EdgeInsetsGeometry padding;
  const SettingsCard({
    super.key,
    this.icon,
    required this.title,
    this.subtitle,
    this.trailing,
    this.children = const [],
    this.padding = const EdgeInsets.fromLTRB(20, 18, 20, 20),
  });

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    return Padding(
      padding: const EdgeInsets.only(bottom: 16),
      // A Material, so list tiles and ink splashes inside paint on it.
      child: Material(
        color: c.surfaceContainerLow,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(20),
          side: BorderSide(color: c.outlineVariant.withValues(alpha: 0.5)),
        ),
        child: Padding(
          padding: padding,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                crossAxisAlignment: CrossAxisAlignment.center,
                children: [
                  if (icon != null) ...[
                    Container(
                      width: 36,
                      height: 36,
                      decoration: BoxDecoration(
                        color: c.primaryContainer,
                        borderRadius: BorderRadius.circular(10),
                      ),
                      child: Icon(icon, size: 20, color: c.onPrimaryContainer),
                    ),
                    const SizedBox(width: 14),
                  ],
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(title, style: t.titleMedium?.copyWith(fontWeight: FontWeight.w600)),
                        if (subtitle != null) ...[
                          const SizedBox(height: 2),
                          Text(subtitle!, style: t.bodySmall?.copyWith(color: c.onSurfaceVariant)),
                        ],
                      ],
                    ),
                  ),
                  if (trailing != null) ...[const SizedBox(width: 12), trailing!],
                ],
              ),
              if (children.isNotEmpty) const SizedBox(height: 16),
              ...children,
            ],
          ),
        ),
      ),
    );
  }
}

/// A rounded badge: a colored dot or icon and a short text.
class StatusPill extends StatelessWidget {
  final Tone tone;
  final String text;
  const StatusPill(this.tone, this.text, {super.key});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    final color = tone.color(c);
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.12),
        borderRadius: BorderRadius.circular(999),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (tone == Tone.busy)
            SizedBox(width: 10, height: 10, child: CircularProgressIndicator(strokeWidth: 2, color: color))
          else
            Container(
              width: 8,
              height: 8,
              decoration: BoxDecoration(color: color, shape: BoxShape.circle),
            ),
          const SizedBox(width: 7),
          Text(
            text,
            style: Theme.of(context).textTheme.labelMedium
                ?.copyWith(color: color, fontWeight: FontWeight.w600),
          ),
        ],
      ),
    );
  }
}

/// A one- or multi-line message under a form: what happened and how it went.
class InlineMessage extends StatelessWidget {
  final Tone tone;
  final String text;
  const InlineMessage(this.tone, this.text, {super.key});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    final color = tone.color(c);
    return Container(
      margin: const EdgeInsets.only(top: 12),
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.08),
        borderRadius: BorderRadius.circular(12),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (tone == Tone.busy)
            Padding(
              padding: const EdgeInsets.only(top: 2),
              child: SizedBox(
                width: 16,
                height: 16,
                child: CircularProgressIndicator(strokeWidth: 2, color: color),
              ),
            )
          else
            Icon(tone.icon, size: 18, color: color),
          const SizedBox(width: 10),
          Expanded(child: SelectableText(text, style: Theme.of(context).textTheme.bodyMedium)),
        ],
      ),
    );
  }
}

/// Label / value rows, e.g. the network check.
class KeyValueTable extends StatelessWidget {
  final List<(String, String, Tone?)> rows;
  const KeyValueTable(this.rows, {super.key});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    return Column(
      children: [
        for (final (i, (k, v, tone)) in rows.indexed)
          Container(
            padding: const EdgeInsets.symmetric(vertical: 9),
            decoration: BoxDecoration(
              border: i == 0 ? null : Border(top: BorderSide(color: c.outlineVariant.withValues(alpha: 0.5))),
            ),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  width: 140,
                  child: Text(k, style: t.bodyMedium?.copyWith(color: c.onSurfaceVariant)),
                ),
                if (tone != null) ...[
                  Padding(
                    padding: const EdgeInsets.only(top: 2, right: 8),
                    child: Icon(tone.icon, size: 16, color: tone.color(c)),
                  ),
                ],
                Expanded(child: SelectableText(v, style: t.bodyMedium)),
              ],
            ),
          ),
      ],
    );
  }
}

/// A switch row inside a card.
class SwitchRow extends StatelessWidget {
  final String title;
  final String? subtitle;
  final bool value;
  final ValueChanged<bool>? onChanged;
  const SwitchRow({super.key, required this.title, this.subtitle, required this.value, this.onChanged});

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context).textTheme;
    final c = Theme.of(context).colorScheme;
    return InkWell(
      borderRadius: BorderRadius.circular(12),
      onTap: onChanged == null ? null : () => onChanged!(!value),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 8),
        child: Row(
          children: [
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(title, style: t.bodyLarge),
                  if (subtitle != null)
                    Text(subtitle!, style: t.bodySmall?.copyWith(color: c.onSurfaceVariant)),
                ],
              ),
            ),
            const SizedBox(width: 12),
            Switch(value: value, onChanged: onChanged),
          ],
        ),
      ),
    );
  }
}

/// A labelled field block: label above, hint below.
class FieldLabel extends StatelessWidget {
  final String text;
  const FieldLabel(this.text, {super.key});

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 8, left: 2),
    child: Text(
      text,
      style: Theme.of(context).textTheme.labelLarge
          ?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant),
    ),
  );
}

/// Buttons in a row that wraps on narrow widths.
class ButtonRow extends StatelessWidget {
  final List<Widget> children;
  const ButtonRow(this.children, {super.key});

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 16),
    child: Wrap(spacing: 10, runSpacing: 10, children: children),
  );
}

/// A small spinner for buttons that are working.
class ButtonSpinner extends StatelessWidget {
  const ButtonSpinner({super.key});

  @override
  Widget build(BuildContext context) =>
      const SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2));
}

/// Loading / error placeholder for a card that fetches its data.
class LoadFailed extends StatelessWidget {
  final String message;
  final VoidCallback onRetry;
  const LoadFailed(this.message, {super.key, required this.onRetry});

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      InlineMessage(Tone.bad, message),
      const SizedBox(height: 8),
      TextButton.icon(onPressed: onRetry, icon: const Icon(Icons.refresh), label: const Text('Повторить')),
    ],
  );
}

const inputDecorationRadius = BorderRadius.all(Radius.circular(12));

InputDecoration fieldDecoration({
  String? hint,
  String? helper,
  String? error,
  Widget? suffix,
  String? label,
}) => InputDecoration(
  hintText: hint,
  helperText: helper,
  helperMaxLines: 3,
  errorText: error,
  errorMaxLines: 3,
  labelText: label,
  suffixIcon: suffix,
  isDense: true,
  filled: true,
  border: const OutlineInputBorder(borderRadius: inputDecorationRadius),
);
