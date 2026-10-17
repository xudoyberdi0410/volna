import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../api/settings.dart';
import '../../../ipc/proto_image.dart';
import '../../../state/settings.dart';
import '../widgets.dart';

/// Settings → «Волна»: character, mood, activity, language, and the
/// cold-start artist picker that seeds the wave.
class WaveSection extends ConsumerStatefulWidget {
  const WaveSection({super.key});

  @override
  ConsumerState<WaveSection> createState() => _WaveSectionState();
}

class _WaveSectionState extends ConsumerState<WaveSection> {
  WaveStatus? _status;
  String? _loadError;
  WaveActivities? _activities;
  String? _activitiesError;
  bool _saving = false;
  String? _saveError;

  WaveSettingsApi get _api => ref.read(waveSettingsApiProvider);

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
    _loadActivities();
  }

  Future<void> _loadActivities() async {
    setState(() => _activitiesError = null);
    try {
      final a = await _api.activities();
      if (mounted) setState(() => _activities = a);
    } catch (e) {
      if (mounted) setState(() => _activitiesError = errorText(e));
    }
  }

  Future<void> _save(WaveSettings s) async {
    final before = _status;
    setState(() {
      _status = WaveStatus(cold: before?.cold ?? false, active: before?.active ?? false, settings: s);
      _saving = true;
      _saveError = null;
    });
    try {
      final st = await _api.setSettings(s);
      if (mounted) setState(() => _status = st);
    } catch (e) {
      if (mounted) {
        setState(() {
          _status = before;
          _saveError = 'Не удалось сохранить: ${errorText(e)}';
        });
      }
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final st = _status;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionHeader('Моя волна',
            subtitle: 'Бесконечный поток под ваш вкус. Настройки применяются к следующим трекам волны.'),
        if (_loadError != null)
          SettingsCard(icon: Icons.waves, title: 'Настройки волны', children: [LoadFailed(_loadError!, onRetry: _load)])
        else if (st == null)
          const Padding(padding: EdgeInsets.all(32), child: Center(child: CircularProgressIndicator()))
        else ...[
          _settingsCard(st),
          ColdStartCard(cold: st.cold, onSeeded: (s) => setState(() => _status = s)),
        ],
      ],
    );
  }

  Widget _settingsCard(WaveStatus st) {
    final s = st.settings;
    final t = Theme.of(context).textTheme;
    final acts = _activities;
    return SettingsCard(
      icon: Icons.waves,
      title: 'Настройки волны',
      subtitle: 'Что подмешивать в поток',
      trailing: _saving
          ? const StatusPill(Tone.busy, 'Сохраняю')
          : StatusPill(st.active ? Tone.good : Tone.neutral, st.active ? 'Играет' : 'Не запущена'),
      children: [
        const FieldLabel('Характер'),
        SegmentedButton<WaveCharacter>(
          segments: [
            for (final c in WaveCharacter.values) ButtonSegment(value: c, label: Text(c.label)),
          ],
          selected: {s.character},
          showSelectedIcon: false,
          onSelectionChanged: (v) => _save(s.copyWith(character: v.first)),
        ),
        const SizedBox(height: 20),
        const FieldLabel('Настроение'),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            ChoiceChip(
              label: const Text('Любое'),
              selected: s.mood == null,
              onSelected: (_) => _save(s.copyWith(mood: () => null)),
            ),
            for (final m in WaveMood.values)
              ChoiceChip(
                label: Text(m.label),
                selected: s.mood == m,
                onSelected: (on) => _save(s.copyWith(mood: () => on ? m : null)),
              ),
          ],
        ),
        const SizedBox(height: 20),
        const FieldLabel('Язык'),
        SegmentedButton<WaveLanguage>(
          segments: [
            for (final l in WaveLanguage.values) ButtonSegment(value: l, label: Text(l.label)),
          ],
          selected: {s.language},
          showSelectedIcon: false,
          onSelectionChanged: (v) => _save(s.copyWith(language: v.first)),
        ),
        const SizedBox(height: 20),
        const FieldLabel('Занятие или жанр'),
        if (_activitiesError != null)
          LoadFailed('Не удалось загрузить список: $_activitiesError', onRetry: _loadActivities)
        else if (acts == null)
          const LinearProgressIndicator()
        else ...[
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              ChoiceChip(
                label: const Text('Без занятия'),
                selected: s.activity == null,
                onSelected: (_) => _save(s.copyWith(activity: () => null)),
              ),
              for (final a in acts.activities) _activityChip(s, a),
            ],
          ),
          if (acts.genres.isNotEmpty) ...[
            const SizedBox(height: 12),
            Text('Жанры', style: t.labelMedium),
            const SizedBox(height: 8),
            Wrap(spacing: 8, runSpacing: 8, children: [for (final a in acts.genres) _activityChip(s, a)]),
          ],
        ],
        if (_saveError != null) InlineMessage(Tone.bad, _saveError!),
      ],
    );
  }

  Widget _activityChip(WaveSettings s, WaveActivity a) => ChoiceChip(
        label: Text(a.label),
        selected: s.activity?.title == a.title,
        onSelected: (on) => _save(s.copyWith(activity: () => on ? a : null)),
      );
}

