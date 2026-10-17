/// Browse routes (search, album, artist, playlist, library, history, home).
///
/// STUB: owned by the browse work package, which replaces this file. The
/// contract the shell relies on: a top-level `browseRoutes` list of routes
/// that live inside the app shell (they get the sidebar and the mini
/// player), with these paths:
///
/// `/search` (query `?q=`), `/album/:id`, `/artist/:id`, `/playlist/:id`,
/// `/library`, `/library/playlist/:id`, `/history`, `/home`.
///
/// Open them through `lib/ui/nav.dart` rather than with raw paths.
library;

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import '../shell/placeholder_page.dart';

List<RouteBase> get browseRoutes => [
      GoRoute(
        path: '/search',
        builder: (c, s) => PlaceholderPage(
            title: 'Поиск', subtitle: s.uri.queryParameters['q'], icon: Icons.search_rounded),
      ),
      GoRoute(
        path: '/album/:id',
        builder: (c, s) => PlaceholderPage(title: 'Альбом', subtitle: s.pathParameters['id']),
      ),
      GoRoute(
        path: '/artist/:id',
        builder: (c, s) => PlaceholderPage(title: 'Исполнитель', subtitle: s.pathParameters['id']),
      ),
      GoRoute(
        path: '/playlist/:id',
        builder: (c, s) => PlaceholderPage(title: 'Плейлист', subtitle: s.pathParameters['id']),
      ),
      GoRoute(
        path: '/library',
        builder: (c, s) =>
            const PlaceholderPage(title: 'Библиотека', icon: Icons.library_music_rounded),
        routes: [
          GoRoute(
            path: 'playlist/:id',
            builder: (c, s) =>
                PlaceholderPage(title: 'Мой плейлист', subtitle: s.pathParameters['id']),
          ),
        ],
      ),
      GoRoute(
        path: '/history',
        builder: (c, s) => const PlaceholderPage(title: 'История', icon: Icons.history_rounded),
      ),
      GoRoute(
        path: '/home',
        builder: (c, s) => const PlaceholderPage(title: 'Главная', icon: Icons.home_rounded),
      ),
    ];
