/// Typed wrappers over the daemon's `catalog_*` commands (results of
/// `crates/ytm/src/api.rs`, serialised as the ytm models), as implemented in
/// `crates/daemon/src/api/catalog.rs`. Errors are [DaemonException]s with
/// code `bad_args`, `not_found`, `upstream` or `internal`.
library;

import '../ipc/transport.dart';
import '_json.dart';
import 'models.dart';

class CatalogApi {
  final Transport _t;
  const CatalogApi(this._t);

  Future<Json> _obj(String cmd, Map<String, Object?> args) async =>
      (await _t.call(cmd, args)) as Json;

  Future<SearchPage> search(String query, {SearchFilter filter = SearchFilter.all}) async =>
      SearchPage.fromJson(await _obj('catalog_search', {'query': query, 'filter': filter.name}));

  /// Next page of a filtered search. [filter] must be the one of the first
  /// page (`ytm::search_more` needs it to parse the rows).
  Future<SearchPage> searchMore(String continuation, {required SearchFilter filter}) async =>
      SearchPage.fromJson(await _obj(
          'catalog_search_more', {'continuation': continuation, 'filter': filter.name}));

  Future<SearchSuggestions> suggestions(String input) async =>
      SearchSuggestions.fromJson(await _obj('catalog_suggestions', {'input': input}));

  /// `MPREb_…` or an album playlist id (`OLAK5uy_…`).
  Future<Album> album(String id) async => Album.fromJson(await _obj('catalog_album', {'id': id}));

  Future<Artist> artist(String id) async =>
      Artist.fromJson(await _obj('catalog_artist', {'id': id}));

  Future<Playlist> playlist(String id) async =>
      Playlist.fromJson(await _obj('catalog_playlist', {'id': id}));

  Future<TrackPage> playlistMore(String continuation) async =>
      TrackPage.fromJson(await _obj('catalog_playlist_more', {'continuation': continuation}));

  Future<TrackDetails> track(String videoId) async =>
      TrackDetails.fromJson(await _obj('catalog_track', {'videoId': videoId}));

  Future<Related> related(String videoId) async =>
      Related.fromJson(await _obj('catalog_related', {'videoId': videoId}));

  /// Logged-out home feed (daemon default: 6 sections).
  Future<List<Section>> home({int? minSections}) async => listOf(
      await _t.call('catalog_home', {'minSections': ?minSections}), Section.fromJson);

  Future<List<MoodCategory>> moods() async =>
      listOf(await _t.call('catalog_moods'), MoodCategory.fromJson);

  /// Playlist shelves of one mood ([Mood.params]).
  Future<List<Section>> mood(String params) async =>
      listOf(await _t.call('catalog_mood', {'params': params}), Section.fromJson);
}