/// Picks artists to seed the wave (`wave_artist_search`, `wave_related_artists`,
/// `wave_cold_start`).
class ColdStartCard extends ConsumerStatefulWidget {
  final bool cold;
  final ValueChanged<WaveStatus> onSeeded;
  const ColdStartCard({super.key, required this.cold, required this.onSeeded});

  @override
  ConsumerState<ColdStartCard> createState() => _ColdStartCardState();
}

class _ColdStartCardState extends ConsumerState<ColdStartCard> {
  final _query = TextEditingController();
  Timer? _debounce;
  int _seq = 0;
  List<ArtistCard>? _results;
  String? _error;
  bool _searching = false;
  final _picked = <String, ArtistCard>{};
  final _related = <ArtistCard>[];
  bool _seeding = false;
  (Tone, String)? _message;

  WaveSettingsApi get _api => ref.read(waveSettingsApiProvider);

  @override
  void initState() {
    super.initState();
    _search('');
  }

  @override
  void dispose() {
    _debounce?.cancel();
    _query.dispose();
    super.dispose();
  }

  Future<void> _search(String q) async {
    final seq = ++_seq;
    setState(() {
      _searching = true;
      _error = null;
    });
    try {
      final r = await _api.artistSearch(q);
      if (mounted && seq == _seq) setState(() => _results = r);
    } catch (e) {
      if (mounted && seq == _seq) setState(() => _error = errorText(e));
    } finally {
      if (mounted && seq == _seq) setState(() => _searching = false);
    }
  }

