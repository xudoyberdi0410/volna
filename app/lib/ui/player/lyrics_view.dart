/// Lyrics panel: karaoke when timed (word wipe for `synced: word`, line
/// highlight for `line`), plain text otherwise. Follows playback with smooth
/// auto-scroll (paused for a few seconds after the user scrolls), tap a line
/// to seek there, and a source switcher.
library;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../api/lyrics.dart';
import '../../api/player_models.dart';
import '../../state/lyrics.dart';
import '../../state/player.dart';

class LyricsPanel extends ConsumerWidget {
  const LyricsPanel({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final track = ref.watch(playerViewProvider.select((v) => v.current));
    final t = Theme.of(context);
    if (track == null) return const _Message(text: 'Ничего не играет');
    final chosen = ref.watch(lyricsSourceChoiceProvider.select((m) => m[track.id]));
    final key = lyricsKeyFor(track, source: chosen);
    final lyrics = ref.watch(lyricsProvider(key));
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          child: AnimatedSwitcher(
            duration: const Duration(milliseconds: 300),
            child: switch (lyrics) {
              AsyncData(value: final l?) when l.lines.isNotEmpty =>
                LyricsBody(key: ValueKey((track.uid, l.source)), lyrics: l),
              AsyncData() => const _Message(
                  key: ValueKey('none'),
                  icon: Icons.lyrics_outlined,
                  text: 'Текст для этого трека не нашёлся'),
              AsyncError() => const _Message(
                  key: ValueKey('err'),
                  icon: Icons.cloud_off_rounded,
                  text: 'Не удалось загрузить текст'),
              _ => _Message(key: const ValueKey('loading'), text: 'Ищу текст…', spinner: true),
            },
          ),
        ),
        if (lyrics.value case final l?)
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 8, 4, 0),
            child: Row(
              children: [
                _SourceButton(track: track, current: l.source),
                const Spacer(),
                if (l.songwriters != null)
                  Flexible(
                    child: Text('Авторы: ${l.songwriters}',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: t.textTheme.bodySmall
                            ?.copyWith(color: t.colorScheme.onSurfaceVariant)),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

class _Message extends StatelessWidget {
  final String text;
  final IconData? icon;
  final bool spinner;
  const _Message({super.key, required this.text, this.icon, this.spinner = false});

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    final c = t.colorScheme.onSurfaceVariant;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (spinner)
            SizedBox(
                width: 22, height: 22, child: CircularProgressIndicator(strokeWidth: 2, color: c))
          else if (icon != null)
            Icon(icon, size: 34, color: c),
          const SizedBox(height: 14),
          Text(text, style: t.textTheme.bodyLarge?.copyWith(color: c)),
        ],
      ),
    );
  }
}

class _SourceButton extends ConsumerWidget {
  final PlayerTrack track;
  final String current;
  const _SourceButton({required this.track, required this.current});

  Future<void> _open(BuildContext context, WidgetRef ref) async {
    final box = context.findRenderObject() as RenderBox?;
    final overlay = Overlay.of(context).context.findRenderObject() as RenderBox?;
    List<LyricsSource> sources;
    try {
      sources = await ref.read(lyricsSourcesProvider(lyricsKeyFor(track)).future);
    } catch (_) {
      sources = const [];
    }
    if (!context.mounted || box == null || overlay == null) return;
    final pos = RelativeRect.fromRect(
      box.localToGlobal(Offset.zero, ancestor: overlay) & box.size,
      Offset.zero & overlay.size,
    );
    final picked = await showMenu<String>(
      context: context,
      position: pos,
      items: [
        if (sources.isEmpty)
          const PopupMenuItem(enabled: false, child: Text('Других источников нет')),
        for (final s in sources)
          PopupMenuItem(
            value: s.source,
            child: Row(
              children: [
                SizedBox(
                  width: 22,
                  child: s.source == current ? const Icon(Icons.check_rounded, size: 18) : null,
                ),
                const SizedBox(width: 8),
                Text(s.source),
                const SizedBox(width: 16),
                Text(
                  switch (s.synced) {
                    Synced.word => 'по словам',
                    Synced.line => 'по строкам',
                    Synced.none => 'без синхронизации',
                  },
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                      color: Theme.of(context).colorScheme.onSurfaceVariant),
                ),
              ],
            ),
          ),
      ],
    );
    if (picked != null) ref.read(lyricsSourceChoiceProvider.notifier).choose(track.id, picked);
  }

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final t = Theme.of(context);
    return TextButton.icon(
      onPressed: () => _open(context, ref),
      icon: Icon(Icons.swap_horiz_rounded, size: 16, color: t.colorScheme.onSurfaceVariant),
      label: Text('Источник: $current',
          style: t.textTheme.labelMedium?.copyWith(color: t.colorScheme.onSurfaceVariant)),
    );
  }
}

