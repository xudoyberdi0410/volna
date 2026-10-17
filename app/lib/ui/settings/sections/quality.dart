import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../api/player.dart';
import '../../../api/player_models.dart';
import '../../../state/connection.dart';
import '../../../state/settings.dart';
import '../widgets.dart';

/// Settings → «Качество»: stream quality and playback options of the
/// daemon's player (`player_set_settings`).
class QualitySection extends ConsumerStatefulWidget {
  const QualitySection({super.key});

  @override
  ConsumerState<QualitySection> createState() => _QualitySectionState();
}

class _QualitySectionState extends ConsumerState<QualitySection> {
  PlayerSettings? _s;
  String? _loadError;
  String? _saveError;
  StreamSubscription<PlayerEvent>? _sub;
  double? _crossfadeDrag;

  PlayerApi get _api => ref.read(playerProvider);

  @override
  void initState() {
    super.initState();
    _load();
    // Changes from the tray or another window.
    _sub = _api.events.listen((e) {
      if (e is SettingsEvent && mounted) setState(() => _s = e.settings);
    });
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    setState(() => _loadError = null);
    try {
      final snap = await _api.snapshot();
      if (mounted) setState(() => _s = snap.settings);
    } catch (e) {
      if (mounted) setState(() => _loadError = errorText(e));
    }
  }

  Future<void> _patch(Map<String, Object?> patch) async {
    setState(() => _saveError = null);
    try {
      final s = await _api.setSettings(patch);
      if (mounted) setState(() => _s = s);
    } catch (e) {
      if (mounted) setState(() => _saveError = 'Не удалось сохранить: ${errorText(e)}');
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = _s;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionHeader('Качество и воспроизведение',
            subtitle: 'Звук играет демон Volna, поэтому настройки действуют и при закрытом окне.'),
        if (_loadError != null)
          SettingsCard(
              icon: Icons.high_quality_outlined,
              title: 'Качество звука',
              children: [LoadFailed(_loadError!, onRetry: _load)])
        else if (s == null)
          const Padding(padding: EdgeInsets.all(32), child: Center(child: CircularProgressIndicator()))
        else ...[
          _qualityCard(s),
          _playbackCard(s),
          if (_saveError != null) InlineMessage(Tone.bad, _saveError!),
        ],
      ],
    );
  }

  Widget _qualityCard(PlayerSettings s) {
    Widget option(String value, String title, String text, IconData icon) {
      final c = Theme.of(context).colorScheme;
      final selected = s.quality == value;
      return Expanded(
        child: InkWell(
          key: Key('quality-$value'),
          borderRadius: BorderRadius.circular(16),
          onTap: () => _patch({'quality': value}),
          child: AnimatedContainer(
            duration: const Duration(milliseconds: 160),
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(16),
              color: selected ? c.primaryContainer.withValues(alpha: 0.6) : c.surfaceContainer,
              border: Border.all(color: selected ? c.primary : c.outlineVariant, width: selected ? 2 : 1),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(children: [
                  Icon(icon, color: selected ? c.primary : c.onSurfaceVariant),
                  const Spacer(),
                  if (selected) Icon(Icons.check_circle, color: c.primary, size: 20),
                ]),
                const SizedBox(height: 10),
                Text(title, style: Theme.of(context).textTheme.titleSmall?.copyWith(fontWeight: FontWeight.w600)),
                const SizedBox(height: 4),
                Text(text, style: Theme.of(context).textTheme.bodySmall?.copyWith(color: c.onSurfaceVariant)),
              ],
            ),
          ),
        ),
      );
    }

    return SettingsCard(
      icon: Icons.high_quality_outlined,
      title: 'Качество звука',
      subtitle: 'Применяется к следующему треку',
      children: [
        IntrinsicHeight(
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              option('high', 'Высокое', 'Лучший поток YouTube, обычно Opus около 160 кбит/с', Icons.graphic_eq),
              const SizedBox(width: 12),
              option('low', 'Экономное', 'Меньше трафика: для медленного прокси или мобильного интернета',
                  Icons.data_saver_on_outlined),
            ],
          ),
        ),
      ],
    );
  }

  Widget _playbackCard(PlayerSettings s) {
    final seconds = _crossfadeDrag ?? s.crossfadeSeconds.clamp(1, 12).toDouble();
    return SettingsCard(
      icon: Icons.tune,
      title: 'Воспроизведение',
      children: [
        SwitchRow(
          title: 'Без пауз между треками',
          subtitle: 'Следующий трек готовится заранее и начинается сразу',
          value: s.gapless,
          onChanged: (v) => _patch({'gapless': v}),
        ),
        SwitchRow(
          title: 'Плавный переход',
          subtitle: 'Конец трека перетекает в начало следующего',
          value: s.crossfadeEnabled,
          onChanged: (v) => _patch({
            'crossfade': {'enabled': v, 'seconds': s.crossfadeSeconds}
          }),
        ),
        if (s.crossfadeEnabled)
          Row(
            children: [
              const SizedBox(width: 4),
              Text('${seconds.round()} с', style: Theme.of(context).textTheme.labelLarge),
              Expanded(
                child: Slider(
                  value: seconds,
                  min: 1,
                  max: 12,
                  divisions: 11,
                  label: '${seconds.round()} с',
                  onChanged: (v) => setState(() => _crossfadeDrag = v),
                  onChangeEnd: (v) {
                    _crossfadeDrag = null;
                    _patch({
                      'crossfade': {'enabled': true, 'seconds': v.roundToDouble()}
                    });
                  },
                ),
              ),
            ],
          ),
      ],
    );
  }
}
