/// Settings routes.
///
/// STUB: owned by the settings work package, which replaces this file. The
/// contract the shell relies on: a top-level `settingsRoutes` list of routes
/// inside the app shell, with paths `/settings` and `/settings/:section`
/// (sections used by other screens: `wave`, `network`, `theme`, `quality`,
/// `scrobbling`, `yandex`). Theme toggles go through `themeModeProvider`
/// (`lib/state/theme.dart`).
library;

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import '../shell/placeholder_page.dart';

List<RouteBase> get settingsRoutes => [
      GoRoute(
        path: '/settings',
        builder: (c, s) => const PlaceholderPage(title: 'Настройки', icon: Icons.tune_rounded),
        routes: [
          GoRoute(
            path: ':section',
            builder: (c, s) => PlaceholderPage(
                title: 'Настройки', subtitle: s.pathParameters['section'], icon: Icons.tune_rounded),
          ),
        ],
      ),
    ];