  void _onQuery(String q) {
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 350), () => _search(q));
  }

  Future<void> _toggle(ArtistCard a) async {
    setState(() {
      _message = null;
      if (_picked.remove(a.id) == null) _picked[a.id] = a;
    });
    if (!_picked.containsKey(a.id)) return;
    try {
      final rel = await _api.relatedArtists(a.id);
      if (!mounted) return;
      final known = {..._related.map((x) => x.id), ...?_results?.map((x) => x.id)};
      setState(() => _related.addAll(rel.where((x) => !known.contains(x.id)).take(6)));
    } catch (_) {
      // Related artists only grow the grid; nothing to report.
    }
  }

  Future<void> _seed() async {
    setState(() {
      _seeding = true;
      _message = null;
    });
    try {
      final st = await _api.coldStart(_picked.values.toList());
      if (!mounted) return;
      final n = _picked.length;
      setState(() {
        _picked.clear();
        _related.clear();
        _message = (Tone.good, 'Добавлено: $n ${_artistsWord(n)}. Волна учтёт их со следующего трека.');
      });
      widget.onSeeded(st);
    } catch (e) {
      if (mounted) setState(() => _message = (Tone.bad, 'Не удалось: ${errorText(e)}'));
    } finally {
      if (mounted) setState(() => _seeding = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final grid = <ArtistCard>[
      ..._picked.values,
      ...?_results?.where((a) => !_picked.containsKey(a.id)),
      ..._related.where((a) => !_picked.containsKey(a.id) && !(_results?.any((r) => r.id == a.id) ?? false)),
    ];
    final cache = ref.watch(settingsImageCacheProvider);
    return SettingsCard(
      icon: Icons.person_search_outlined,
      title: 'Любимые артисты',
      subtitle: widget.cold
          ? 'Волна ещё не знает ваш вкус. Отметьте несколько артистов, чтобы с них начать.'
          : 'Добавьте артистов, чтобы волна чаще их предлагала.',
      trailing: widget.cold ? const StatusPill(Tone.warn, 'Нужен выбор') : const StatusPill(Tone.good, 'Вкус известен'),
      children: [
        TextField(
          controller: _query,
          onChanged: _onQuery,
          decoration: fieldDecoration(
            hint: 'Найти артиста',
            suffix: _searching
                ? const Padding(padding: EdgeInsets.all(12), child: ButtonSpinner())
                : const Icon(Icons.search),
          ),
        ),
        const SizedBox(height: 16),
        if (_error != null)
          LoadFailed(_error!, onRetry: () => _search(_query.text))
        else if (_results == null)
          const SizedBox(height: 120, child: Center(child: CircularProgressIndicator()))
        else if (grid.isEmpty)
          const InlineMessage(Tone.neutral, 'Никого не нашлось')
        else
          Wrap(
            spacing: 6,
            runSpacing: 10,
            children: [
              for (final a in grid.take(36))
                _ArtistTile(
                  artist: a,
                  cache: cache,
                  picked: _picked.containsKey(a.id),
                  onTap: () => _toggle(a),
                ),
            ],
          ),
        ButtonRow([
          FilledButton.icon(
            onPressed: _picked.isEmpty || _seeding ? null : _seed,
            icon: _seeding ? const ButtonSpinner() : const Icon(Icons.add),
            label: Text(_picked.isEmpty ? 'Добавить в волну' : 'Добавить в волну (${_picked.length})'),
          ),
          if (_picked.isNotEmpty)
            TextButton(onPressed: () => setState(_picked.clear), child: const Text('Сбросить выбор')),
        ]),
        if (_message != null) InlineMessage(_message!.$1, _message!.$2),
      ],
    );
  }
}

String _artistsWord(int n) {
  final m10 = n % 10, m100 = n % 100;
  if (m10 == 1 && m100 != 11) return 'артист';
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return 'артиста';
  return 'артистов';
}

class _ArtistTile extends StatelessWidget {
  final ArtistCard artist;
  final ProtoImageCache cache;
  final bool picked;
  final VoidCallback onTap;
  const _ArtistTile({required this.artist, required this.cache, required this.picked, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    final pic = artist.picture;
    return InkWell(
      borderRadius: BorderRadius.circular(16),
      onTap: onTap,
      child: SizedBox(
        width: 104,
        child: Padding(
          padding: const EdgeInsets.all(6),
          child: Column(
            children: [
              Stack(
                children: [
                  AnimatedContainer(
                    duration: const Duration(milliseconds: 160),
                    padding: const EdgeInsets.all(3),
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      border: Border.all(color: picked ? c.primary : Colors.transparent, width: 3),
                    ),
                    child: CircleAvatar(
                      radius: 38,
                      backgroundColor: c.surfaceContainerHighest,
                      foregroundImage: pic == null ? null : VolnaImage(pic, cache: cache),
                      child: Text(
                        artist.name.isEmpty ? '?' : artist.name.characters.first.toUpperCase(),
                        style: Theme.of(context).textTheme.titleLarge?.copyWith(color: c.onSurfaceVariant),
                      ),
                    ),
                  ),
                  if (picked)
                    Positioned(
                      right: 0,
                      bottom: 0,
                      child: Container(
                        decoration: BoxDecoration(color: c.primary, shape: BoxShape.circle),
                        padding: const EdgeInsets.all(3),
                        child: Icon(Icons.check, size: 16, color: c.onPrimary),
                      ),
                    ),
                ],
              ),
              const SizedBox(height: 6),
              Text(
                artist.name,
                maxLines: 2,
                textAlign: TextAlign.center,
                overflow: TextOverflow.ellipsis,
                style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    fontWeight: picked ? FontWeight.w600 : FontWeight.normal),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
