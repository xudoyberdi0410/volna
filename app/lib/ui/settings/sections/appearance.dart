import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../state/theme.dart';
import '../widgets.dart';

/// Settings → «Оформление»: dark / light / follow the system.
class AppearanceSection extends ConsumerWidget {
  const AppearanceSection({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final mode = ref.watch(themeModeProvider);
    void set(ThemeMode m) => ref.read(themeModeProvider.notifier).set(m);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionHeader('Оформление'),
        SettingsCard(
          icon: Icons.palette_outlined,
          title: 'Тема',
          subtitle: 'Цвета плеера дополнительно подстраиваются под обложку',
          children: [
            LayoutBuilder(builder: (context, box) {
              final tiles = [
                _ThemeTile(
                    mode: ThemeMode.system, label: 'Как в системе', selected: mode == ThemeMode.system, onTap: set),
                _ThemeTile(mode: ThemeMode.light, label: 'Светлая', selected: mode == ThemeMode.light, onTap: set),
                _ThemeTile(mode: ThemeMode.dark, label: 'Тёмная', selected: mode == ThemeMode.dark, onTap: set),
              ];
              if (box.maxWidth < 420) {
                return Column(children: [
                  for (final t in tiles) Padding(padding: const EdgeInsets.only(bottom: 10), child: t),
                ]);
              }
              return Row(children: [
                for (final (i, t) in tiles.indexed) ...[
                  if (i > 0) const SizedBox(width: 12),
                  Expanded(child: t),
                ],
              ]);
            }),
          ],
        ),
      ],
    );
  }
}

class _ThemeTile extends StatelessWidget {
  final ThemeMode mode;
  final String label;
  final bool selected;
  final ValueChanged<ThemeMode> onTap;
  const _ThemeTile({required this.mode, required this.label, required this.selected, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    return InkWell(
      key: Key('theme-${mode.name}'),
      borderRadius: BorderRadius.circular(16),
      onTap: () => onTap(mode),
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 160),
        padding: const EdgeInsets.all(10),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(16),
          border: Border.all(color: selected ? c.primary : c.outlineVariant, width: selected ? 2 : 1),
          color: selected ? c.primaryContainer.withValues(alpha: 0.35) : null,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            AspectRatio(
              aspectRatio: 16 / 10,
              child: ClipRRect(
                borderRadius: BorderRadius.circular(10),
                child: switch (mode) {
                  ThemeMode.light => const _Preview(dark: false),
                  ThemeMode.dark => const _Preview(dark: true),
                  ThemeMode.system => const Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Expanded(child: _Preview(dark: false)),
                        Expanded(child: _Preview(dark: true)),
                      ],
                    ),
                },
              ),
            ),
            const SizedBox(height: 10),
            Row(
              children: [
                Icon(
                  selected ? Icons.radio_button_checked : Icons.radio_button_unchecked,
                  size: 18,
                  color: selected ? c.primary : c.onSurfaceVariant,
                ),
                const SizedBox(width: 8),
                Flexible(child: Text(label, style: Theme.of(context).textTheme.labelLarge)),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// A tiny mock of the window in one theme.
class _Preview extends StatelessWidget {
  final bool dark;
  const _Preview({required this.dark});

  @override
  Widget build(BuildContext context) {
    final seed = Theme.of(context).colorScheme.primary;
    final s = ColorScheme.fromSeed(seedColor: seed, brightness: dark ? Brightness.dark : Brightness.light);
    return Container(
      color: s.surface,
      padding: const EdgeInsets.all(8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Container(
            width: 14,
            decoration: BoxDecoration(color: s.surfaceContainerHigh, borderRadius: BorderRadius.circular(4)),
          ),
          const SizedBox(width: 6),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Container(
                  height: 6,
                  width: 40,
                  decoration: BoxDecoration(color: s.onSurface, borderRadius: BorderRadius.circular(3)),
                ),
                const SizedBox(height: 6),
                Expanded(
                  child: Row(
                    children: [
                      Expanded(
                        child: Container(
                          decoration:
                              BoxDecoration(color: s.primaryContainer, borderRadius: BorderRadius.circular(4)),
                        ),
                      ),
                      const SizedBox(width: 4),
                      Expanded(
                        child: Container(
                          decoration:
                              BoxDecoration(color: s.surfaceContainerHighest, borderRadius: BorderRadius.circular(4)),
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 6),
                Container(
                  height: 8,
                  decoration: BoxDecoration(color: s.primary, borderRadius: BorderRadius.circular(4)),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