/// The text itself, following [positionProvider].
class LyricsBody extends ConsumerStatefulWidget {
  final Lyrics lyrics;
  const LyricsBody({super.key, required this.lyrics});

  @override
  ConsumerState<LyricsBody> createState() => _LyricsBodyState();
}

class _LyricsBodyState extends ConsumerState<LyricsBody> with SingleTickerProviderStateMixin {
  late final Ticker _ticker = createTicker(_onTick);
  final _posMs = ValueNotifier<int>(0);
  final _scroll = ScrollController();
  late List<GlobalKey> _keys;
  int _active = -1;
  DateTime _userScrolledAt = DateTime(0);

  bool get _timed => widget.lyrics.synced != Synced.none;

  @override
  void initState() {
    super.initState();
    _keys = [for (final _ in widget.lyrics.lines) GlobalKey()];
    if (_timed) {
      _syncTicker(ref.read(positionProvider).playing);
      WidgetsBinding.instance.addPostFrameCallback((_) => _onTick(Duration.zero, jump: true));
    }
  }

  @override
  void dispose() {
    _ticker.dispose();
    _posMs.dispose();
    _scroll.dispose();
    super.dispose();
  }

  /// Ticks per frame only while playing; paused, every sample updates once.
  void _syncTicker(bool playing) {
    if (playing && !_ticker.isActive) {
      _ticker.start();
    } else if (!playing && _ticker.isActive) {
      _ticker.stop();
    }
  }

  void _onTick(Duration _, {bool jump = false}) {
    if (!mounted) return;
    final sample = ref.read(positionProvider);
    final ms = (sample.positionAt(ref.read(clockProvider)()) * 1000).round();
    if (ms != _posMs.value) _posMs.value = ms;
    final i = activeLineAt(widget.lyrics.lines, ms);
    if (i != _active) {
      setState(() => _active = i);
      _follow(jump: jump);
    } else if (jump) {
      _follow(jump: true);
    }
  }

  void _follow({bool jump = false}) {
    if (_active < 0) return;
    if (DateTime.now().difference(_userScrolledAt) < const Duration(seconds: 4)) return;
    final ctx = _keys[_active].currentContext;
    if (ctx == null) return;
    Scrollable.ensureVisible(
      ctx,
      alignment: 0.32,
      duration: jump ? Duration.zero : const Duration(milliseconds: 650),
      curve: Curves.easeOutCubic,
    );
  }

