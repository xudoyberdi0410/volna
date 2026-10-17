import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import 'sections/about.dart';
import 'sections/appearance.dart';
import 'sections/library.dart';
import 'sections/network.dart';
import 'sections/quality.dart';
import 'sections/scrobbling.dart';
import 'sections/wave.dart';
import 'sections/yandex.dart';
import 'sections/ytdlp.dart';

/// One entry of the settings menu; [id] is the `/settings/:section` path part.
class SettingsSectionInfo {
  final String id;
  final String title;
  final IconData icon;
  final Widget Function() build;
  const SettingsSectionInfo(this.id, this.title, this.icon, this.build);
}

final settingsSections = <SettingsSectionInfo>[
  SettingsSectionInfo('network', 'Сеть', Icons.lan_outlined, () => const NetworkSection()),
  SettingsSectionInfo('wave', 'Моя волна', Icons.waves, () => const WaveSection()),
  SettingsSectionInfo('quality', 'Качество', Icons.high_quality_outlined, () => const QualitySection()),
  SettingsSectionInfo('appearance', 'Оформление', Icons.palette_outlined, () => const AppearanceSection()),
  SettingsSectionInfo('scrobbling', 'Скробблинг', Icons.history, () => const ScrobblingSection()),
  SettingsSectionInfo('yandex', 'Яндекс Музыка', Icons.move_down, () => const YandexSection()),
  SettingsSectionInfo('library', 'Библиотека', Icons.library_music_outlined, () => const LibrarySection()),
  SettingsSectionInfo('ytdlp', 'yt-dlp', Icons.download_for_offline_outlined, () => const YtDlpSection()),
  SettingsSectionInfo('about', 'О программе', Icons.info_outline, () => const AboutSection()),
];

SettingsSectionInfo? sectionById(String? id) => settingsSections.where((s) => s.id == id).firstOrNull;

/// Wide: the section menu on the left, the section on the right.
/// Narrow: the menu alone at `/settings`, a section with a back arrow at
/// `/settings/:section`.
class SettingsScreen extends StatelessWidget {
  /// Null: `/settings` (the first section on wide windows).
  final String? section;
  const SettingsScreen({super.key, this.section});

  static const wideBreakpoint = 760.0;

  void _go(BuildContext context, String id) {
    final router = GoRouter.maybeOf(context);
    if (router != null) {
      router.go('/settings/$id');
    }
  }

  @override
  Widget build(BuildContext context) {
    // Works inside the app shell's Scaffold and on its own.
    return Material(type: MaterialType.transparency, child: LayoutBuilder(builder: (context, box) {
      final wide = box.maxWidth >= wideBreakpoint;
      final current = sectionById(section) ?? (wide ? settingsSections.first : null);
      if (!wide) {
        if (current == null) return _Menu(selected: null, onSelect: (id) => _go(context, id), standalone: true);
        return _SectionPage(info: current, onBack: () => GoRouter.maybeOf(context)?.go('/settings'));
      }
      return Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(width: 248, child: _Menu(selected: current!.id, onSelect: (id) => _go(context, id))),
          VerticalDivider(width: 1, color: Theme.of(context).colorScheme.outlineVariant.withValues(alpha: 0.5)),
          Expanded(child: _SectionPage(info: current)),
        ],
      );
    }));
  }
}

class _Menu extends StatelessWidget {
  final String? selected;
  final ValueChanged<String> onSelect;
  final bool standalone;
  const _Menu({required this.selected, required this.onSelect, this.standalone = false});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    return ListView(
      padding: const EdgeInsets.fromLTRB(12, 20, 12, 20),
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 0, 12, 16),
          child: Text('Настройки', style: (standalone ? t.headlineSmall : t.titleLarge)?.copyWith(fontWeight: FontWeight.w700)),
        ),
        for (final s in settingsSections)
          Padding(
            padding: const EdgeInsets.only(bottom: 2),
            child: ListTile(
              key: Key('settings-menu-${s.id}'),
              dense: true,
              visualDensity: VisualDensity.compact,
              shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
              selected: s.id == selected,
              selectedTileColor: c.secondaryContainer,
              selectedColor: c.onSecondaryContainer,
              leading: Icon(s.icon, size: 20),
              title: Text(s.title, style: t.bodyLarge?.copyWith(
                  color: s.id == selected ? c.onSecondaryContainer : null,
                  fontWeight: s.id == selected ? FontWeight.w600 : null)),
              trailing: standalone ? const Icon(Icons.chevron_right) : null,
              onTap: () => onSelect(s.id),
            ),
          ),
      ],
    );
  }
}

class _SectionPage extends StatelessWidget {
  final SettingsSectionInfo info;
  final VoidCallback? onBack;
  const _SectionPage({required this.info, this.onBack});

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      key: PageStorageKey('settings-${info.id}'),
      padding: const EdgeInsets.fromLTRB(28, 24, 28, 40),
      child: Align(
        alignment: Alignment.topLeft,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 760),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (onBack != null)
                Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton.icon(
                    onPressed: onBack,
                    icon: const Icon(Icons.arrow_back),
                    label: const Text('Настройки'),
                  ),
                ),
              KeyedSubtree(key: ValueKey(info.id), child: info.build()),
            ],
          ),
        ),
      ),
    );
  }
}
