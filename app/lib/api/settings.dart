/// Typed wrappers for the rest of Settings: the wave (`crates/daemon/src/wave.rs`
/// `wave_status`, `wave_set_settings`, `wave_activities`, the cold-start
/// artist picker) and the legacy library import (`legacy_status`,
/// `legacy_import{path}` in `crates/daemon/src/api/library.rs`).
/// Playback quality lives in the player settings (`PlayerApi.setSettings`).
library;

import '../ipc/transport.dart';
import '_json.dart';

/// "Характер".
enum WaveCharacter {
  defaultMix('default', 'Любой'),
  favorite('favorite', 'Любимое'),
  unfamiliar('unfamiliar', 'Незнакомое'),
  popular('popular', 'Популярное');

  final String wire;
  final String label;
  const WaveCharacter(this.wire, this.label);
}

/// "Настроение".
enum WaveMood {
  energetic('Бодрое'),
  happy('Весёлое'),
  calm('Спокойное'),
  sad('Грустное');

  final String label;
  const WaveMood(this.label);
}

/// "Язык".
enum WaveLanguage {
  any('Любой'),
  russian('Русский'),
  uzbek('Узбекский'),
  english('Английский');

  final String label;
  const WaveLanguage(this.label);
}

/// "Занятие" or a genre: a YouTube Music mood button.
class WaveActivity {
  /// YouTube Music title, the stable key.
  final String title;
  final String params;

  /// Russian label (only in `wave_activities`).
  final String label;
  const WaveActivity(this.title, this.params, [String? label]) : label = label ?? title;

  factory WaveActivity.fromJson(Json j) =>
      WaveActivity(j['title'] as String? ?? '', j['params'] as String? ?? '', j['label'] as String?);

  Json toJson() => {'title': title, 'params': params};
}

class WaveActivities {
  final List<WaveActivity> activities;
  final List<WaveActivity> genres;
  const WaveActivities(this.activities, this.genres);
  factory WaveActivities.fromJson(Json j) =>
      WaveActivities(listOf(j['activities'], WaveActivity.fromJson), listOf(j['genres'], WaveActivity.fromJson));
}

class WaveSettings {
  final WaveCharacter character;
  final WaveMood? mood;
  final WaveActivity? activity;
  final WaveLanguage language;
  const WaveSettings(
      {this.character = WaveCharacter.defaultMix, this.mood, this.activity, this.language = WaveLanguage.any});

  factory WaveSettings.fromJson(Json? j) => WaveSettings(
        character: WaveCharacter.values
            .firstWhere((c) => c.wire == j?['character'], orElse: () => WaveCharacter.defaultMix),
        mood: j?['mood'] == null ? null : enumOf(WaveMood.values, j!['mood'], WaveMood.calm),
        activity: objOf(j?['activity'], WaveActivity.fromJson),
        language: enumOf(WaveLanguage.values, j?['language'], WaveLanguage.any),
      );

  Json toJson() => {
        'character': character.wire,
        'mood': mood?.name,
        'activity': activity?.toJson(),
        'language': language.name,
      };

  WaveSettings copyWith({
    WaveCharacter? character,
    WaveMood? Function()? mood,
    WaveActivity? Function()? activity,
    WaveLanguage? language,
  }) =>
      WaveSettings(
        character: character ?? this.character,
        mood: mood == null ? this.mood : mood(),
        activity: activity == null ? this.activity : activity(),
        language: language ?? this.language,
      );
}

class WaveStatus {
  /// No taste yet: the cold-start picker comes first.
  final bool cold;
  final bool active;
  final WaveSettings settings;
  const WaveStatus({this.cold = false, this.active = false, this.settings = const WaveSettings()});

  factory WaveStatus.fromJson(Json j) => WaveStatus(
        cold: j['cold'] as bool? ?? false,
        active: j['active'] as bool? ?? false,
        settings: WaveSettings.fromJson(j['settings'] as Json?),
      );
}

/// An artist tile of the cold-start picker; [picture] is a `volna://img` URL.
class ArtistCard {
  final String id;
  final String name;
  final String? picture;
  const ArtistCard(this.id, this.name, [this.picture]);
  factory ArtistCard.fromJson(Json j) =>
      ArtistCard(j['id'] as String? ?? '', j['name'] as String? ?? '', j['picture'] as String?);
}

class WaveSettingsApi {
  final Transport _t;
  const WaveSettingsApi(this._t);

  Future<WaveStatus> status() async => WaveStatus.fromJson((await _t.call('wave_status')) as Json);

  Future<WaveStatus> setSettings(WaveSettings s) async =>
      WaveStatus.fromJson((await _t.call('wave_set_settings', {'settings': s.toJson()})) as Json);

  Future<WaveActivities> activities() async =>
      WaveActivities.fromJson((await _t.call('wave_activities')) as Json);

  /// Empty [query]: the default grid of popular artists.
  Future<List<ArtistCard>> artistSearch(String query) async =>
      listOf(await _t.call('wave_artist_search', {'query': query}), ArtistCard.fromJson);

  Future<List<ArtistCard>> relatedArtists(String id) async =>
      listOf(await _t.call('wave_related_artists', {'id': id}), ArtistCard.fromJson);

  /// Seeds the wave with the picked artists.
  Future<WaveStatus> coldStart(List<ArtistCard> artists) async => WaveStatus.fromJson((await _t.call(
      'wave_cold_start', {
    'artists': [
      for (final a in artists) {'id': a.id, 'name': a.name}
    ]
  })) as Json);
}

class LegacyStatus {
  final bool imported;

  /// Unix ms.
  final int? importedAt;
  const LegacyStatus({this.imported = false, this.importedAt});
  factory LegacyStatus.fromJson(Json j) =>
      LegacyStatus(imported: j['imported'] as bool? ?? false, importedAt: asInt(j['importedAt']));
}

/// `legacy_import` report.
class LegacyReport {
  final int tracks;
  final int albums;
  final int artists;
  final int playlists;
  final int userPlaylists;
  final int playlistTracks;
  final int history;
  final int pinned;
  final int legacyKeys;
  final int skipped;
  const LegacyReport({
    this.tracks = 0,
    this.albums = 0,
    this.artists = 0,
    this.playlists = 0,
    this.userPlaylists = 0,
    this.playlistTracks = 0,
    this.history = 0,
    this.pinned = 0,
    this.legacyKeys = 0,
    this.skipped = 0,
  });

  factory LegacyReport.fromJson(Json j) => LegacyReport(
        tracks: asInt(j['tracks']) ?? 0,
        albums: asInt(j['albums']) ?? 0,
        artists: asInt(j['artists']) ?? 0,
        playlists: asInt(j['playlists']) ?? 0,
        userPlaylists: asInt(j['userPlaylists']) ?? 0,
        playlistTracks: asInt(j['playlistTracks']) ?? 0,
        history: asInt(j['history']) ?? 0,
        pinned: asInt(j['pinned']) ?? 0,
        legacyKeys: asInt(j['legacyKeys']) ?? 0,
        skipped: asInt(j['skipped']) ?? 0,
      );
}

class LegacyApi {
  final Transport _t;
  const LegacyApi(this._t);

  Future<LegacyStatus> status() async => LegacyStatus.fromJson((await _t.call('legacy_status')) as Json);

  /// Imports a dump file written by the old window («Экспорт библиотеки»).
  Future<LegacyReport> importFile(String path) async =>
      LegacyReport.fromJson((await _t.call('legacy_import', {'path': path})) as Json);
}
