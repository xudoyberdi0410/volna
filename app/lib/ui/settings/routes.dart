/// Settings routes for the app router: `/settings` and `/settings/:section`
/// (`network`, `wave`, `quality`, `appearance`, `scrobbling`, `yandex`,
/// `library`, `ytdlp`, `about`).
library;

import 'package:go_router/go_router.dart';

import 'settings_screen.dart';

final List<RouteBase> settingsRoutes = [
  GoRoute(
    path: '/settings',
    pageBuilder: (context, state) => const NoTransitionPage(child: SettingsScreen()),
    routes: [
      GoRoute(
        path: ':section',
        pageBuilder: (context, state) =>
            NoTransitionPage(child: SettingsScreen(section: state.pathParameters['section'])),
      ),
    ],
  ),
];
