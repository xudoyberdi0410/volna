/// Routes: everything lives in the shell (sidebar + mini player) except the
/// fullscreen player, which slides up over it.
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'browse/routes.dart';
import 'nav.dart';
import 'player/full_player.dart';
import 'settings/routes.dart';
import 'shell/app_shell.dart';
import 'wave/wave_screen.dart';

final rootNavigatorKey = GlobalKey<NavigatorState>(debugLabel: 'root');
final _shellNavigatorKey = GlobalKey<NavigatorState>(debugLabel: 'shell');

GoRouter buildRouter({String initialLocation = Paths.wave}) => GoRouter(
      navigatorKey: rootNavigatorKey,
      initialLocation: initialLocation,
      routes: [
        GoRoute(path: '/', redirect: (_, _) => Paths.wave),
        ShellRoute(
          navigatorKey: _shellNavigatorKey,
          builder: (context, state, child) => AppShell(child: child),
          routes: [
            GoRoute(
              path: Paths.wave,
              pageBuilder: (c, s) => const NoTransitionPage(child: WaveScreen()),
            ),
            ...browseRoutes,
            ...settingsRoutes,
          ],
        ),
        GoRoute(
          path: Paths.player,
          parentNavigatorKey: rootNavigatorKey,
          pageBuilder: (context, state) => CustomTransitionPage(
            key: state.pageKey,
            opaque: false,
            transitionDuration: const Duration(milliseconds: 460),
            reverseTransitionDuration: const Duration(milliseconds: 340),
            child: FullPlayer(initialPanel: panelFromQuery(state.uri.queryParameters['tab'])),
            transitionsBuilder: (context, a, _, child) {
              final curved = CurvedAnimation(
                parent: a,
                curve: Curves.easeOutCubic,
                reverseCurve: Curves.easeInCubic,
              );
              return FadeTransition(
                opacity: Tween(begin: 0.0, end: 1.0).animate(
                    CurvedAnimation(parent: a, curve: const Interval(0, 0.6))),
                child: SlideTransition(
                  position: Tween(begin: const Offset(0, 0.08), end: Offset.zero).animate(curved),
                  child: child,
                ),
              );
            },
          ),
        ),
      ],
    );

final routerProvider = Provider<GoRouter>((ref) {
  final r = buildRouter();
  ref.onDispose(r.dispose);
  return r;
});
