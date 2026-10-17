/// The fullscreen player: big cover (hero from the mini player), seek bar,
/// transport, volume, like; on the right the lyrics or the queue.
library;

import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../state/player.dart';
import '../nav.dart';
import '../shell/background.dart';
import 'controls.dart';
import 'cover.dart';
import 'lyrics_view.dart';
import 'mini_player.dart';
import 'queue_view.dart';
import 'track_title.dart';

enum PlayerPanel { none, lyrics, queue }

PlayerPanel panelFromQuery(String? tab) => switch (tab) {
      'queue' => PlayerPanel.queue,
      'none' => PlayerPanel.none,
      _ => PlayerPanel.lyrics,
    };

class FullPlayer extends ConsumerStatefulWidget {
  final PlayerPanel initialPanel;
  const FullPlayer({super.key, this.initialPanel = PlayerPanel.lyrics});

  @override
  ConsumerState<FullPlayer> createState() => _FullPlayerState();
}

class _FullPlayerState extends ConsumerState<FullPlayer> {
  late PlayerPanel _panel = widget.initialPanel;

  void _toggle(PlayerPanel p) => setState(() => _panel = _panel == p ? PlayerPanel.none : p);

  void _close() {
    if (context.canPop()) {
      context.pop();
    } else {
      openWave(context);
    }
  }

  @override
  Widget build(BuildContext context) {
    final v = ref.watch(playerViewProvider);
    final track = v.current;
    final t = Theme.of(context);
    final size = MediaQuery.sizeOf(context);
    final wide = size.width >= 980;
    final showPanel = _panel != PlayerPanel.none;

    final label = v.waveActive ? 'Моя волна' : 'Сейчас играет';

    final top = Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 0),
      child: Row(
        children: [
          IconButton(
            tooltip: 'Свернуть (Esc)',
            onPressed: _close,
            icon: const Icon(Icons.keyboard_arrow_down_rounded, size: 30),
          ),
          Expanded(
            child: Text(label.toUpperCase(),
                textAlign: TextAlign.center,
                style: t.textTheme.labelSmall?.copyWith(
                  color: t.colorScheme.onSurfaceVariant,
                  letterSpacing: 1.6,
                )),
          ),
          _PanelTabs(panel: _panel, onSelect: _toggle),
        ],
      ),
    );

    Widget nowPlaying(BoxConstraints c) {
      final cover = math.min(math.min(c.maxWidth - 64, c.maxHeight - 250), 560.0).clamp(160.0, 560.0);
      return Center(
        child: SizedBox(
          width: cover,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Hero(
                tag: kNowCoverHero,
                child: CoverArt(url: track?.cover, size: cover, radius: 18, elevated: true),
              ),
              const SizedBox(height: 28),
              Row(
                children: [
                  Expanded(
                    child: track == null
                        ? Text('Ничего не играет', style: t.textTheme.headlineSmall)
                        : TrackTitle(
                            track: track,
                            titleStyle: t.textTheme.headlineSmall,
                            artistStyle: t.textTheme.bodyLarge,
                          ),
                  ),
                  if (track != null) LikeButton(track: track, size: 26),
                ],
              ),
              const SizedBox(height: 18),
              const SeekBar(),
              const SizedBox(height: 10),
              const Center(child: TransportControls(playSize: 64)),
              const SizedBox(height: 14),
              const Center(child: VolumeControl(width: 160)),
            ],
          ),
        ),
      );
    }

    final panel = switch (_panel) {
      PlayerPanel.lyrics => const LyricsPanel(),
      PlayerPanel.queue => const QueueView(),
      PlayerPanel.none => const SizedBox.shrink(),
    };

    final body = wide
        ? Row(
            children: [
              Expanded(flex: 5, child: LayoutBuilder(builder: (_, c) => nowPlaying(c))),
              AnimatedSize(
                duration: const Duration(milliseconds: 380),
                curve: Curves.easeOutCubic,
                child: SizedBox(
                  width: showPanel ? math.min(size.width * 0.44, 640) : 0,
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(0, 8, 32, 24),
                    child: AnimatedSwitcher(
                      duration: const Duration(milliseconds: 260),
                      child: KeyedSubtree(key: ValueKey(_panel), child: panel),
                    ),
                  ),
                ),
              ),
            ],
          )
        : AnimatedSwitcher(
            duration: const Duration(milliseconds: 260),
            child: showPanel
                ? Padding(
                    key: ValueKey(_panel),
                    padding: const EdgeInsets.fromLTRB(24, 8, 24, 16),
                    child: panel,
                  )
                : LayoutBuilder(key: const ValueKey('cover'), builder: (_, c) => nowPlaying(c)),
          );

    return CallbackShortcuts(
      bindings: {const SingleActivator(LogicalKeyboardKey.escape): _close},
      child: Focus(
        autofocus: true,
        child: Material(
          type: MaterialType.transparency,
          child: AppBackground(
            vivid: true,
            child: SafeArea(
              child: Column(
                children: [
                  top,
                  Expanded(child: body),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _PanelTabs extends StatelessWidget {
  final PlayerPanel panel;
  final ValueChanged<PlayerPanel> onSelect;
  const _PanelTabs({required this.panel, required this.onSelect});

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    Widget tab(PlayerPanel p, IconData icon, String label) {
      final on = panel == p;
      return Padding(
        padding: const EdgeInsets.only(left: 6),
        child: Material(
          color: on ? t.colorScheme.onSurface.withValues(alpha: 0.12) : Colors.transparent,
          shape: const StadiumBorder(),
          child: InkWell(
            customBorder: const StadiumBorder(),
            onTap: () => onSelect(p),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Icon(icon, size: 18, color: on ? t.colorScheme.onSurface : t.colorScheme.onSurfaceVariant),
                  const SizedBox(width: 8),
                  Text(label,
                      style: t.textTheme.labelLarge?.copyWith(
                          color: on ? t.colorScheme.onSurface : t.colorScheme.onSurfaceVariant)),
                ],
              ),
            ),
          ),
        ),
      );
    }

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        tab(PlayerPanel.lyrics, Icons.lyrics_outlined, 'Текст'),
        tab(PlayerPanel.queue, Icons.queue_music_rounded, 'Очередь'),
      ],
    );
  }
}
