import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../api/network.dart';
import '../../../state/settings.dart';
import '../network_text.dart';
import '../widgets.dart';

/// Settings → «yt-dlp»: which binary resolves audio, its version, update.
class YtDlpSection extends ConsumerStatefulWidget {
  const YtDlpSection({super.key});

  @override
  ConsumerState<YtDlpSection> createState() => _YtDlpSectionState();
}

class _YtDlpSectionState extends ConsumerState<YtDlpSection> {
  YtDlpInfo? _info;
  String? _error;
  bool _loading = true;
  bool _updating = false;
  YtDlpUpdate? _update;
  String? _updateError;
  bool _showOutput = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final i = await ref.read(networkApiProvider).ytdlpVersion();
      if (mounted) setState(() => _info = i);
    } catch (e) {
      if (mounted) setState(() => _error = errorText(e));
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  Future<void> _runUpdate() async {
    setState(() {
      _updating = true;
      _update = null;
      _updateError = null;
    });
    try {
      final r = await ref.read(networkApiProvider).ytdlpUpdate();
      if (!mounted) return;
      setState(() {
        _update = r;
        if (_info != null) _info = YtDlpInfo(_info!.binary, r.origin, r.versionAfter);
      });
    } catch (e) {
      if (mounted) setState(() => _updateError = 'Не удалось обновить: ${errorText(e)}');
    } finally {
      if (mounted) setState(() => _updating = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final info = _info;
    final u = _update;
    final system = info?.origin == YtDlpOrigin.system;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionHeader('yt-dlp',
            subtitle: 'Получает ссылки на звук. YouTube часто меняется, поэтому yt-dlp стоит держать свежим.'),
        SettingsCard(
          icon: Icons.download_for_offline_outlined,
          title: 'Версия',
          trailing: _loading
              ? const StatusPill(Tone.busy, 'Проверяю')
              : info == null
                  ? const StatusPill(Tone.bad, 'Не найден')
                  : StatusPill(Tone.good, info.version),
          children: [
            if (_error != null)
              LoadFailed('yt-dlp не найден: $_error', onRetry: _load)
            else if (info != null) ...[
              KeyValueTable([
                ('Версия', info.version, null),
                ('Источник', originLabel(info.origin), null),
                ('Файл', info.binary, null),
              ]),
              if (system)
                const InlineMessage(Tone.neutral,
                    'Системный yt-dlp обновляет пакетный менеджер (например, pacman -Syu).'),
              ButtonRow([
                FilledButton.tonalIcon(
                  onPressed: _updating || system ? null : _runUpdate,
                  icon: _updating ? const ButtonSpinner() : const Icon(Icons.system_update_alt),
                  label: Text(_updating ? 'Обновляю…' : 'Обновить'),
                ),
              ]),
            ],
            if (_updateError != null) InlineMessage(Tone.bad, _updateError!),
            if (u != null) ...[
              InlineMessage(
                Tone.good,
                u.updated
                    ? 'Обновлён: ${u.versionBefore} → ${u.versionAfter}'
                    : u.origin == YtDlpOrigin.system
                        ? 'Системный yt-dlp обновляется пакетным менеджером'
                        : 'Уже последняя версия (${u.versionAfter})',
              ),
              if (u.message.trim().isNotEmpty)
                Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton(
                    onPressed: () => setState(() => _showOutput = !_showOutput),
                    child: Text(_showOutput ? 'Скрыть вывод yt-dlp' : 'Показать вывод yt-dlp'),
                  ),
                ),
              if (_showOutput)
                SelectableText(u.message,
                    style: const TextStyle(fontFamily: 'monospace', fontSize: 12, height: 1.4)),
            ],
          ],
        ),
      ],
    );
  }
}
