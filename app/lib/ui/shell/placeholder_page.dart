import 'package:flutter/material.dart';

/// A screen that is not built yet. Used by the route stubs until the real
/// pages land.
class PlaceholderPage extends StatelessWidget {
  final String title;
  final String? subtitle;
  final IconData icon;

  const PlaceholderPage({
    super.key,
    required this.title,
    this.subtitle,
    this.icon = Icons.construction_rounded,
  });

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: 40, color: t.colorScheme.onSurfaceVariant),
          const SizedBox(height: 16),
          Text(title, style: t.textTheme.titleLarge),
          if (subtitle != null) ...[
            const SizedBox(height: 6),
            Text(subtitle!,
                style: t.textTheme.bodyMedium?.copyWith(color: t.colorScheme.onSurfaceVariant)),
          ],
        ],
      ),
    );
  }
}
