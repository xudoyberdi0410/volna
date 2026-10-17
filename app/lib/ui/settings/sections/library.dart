import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../api/settings.dart';
import '../../../state/settings.dart';
import '../platform.dart';
import '../widgets.dart';

/// Settings → «Библиотека»: moving the library from the old window.
class LibrarySection extends ConsumerStatefulWidget {
  const LibrarySection({super.key});

  @override
  ConsumerState<LibrarySection> createState() => _LibrarySectionState();
}

class _LibrarySectionState extends ConsumerState<LibrarySection> {
  LegacyStatus? _status;
  String? _loadError;
  bool _importing = false;
  LegacyReport? _report;
  String? _importError;

  LegacyApi get _api => ref.read(legacyApiProvider);

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    setState(() => _loadError = null);
    try {
      final s = await _api.status();
      if (mounted) setState(() => _status = s);
    } catch (e) {
      if (mounted) setState(() => _loadError = errorText(e));
    }
  }

  Future<void> _importFile() async {
    final path = await ref.read(jsonFilePickerProvider)();
    if (path == null || !mounted) return;
    setState(() {
      _importing = true;
      _report = null;
      _importError = null;
    });
    try {
      final r = await _api.importFile(path);
      if (!mounted) return;
      setState(() => _report = r);
      await _load();
    } catch (e) {
      if (mounted) setState(() => _importError = 'Не удалось импортировать: ${errorText(e)}');
    } finally {
      if (mounted) setState(() => _importing = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = _status;
    final r = _report;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionHeader('Библиотека',
            subtitle: 'Лайки, плейлисты и история хранятся в демоне Volna и не зависят от окна.'),
        SettingsCard(
          icon: Icons.drive_file_move_outline,
          title: 'Перенос из старой версии',
          subtitle: 'Библиотека прежнего окна переносится сама при первом запуске новой версии. '
              'Если этого не случилось, импортируйте файл, сохранённый старым окном '
              '(«Экспорт библиотеки», volna-library-….json).',
          trailing: _loadError != null
              ? const StatusPill(Tone.bad, 'Нет связи')
              : s == null
                  ? const StatusPill(Tone.busy, 'Проверяю')
                  : s.imported
                      ? const StatusPill(Tone.good, 'Перенесена')
                      : const StatusPill(Tone.neutral, 'Не переносилась'),
          children: [
            if (_loadError != null) LoadFailed(_loadError!, onRetry: _load),
            if (s?.imported == true && s?.importedAt != null)
              KeyValueTable([('Перенесена', _date(s!.importedAt!), Tone.good)]),
            ButtonRow([
              FilledButton.tonalIcon(
                key: const Key('legacy-import'),
                onPressed: _importing ? null : _importFile,
                icon: _importing ? const ButtonSpinner() : const Icon(Icons.file_open_outlined),
                label: const Text('Импортировать из файла…'),
              ),
            ]),
            if (_importError != null) InlineMessage(Tone.bad, _importError!),
            if (r != null)
              InlineMessage(
                Tone.good,
                'Готово. Треков: ${r.tracks}, альбомов: ${r.albums}, артистов: ${r.artists}, '
                'плейлистов: ${r.playlists + r.userPlaylists} (в них ${r.playlistTracks} треков), '
                'история: ${r.history}.'
                '${r.legacyKeys > 0 ? ' Настройки скробблеров перенесены.' : ''}'
                '${r.skipped > 0 ? ' Пропущено без id YouTube Music: ${r.skipped}.' : ''}',
              ),
          ],
        ),
      ],
    );
  }
}

String _date(int ms) {
  final d = DateTime.fromMillisecondsSinceEpoch(ms);
  String two(int n) => n.toString().padLeft(2, '0');
  return '${two(d.day)}.${two(d.month)}.${d.year} ${two(d.hour)}:${two(d.minute)}';
}