  @override
  Widget build(BuildContext context) {
    if (_timed) {
      ref.listen(positionProvider, (_, s) {
        _syncTicker(s.playing);
        if (!s.playing) _onTick(Duration.zero);
      });
    }
    final t = Theme.of(context);
    final lines = widget.lyrics.lines;
    final base = t.textTheme.headlineSmall!.copyWith(
      fontSize: 26,
      fontWeight: FontWeight.w700,
      height: 1.28,
      letterSpacing: -0.4,
    );
    final fg = t.colorScheme.onSurface;
    return NotificationListener<UserScrollNotification>(
      onNotification: (n) {
        if (n.direction != ScrollDirection.idle) _userScrolledAt = DateTime.now();
        return false;
      },
      child: ShaderMask(
        // Fade the text out at the top and bottom edges.
        shaderCallback: (r) => const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [Colors.transparent, Colors.black, Colors.black, Colors.transparent],
          stops: [0, 0.08, 0.86, 1],
        ).createShader(r),
        blendMode: BlendMode.dstIn,
        child: SingleChildScrollView(
          controller: _scroll,
          padding: const EdgeInsets.fromLTRB(8, 80, 24, 240),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              for (var i = 0; i < lines.length; i++)
                _LineView(
                  key: _keys[i],
                  line: lines[i],
                  state: !_timed
                      ? _LineState.plain
                      : i == _active
                          ? _LineState.active
                          : i < _active
                              ? _LineState.past
                              : _LineState.future,
                  wordSync: widget.lyrics.synced == Synced.word,
                  posMs: _posMs,
                  style: base,
                  color: fg,
                  onTap: _timed && lines[i].startMs > 0
                      ? () {
                          _userScrolledAt = DateTime(0);
                          ref.read(playerViewProvider.notifier).seek(lines[i].startMs / 1000);
                        }
                      : null,
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Index of the line playing at [ms]: the last one that started. -1 before
/// the first line.
int activeLineAt(List<LyricLine> lines, int ms) {
  var lo = 0, hi = lines.length - 1, ans = -1;
  while (lo <= hi) {
    final mid = (lo + hi) >> 1;
    if (lines[mid].startMs <= ms) {
      ans = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return ans;
}

enum _LineState { plain, past, active, future }

class _LineView extends StatefulWidget {
  final LyricLine line;
  final _LineState state;
  final bool wordSync;
  final ValueNotifier<int> posMs;
  final TextStyle style;
  final Color color;
  final VoidCallback? onTap;

  const _LineView({
    super.key,
    required this.line,
    required this.state,
    required this.wordSync,
    required this.posMs,
    required this.style,
    required this.color,
    this.onTap,
  });

  @override
  State<_LineView> createState() => _LineViewState();
}

class _LineViewState extends State<_LineView> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final st = widget.state;
    final alpha = switch (st) {
      _LineState.plain => 0.92,
      _LineState.active => 1.0,
      _LineState.past => 0.34,
      _LineState.future => 0.34,
    };
    final words = widget.line.words;
    final karaoke = st == _LineState.active && widget.wordSync && words != null && words.isNotEmpty;
    final style = widget.style.copyWith(
      fontSize: st == _LineState.plain ? 22 : widget.style.fontSize,
      fontWeight: st == _LineState.plain ? FontWeight.w600 : widget.style.fontWeight,
    );
    Widget text = karaoke
        ? ValueListenableBuilder<int>(
            valueListenable: widget.posMs,
            builder: (context, ms, _) => _KaraokeLine(
              words: words,
              ms: ms,
              style: style,
              lit: widget.color,
              dim: widget.color.withValues(alpha: 0.34),
            ),
          )
        : AnimatedDefaultTextStyle(
            duration: const Duration(milliseconds: 300),
            curve: Curves.easeOut,
            style: style.copyWith(
              color: widget.color.withValues(alpha: _hover && widget.onTap != null ? alpha + 0.25 : alpha),
            ),
            child: Text(widget.line.text.isEmpty ? '♪' : widget.line.text),
          );
    final bg = widget.line.background;
    if (bg != null && bg.isNotEmpty && st != _LineState.plain) {
      text = Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          text,
          Padding(
            padding: const EdgeInsets.only(top: 2),
            child: Text(bg.map((w) => w.text).join(),
                style: style.copyWith(
                  fontSize: (style.fontSize ?? 26) * 0.62,
                  fontWeight: FontWeight.w500,
                  color: widget.color.withValues(alpha: alpha * 0.7),
                )),
          ),
        ],
      );
    }
    return MouseRegion(
      cursor: widget.onTap == null ? MouseCursor.defer : SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        onTap: widget.onTap,
        child: AnimatedScale(
          scale: st == _LineState.active ? 1.0 : 0.97,
          alignment: Alignment.centerLeft,
          duration: const Duration(milliseconds: 380),
          curve: Curves.easeOutCubic,
          child: Padding(
            padding: EdgeInsets.symmetric(vertical: st == _LineState.plain ? 5 : 11),
            child: text,
          ),
        ),
      ),
    );
  }
}

/// A word-synced line: sung words lit, the current one wiped left to right.
class _KaraokeLine extends StatelessWidget {
  final List<LyricWord> words;
  final int ms;
  final TextStyle style;
  final Color lit;
  final Color dim;

  const _KaraokeLine({
    required this.words,
    required this.ms,
    required this.style,
    required this.lit,
    required this.dim,
  });

  @override
  Widget build(BuildContext context) {
    return Text.rich(
      TextSpan(children: [
        for (final w in words)
          if (ms >= w.endMs)
            TextSpan(text: w.text, style: style.copyWith(color: lit))
          else if (ms < w.startMs)
            TextSpan(text: w.text, style: style.copyWith(color: dim))
          else
            WidgetSpan(
              alignment: PlaceholderAlignment.baseline,
              baseline: TextBaseline.alphabetic,
              child: ShaderMask(
                blendMode: BlendMode.srcIn,
                shaderCallback: (r) {
                  final p = w.endMs > w.startMs
                      ? ((ms - w.startMs) / (w.endMs - w.startMs)).clamp(0.0, 1.0)
                      : 1.0;
                  // A soft edge 12 % wide.
                  return LinearGradient(
                    colors: [lit, lit, dim, dim],
                    stops: [0, (p - 0.06).clamp(0.0, 1.0), (p + 0.06).clamp(0.0, 1.0), 1],
                  ).createShader(r);
                },
                child: Text(w.text, style: style.copyWith(color: Colors.white)),
              ),
            ),
      ]),
    );
  }
}
