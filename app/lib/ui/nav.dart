/// Navigation helpers: every screen opens others through these, so paths
/// live in one place (`router.dart`, `browse/routes.dart`, `settings/routes.dart`).
library;

import 'package:flutter/widgets.dart';
import 'package:go_router/go_router.dart';

String _seg(String id) => Uri.encodeComponent(id);

/// Path constants for the top-level destinations.
abstract final class Paths {
  static const wave = '/wave';
  static const search = '/search';
  static const library = '/library';
  static const history = '/history';
  static const home = '/home';
  static const settings = '/settings';
  static const player = '/player';

  static String album(String id) => '/album/${_seg(id)}';
  static String artist(String id) => '/artist/${_seg(id)}';
  static String playlist(String id) => '/playlist/${_seg(id)}';
  static String libraryPlaylist(String id) => '/library/playlist/${_seg(id)}';
  static String settingsSection(String section) => '/settings/${_seg(section)}';
  static String searchQuery(String q) =>
      q.isEmpty ? search : '/search?q=${Uri.encodeQueryComponent(q)}';
}

// Detail pages are pushed (back returns to where the user was); top-level
// destinations replace the stack. Both close the fullscreen player first,
// since it sits above the shell.

GoRouter _router(BuildContext context) {
  final r = GoRouter.of(context);
  if (r.routerDelegate.currentConfiguration.uri.path == Paths.player && r.canPop()) r.pop();
  return r;
}

void openAlbum(BuildContext context, String id) => _router(context).push(Paths.album(id));

void openArtist(BuildContext context, String id) => _router(context).push(Paths.artist(id));

void openPlaylist(BuildContext context, String id) =>
    _router(context).push(Paths.playlist(id));

void openLibraryPlaylist(BuildContext context, String id) =>
    _router(context).push(Paths.libraryPlaylist(id));

void openSearch(BuildContext context, [String query = '']) =>
    _router(context).go(Paths.searchQuery(query));

void openLibrary(BuildContext context) => _router(context).go(Paths.library);

void openHistory(BuildContext context) => _router(context).push(Paths.history);

void openWave(BuildContext context) => _router(context).go(Paths.wave);

void openSettings(BuildContext context, [String? section]) =>
    _router(context).go(section == null ? Paths.settings : Paths.settingsSection(section));

/// The fullscreen player (over the shell). [tab]: `lyrics` or `queue`.
void openPlayer(BuildContext context, {String? tab}) =>
    context.push(tab == null ? Paths.player : '${Paths.player}?tab=$tab');
