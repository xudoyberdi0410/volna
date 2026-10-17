/// The browse screens for the app router (B1's shell mounts them):
/// `/home`, `/search`, `/album/:id`, `/artist/:id`, `/playlist/:id`,
/// `/library`, `/library/playlist/:id`, `/history`, plus `/mood/:params`.
library;

import 'package:flutter/widgets.dart';
import 'package:go_router/go_router.dart';

import 'album_screen.dart';
import 'artist_screen.dart';
import 'history_screen.dart';
import 'home_screen.dart';
import 'library_screen.dart';
import 'playlist_screen.dart';
import 'search_screen.dart';
import 'user_playlist_screen.dart';

final List<RouteBase> browseRoutes = [
  GoRoute(path: '/home', builder: (_, _) => const HomeScreen()),
  GoRoute(
    path: '/search',
    builder: (_, s) => SearchScreen(initialQuery: s.uri.queryParameters['q']),
  ),
  GoRoute(
    path: '/album/:id',
    builder: (_, s) => AlbumScreen(key: ValueKey(s.pathParameters['id']), id: s.pathParameters['id']!),
  ),
  GoRoute(
    path: '/artist/:id',
    builder: (_, s) => ArtistScreen(key: ValueKey(s.pathParameters['id']), id: s.pathParameters['id']!),
  ),
  GoRoute(
    path: '/playlist/:id',
    builder: (_, s) => PlaylistScreen(key: ValueKey(s.pathParameters['id']), id: s.pathParameters['id']!),
  ),
  GoRoute(
    path: '/library',
    builder: (_, s) => LibraryScreen(initialTab: int.tryParse(s.uri.queryParameters['tab'] ?? '') ?? 0),
    routes: [
      GoRoute(
        path: 'playlist/:id',
        builder: (_, s) =>
            UserPlaylistScreen(key: ValueKey(s.pathParameters['id']), id: s.pathParameters['id']!),
      ),
    ],
  ),
  GoRoute(path: '/history', builder: (_, _) => const HistoryScreen()),
  GoRoute(
    path: '/mood/:params',
    builder: (_, s) => MoodScreen(
      params: s.pathParameters['params']!,
      title: s.uri.queryParameters['title'] ?? 'Подборки',
    ),
  ),
];
