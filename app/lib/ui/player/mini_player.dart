/// The bar at the bottom of the shell: what plays, transport, like, volume,
/// and the ways into the fullscreen player (cover, lyrics, queue).
library;

import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../state/player.dart';
import '../nav.dart';
import 'controls.dart';
import 'cover.dart';
import 'track_title.dart';

/// Hero tag of the now-playing cover (mini player ↔ fullscreen player).
const kNowCoverHero = 'now-playing-cover';

class MiniPlayer extends ConsumerWidget {
  const MiniPlayer({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final track = ref.watch(playerViewProvider.select((v) => v.current));
    final t = Theme.of(context);
    final dark = t.brightness == Brightness.dark;
    final wide = MediaQuery.sizeOf(context).width >= 980;
    return AnimatedSize(
      duration: const Duration(milliseconds: 320),
      curve: Curves.easeOutCubic,
      alignment: Alignment.bottomCenter,
      child: track == null
          ? const SizedBox(width: double.infinity)
          : Padding(
              padding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
              child: ClipRRect(
                borderRadius: BorderRadius.circular(18),
                child: BackdropFilter(
                  filter: ImageFilter.blur(sigmaX: 30, sigmaY: 30),
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      color: dark
                          ? Colors.white.withValues(alpha: 0.07)
                          : Colors.white.withValues(alpha: 0.55),
                      border: Border.all(color: t.colorScheme.onSurface.withValues(alpha: 0.07)),
                      borderRadius: BorderRadius.circular(18),
                    ),
                    child: SizedBox(
                      height: 76,
                      child: Stack(
                        children: [
                          Row(
                            children: [
                              const SizedBox(width: 10),
                              Expanded(
                                flex: 5,
                                child: _NowPlaying(trackTitle: TrackTitle(track: track, dense: true)),
                              ),
                              const TransportControls(playSize: 40, showModes: false),
                              Expanded(
                                flex: 5,
                                child: Row(
                                  mainAxisAlignment: MainAxisAlignment.end,
                                  children: [
                                    LikeButton(track: track, size: 20),
                                    IconButton(
                                      tooltip: 'Текст песни',
                                      onPressed: () => openPlayer(context, tab: 'lyrics'),
                                      icon: Icon(Icons.lyrics_outlined,
                                          size: 20, color: t.colorScheme.onSurfaceVariant),
                                    ),
                                    IconButton(
                                      tooltip: 'Очередь',
                                      onPressed: () => openPlayer(context, tab: 'queue'),
                                      icon: Icon(Icons.queue_music_rounded,
                                          size: 21, color: t.colorScheme.onSurfaceVariant),
                                    ),
                                    if (wide) const VolumeControl(width: 96),
                                    const SizedBox(width: 10),
                                  ],
                                ),
                              ),
                            ],
                          ),
                          const Positioned(left: 0, right: 0, bottom: 0, child: ProgressLine()),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
            ),
    );
  }
}

class _NowPlaying extends ConsumerWidget {
  final Widget trackTitle;
  const _NowPlaying({required this.trackTitle});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final cover = ref.watch(playerViewProvider.select((v) => v.current?.cover));
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => openPlayer(context),
        child: Row(
          children: [
            Hero(
              tag: kNowCoverHero,
              child: CoverArt(url: cover, size: 52, radius: 8),
            ),
            const SizedBox(width: 12),
            Flexible(child: trackTitle),
          ],
        ),
      ),
    );
  }
}
