import 'package:flutter/material.dart';

import '../nav.dart';

class NavDestination {
  final String path;
  final String label;
  final IconData icon;
  final IconData selectedIcon;
  const NavDestination(this.path, this.label, this.icon, this.selectedIcon);
}

const kMainDestinations = [
  NavDestination(Paths.wave, 'Волна', Icons.waves_rounded, Icons.waves_rounded),
  NavDestination(Paths.search, 'Поиск', Icons.search_rounded, Icons.search_rounded),
  NavDestination(Paths.library, 'Библиотека', Icons.library_music_outlined, Icons.library_music_rounded),
];

const kSettingsDestination =
    NavDestination(Paths.settings, 'Настройки', Icons.tune_rounded, Icons.tune_rounded);

/// Which destination owns [location] (the shell's current path).
String? destinationFor(String location) {
  if (location.startsWith(Paths.wave)) return Paths.wave;
  if (location.startsWith(Paths.search)) return Paths.search;
  if (location.startsWith(Paths.library) || location.startsWith(Paths.history)) {
    return Paths.library;
  }
  if (location.startsWith(Paths.settings)) return Paths.settings;
  return null;
}

class Sidebar extends StatelessWidget {
  final String location;
  final bool compact;
  final ValueChanged<String> onSelect;

  const Sidebar({super.key, required this.location, required this.onSelect, this.compact = false});

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    final selected = destinationFor(location);
    return AnimatedContainer(
      duration: const Duration(milliseconds: 260),
      curve: Curves.easeOutCubic,
      width: compact ? 76 : 228,
      decoration: BoxDecoration(
        color: t.colorScheme.onSurface.withValues(alpha: t.brightness == Brightness.dark ? 0.025 : 0.03),
        border: Border(right: BorderSide(color: t.colorScheme.onSurface.withValues(alpha: 0.06))),
      ),
      child: Padding(
        padding: EdgeInsets.fromLTRB(compact ? 10 : 14, 22, compact ? 10 : 10, 14),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: EdgeInsets.only(left: compact ? 0 : 10, bottom: 26),
              child: Row(
                mainAxisAlignment: compact ? MainAxisAlignment.center : MainAxisAlignment.start,
                children: [
                  const _Logo(size: 30),
                  if (!compact) ...[
                    const SizedBox(width: 11),
                    Text('Волна',
                        style: t.textTheme.titleLarge?.copyWith(
                            fontWeight: FontWeight.w700, letterSpacing: -0.6)),
                  ],
                ],
              ),
            ),
            for (final d in kMainDestinations)
              _NavItem(d: d, selected: selected == d.path, compact: compact, onTap: () => onSelect(d.path)),
            const Spacer(),
            _NavItem(
              d: kSettingsDestination,
              selected: selected == Paths.settings,
              compact: compact,
              onTap: () => onSelect(Paths.settings),
            ),
          ],
        ),
      ),
    );
  }
}

class _NavItem extends StatefulWidget {
  final NavDestination d;
  final bool selected;
  final bool compact;
  final VoidCallback onTap;
  const _NavItem({required this.d, required this.selected, required this.compact, required this.onTap});

  @override
  State<_NavItem> createState() => _NavItemState();
}

class _NavItemState extends State<_NavItem> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    final c = t.colorScheme;
    final sel = widget.selected;
    final fg = sel ? c.onSurface : c.onSurface.withValues(alpha: _hover ? 0.9 : 0.66);
    final item = MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        onTap: widget.onTap,
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 180),
          curve: Curves.easeOut,
          height: 42,
          margin: const EdgeInsets.only(bottom: 2),
          padding: EdgeInsets.symmetric(horizontal: widget.compact ? 0 : 12),
          decoration: BoxDecoration(
            color: sel
                ? c.onSurface.withValues(alpha: 0.09)
                : _hover
                    ? c.onSurface.withValues(alpha: 0.045)
                    : Colors.transparent,
            borderRadius: BorderRadius.circular(11),
          ),
          child: Row(
            mainAxisAlignment: widget.compact ? MainAxisAlignment.center : MainAxisAlignment.start,
            children: [
              Icon(sel ? widget.d.selectedIcon : widget.d.icon,
                  size: 21, color: sel ? c.primary : fg),
              if (!widget.compact) ...[
                const SizedBox(width: 13),
                Text(widget.d.label,
                    style: t.textTheme.titleSmall?.copyWith(
                      color: fg,
                      fontWeight: sel ? FontWeight.w600 : FontWeight.w500,
                    )),
              ],
            ],
          ),
        ),
      ),
    );
    return Semantics(
      selected: sel,
      button: true,
      label: widget.d.label,
      child: widget.compact ? Tooltip(message: widget.d.label, child: item) : item,
    );
  }
}

/// The wordmark glyph: three stacked waves in the accent colour.
class _Logo extends StatelessWidget {
  final double size;
  const _Logo({required this.size});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(size * 0.3),
        gradient: LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [c.primary, Color.lerp(c.primary, c.onSurface, 0.35)!],
        ),
      ),
      child: Icon(Icons.waves_rounded, size: size * 0.62, color: c.onPrimary),
    );
  }
}
