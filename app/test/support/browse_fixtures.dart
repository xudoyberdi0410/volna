/// Realistic daemon replies (wire JSON, ytm serde shapes) for the browse
/// screens' tests and goldens.
library;

import 'package:volna/ipc/transport.dart';

import 'fake_library.dart';
import 'fake_png.dart';
import 'fake_transport.dart';
import 'test_app.dart' show kTestNow;

typedef J = Map<String, Object?>;

List<J> thumbs(String seed, [int w = 544]) => [
      {'url': 'https://lh3.googleusercontent.com/$seed=w120-h120', 'width': 120, 'height': 120},
      {'url': 'https://lh3.googleusercontent.com/$seed=w$w-h$w', 'width': w, 'height': w},
    ];

J artistRef(String name, [String? id]) => {'id': id, 'name': name};

J track(String id, String title, List<J> artists,
        {J? album, int? dur, bool explicit = false, String kind = 'song', String? views, String? cover, int? year}) =>
    {
      'video_id': id,
      'title': title,
      'artists': artists,
      'album': album,
      'duration_s': dur,
      'thumbnails': cover == null ? <J>[] : thumbs(cover, 226),
      'explicit': explicit,
      'kind': kind,
      'year': year,
      'views': views,
      'playable': true,
    };

J albumSummary(String id, String title, List<J> artists, {String kind = 'album', int? year, bool explicit = false}) => {
      'browse_id': id,
      'playlist_id': null,
      'title': title,
      'kind': kind,
      'artists': artists,
      'year': year,
      'thumbnails': thumbs(id),
      'explicit': explicit,
    };

J artistSummary(String id, String name, String? audience) =>
    {'id': id, 'name': name, 'thumbnails': thumbs(id), 'audience': audience};

J playlistSummary(String id, String title, {J? author, int? count, String? subtitle}) => {
      'id': id,
      'title': title,
      'author': author,
      'thumbnails': thumbs(id),
      'track_count': count,
      'subtitle': subtitle,
    };

J typed(String type, J m) => {'type': type, ...m};

final zemfira = artistRef('Земфира', 'UCzemfira');
final kino = artistRef('Кино', 'UCkino');
final splin = artistRef('Сплин', 'UCsplin');
final daft = artistRef('Daft Punk', 'UCdaftpunk');
final monetochka = artistRef('Монеточка', 'UCmonetochka');

final albumRef = {'id': 'MPREb_prosti', 'name': 'Прости меня моя любовь'};

final albumJson = <String, Object?>{
  'browse_id': 'MPREb_prosti',
  'playlist_id': 'OLAK5uy_prosti',
  'title': 'Прости меня моя любовь',
  'kind': 'album',
  'artists': [zemfira],
  'year': 2000,
  'description':
      'Второй студийный альбом Земфиры, вышедший в марте 2000 года. Записан в Лондоне и Москве; '
          'в него вошли «Искала», «Хочешь?» и «П.М.М.Л.». Один из самых продаваемых русских рок-альбомов.',
  'thumbnails': thumbs('MPREb_prosti'),
  'explicit': false,
  'track_count': 12,
  'duration_s': 2591,
  'tracks': [
    for (final (i, (t, d)) in const [
      ('Приговорённый', 192),
      ('Хочешь?', 248),
      ('Искала', 214),
      ('Сигареты', 187),
      ('П.М.М.Л.', 236),
      ('Кому-то с тобой', 203),
      ('Минус 140', 205),
      ('Ненавижу', 221),
      ('Город', 176),
      ('Полуфабрикаты', 199),
      ('Доказано', 230),
      ('Webgirl', 280),
    ].indexed)
      track('prosti$i', t, [zemfira], album: albumRef, dur: d),
  ],
  'related': [
    albumSummary('MPREb_zemfira', 'Земфира', [zemfira], year: 1999),
    albumSummary('MPREb_14', 'Четырнадцать недель тишины', [zemfira], year: 2002),
    albumSummary('MPREb_vendetta', 'Вендетта', [zemfira], year: 2005),
    albumSummary('MPREb_spasibo', 'Спасибо', [zemfira], year: 2007),
    albumSummary('MPREb_zhit', 'Жить в твоей голове', [zemfira], year: 2013),
    albumSummary('MPREb_bordo', 'Бордерлайн', [zemfira], year: 2021),
  ],
};

final artistJson = <String, Object?>{
  'id': 'UCzemfira',
  'name': 'Земфира',
  'description':
      'Земфира Рамазанова — российская рок-певица, автор песен, композитор и продюсер. '
          'Дебютный альбом 1999 года сделал её одной из главных фигур русского рока.',
  'thumbnails': thumbs('UCzemfira', 800),
  'subscribers': '1,2 млн подписчиков',
  'shuffle_playlist_id': 'RDAOzemfira',
  'radio_playlist_id': 'RDEMzemfira',
  'top_songs': [
    track('z1', 'Хочешь?', [zemfira], album: albumRef, dur: 248, cover: 'MPREb_prosti', views: '120 млн'),
    track('z2', 'Искала', [zemfira], album: albumRef, dur: 214, cover: 'MPREb_prosti'),
    track('z3', 'Жить в твоей голове', [zemfira],
        album: {'id': 'MPREb_zhit', 'name': 'Жить в твоей голове'}, dur: 302, cover: 'MPREb_zhit'),
    track('z4', 'Ариведерчи', [zemfira], album: {'id': 'MPREb_zemfira', 'name': 'Земфира'}, dur: 236, cover: 'MPREb_zemfira'),
    track('z5', 'Почему', [zemfira], album: {'id': 'MPREb_vendetta', 'name': 'Вендетта'}, dur: 262, cover: 'MPREb_vendetta'),
    track('z6', 'Бесконечность', [zemfira], album: {'id': 'MPREb_14', 'name': 'Четырнадцать недель тишины'}, dur: 240, cover: 'MPREb_14'),
  ],
  'top_songs_playlist_id': 'OLAK5uy_top',
  'albums': [
    albumSummary('MPREb_bordo', 'Бордерлайн', [zemfira], year: 2021),
    albumSummary('MPREb_zhit', 'Жить в твоей голове', [zemfira], year: 2013),
    albumSummary('MPREb_spasibo', 'Спасибо', [zemfira], year: 2007),
    albumSummary('MPREb_vendetta', 'Вендетта', [zemfira], year: 2005),
    albumSummary('MPREb_14', 'Четырнадцать недель тишины', [zemfira], year: 2002),
    albumSummary('MPREb_prosti', 'Прости меня моя любовь', [zemfira], year: 2000),
    albumSummary('MPREb_zemfira', 'Земфира', [zemfira], year: 1999),
  ],
  'singles': [
    albumSummary('MPREb_s1', 'Остин', [zemfira], kind: 'single', year: 2020),
    albumSummary('MPREb_s2', 'Крым', [zemfira], kind: 'single', year: 2020),
    albumSummary('MPREb_s3', 'Таблетки', [zemfira], kind: 'single', year: 2021),
  ],
  'videos': [],
  'playlists': [],
  'related': [
    artistSummary('UCmonetochka', 'Монеточка', '890 тыс. подписчиков'),
    artistSummary('UCsplin', 'Сплин', '650 тыс. подписчиков'),
    artistSummary('UCkino', 'Кино', '1,1 млн подписчиков'),
    artistSummary('UCnautilus', 'Наутилус Помпилиус', '420 тыс. подписчиков'),
    artistSummary('UCbi2', 'Би-2', '700 тыс. подписчиков'),
  ],
};

final playlistJson = <String, Object?>{
  'id': 'RDCLAK5uy_rusrock',
  'title': 'Русский рок: главное',
  'author': {'id': null, 'name': 'YouTube Music'},
  'description': 'От «Кино» до «Сплина»: песни, на которых выросли поколения.',
  'thumbnails': thumbs('RDCLAK5uy_rusrock'),
  'year': 2026,
  'track_count': 60,
  'tracks': [
    track('k1', 'Группа крови', [kino], album: {'id': 'MPREb_gk', 'name': 'Группа крови'}, dur: 285, cover: 'MPREb_gk'),
    track('s1', 'Выхода нет', [splin], album: {'id': 'MPREb_25', 'name': '25-й кадр'}, dur: 236, cover: 'MPREb_25'),
    track('z1', 'Хочешь?', [zemfira], album: albumRef, dur: 248, cover: 'MPREb_prosti'),
    track('k2', 'Звезда по имени Солнце', [kino], album: {'id': 'MPREb_zvezda', 'name': 'Звезда по имени Солнце'}, dur: 225, cover: 'MPREb_zvezda'),
    track('s2', 'Романс', [splin], album: {'id': 'MPREb_gr', 'name': 'Гранатовый альбом'}, dur: 196, cover: 'MPREb_gr'),
    track('k3', 'Кукушка', [kino], album: {'id': 'MPREb_black', 'name': 'Чёрный альбом'}, dur: 400, cover: 'MPREb_black'),
    track('m1', 'Каждый раз', [monetochka], album: {'id': 'MPREb_raskr', 'name': 'Раскраски для взрослых'}, dur: 210, cover: 'MPREb_raskr', explicit: true),
    track('s3', 'Моё сердце', [splin], album: {'id': 'MPREb_gr', 'name': 'Гранатовый альбом'}, dur: 254, cover: 'MPREb_gr'),
    track('k4', 'Перемен', [kino], album: {'id': 'MPREb_last', 'name': 'Последний герой'}, dur: 296, cover: 'MPREb_last'),
    track('z2', 'Искала', [zemfira], album: albumRef, dur: 214, cover: 'MPREb_prosti'),
  ],
  'continuation': 'CONT-1',
};

final playlistPage2 = <String, Object?>{
  'tracks': [
    track('k5', 'Хочу перемен', [kino], dur: 290, cover: 'MPREb_last'),
    track('s4', 'Орбит без сахара', [splin], dur: 190, cover: 'MPREb_25'),
  ],
  'continuation': null,
};

final searchAllJson = <String, Object?>{
  'top_result': typed('artist', artistSummary('UCzemfira', 'Земфира', '1,2 млн подписчиков')),
  'items': [
    typed('track', track('z1', 'Хочешь?', [zemfira], album: albumRef, dur: 248, cover: 'MPREb_prosti')),
    typed('track', track('z2', 'Искала', [zemfira], album: albumRef, dur: 214, cover: 'MPREb_prosti')),
    typed('track', track('z3', 'Жить в твоей голове', [zemfira],
        album: {'id': 'MPREb_zhit', 'name': 'Жить в твоей голове'}, dur: 302, cover: 'MPREb_zhit')),
    typed('track', track('z4', 'Ариведерчи', [zemfira], album: {'id': 'MPREb_zemfira', 'name': 'Земфира'}, dur: 236, cover: 'MPREb_zemfira')),
    typed('album', albumSummary('MPREb_prosti', 'Прости меня моя любовь', [zemfira], year: 2000)),
    typed('album', albumSummary('MPREb_bordo', 'Бордерлайн', [zemfira], year: 2021)),
    typed('album', albumSummary('MPREb_zhit', 'Жить в твоей голове', [zemfira], year: 2013)),
    typed('album', albumSummary('MPREb_14', 'Четырнадцать недель тишины', [zemfira], year: 2002)),
    typed('artist', artistSummary('UCzemfira', 'Земфира', '1,2 млн подписчиков')),
    typed('artist', artistSummary('UCmonetochka', 'Монеточка', '890 тыс. подписчиков')),
    typed('track', track('zv1', 'Земфира — Хочешь? (клип)', [zemfira], dur: 260, kind: 'official_video', cover: 'zv1', views: '45 млн просмотров')),
    typed('playlist', playlistSummary('RDCLAK5uy_rusrock', 'Русский рок: главное', author: {'id': null, 'name': 'YouTube Music'}, count: 60)),
    typed('playlist', playlistSummary('PLzemfira', 'Земфира: лучшее', author: {'id': null, 'name': 'Лена'}, count: 34)),
  ],
  'continuation': 'SEARCH-CONT',
};

J searchSongsJson(int page) => {
      'top_result': null,
      'items': [
        for (var i = 0; i < 12; i++)
          typed('track', track('ss$page-$i', ['Хочешь?', 'Искала', 'Сигареты', 'Ненавижу', 'Город', 'Почему'][i % 6] + (page > 0 ? ' (live)' : ''),
              [zemfira], album: albumRef, dur: 180 + i * 7, cover: 'MPREb_${['prosti', 'zhit', 'bordo'][i % 3]}')),
      ],
      'continuation': page < 2 ? 'SONGS-$page' : null,
    };

final suggestionsJson = <String, Object?>{
  'queries': ['земфира', 'земфира хочешь', 'земфира искала', 'земфира бордерлайн'],
  'items': [
    typed('artist', artistSummary('UCzemfira', 'Земфира', '1,2 млн подписчиков')),
    typed('album', albumSummary('MPREb_bordo', 'Бордерлайн', [zemfira], year: 2021)),
  ],
};

final homeJson = <J>[
  {
    'title': 'Быстрый выбор',
    'strapline': null,
    'items': [
      for (final t in (playlistJson['tracks'] as List).cast<J>().take(8)) typed('track', t),
    ],
  },
  {
    'title': 'Новые альбомы и синглы',
    'strapline': null,
    'items': [
      typed('album', albumSummary('MPREb_bordo', 'Бордерлайн', [zemfira], year: 2021)),
      typed('album', albumSummary('MPREb_ram', 'Random Access Memories', [daft], year: 2013)),
      typed('album', albumSummary('MPREb_raskr', 'Раскраски для взрослых', [monetochka], year: 2018, explicit: true)),
      typed('album', albumSummary('MPREb_gr', 'Гранатовый альбом', [splin], year: 1998)),
      typed('album', albumSummary('MPREb_gk', 'Группа крови', [kino], year: 1988)),
      typed('album', albumSummary('MPREb_disc', 'Discovery', [daft], year: 2001)),
      typed('album', albumSummary('MPREb_s2', 'Крым', [zemfira], kind: 'single', year: 2020)),
    ],
  },
  {
    'title': 'Популярные исполнители',
    'strapline': null,
    'items': [
      typed('artist', artistSummary('UCzemfira', 'Земфира', '1,2 млн подписчиков')),
      typed('artist', artistSummary('UCkino', 'Кино', '1,1 млн подписчиков')),
      typed('artist', artistSummary('UCsplin', 'Сплин', '650 тыс. подписчиков')),
      typed('artist', artistSummary('UCdaftpunk', 'Daft Punk', '9,8 млн подписчиков')),
      typed('artist', artistSummary('UCmonetochka', 'Монеточка', '890 тыс. подписчиков')),
    ],
  },
  {
    'title': 'Подборки для вас',
    'strapline': 'Под настроение',
    'items': [
      typed('playlist', playlistSummary('RDCLAK5uy_rusrock', 'Русский рок: главное', subtitle: 'Кино, Сплин, Земфира')),
      typed('playlist', playlistSummary('RDCLAK5uy_focus', 'Фокус', subtitle: 'Спокойная электроника')),
      typed('playlist', playlistSummary('RDCLAK5uy_run', 'Пробежка', subtitle: 'Daft Punk, Justice, The Prodigy')),
      typed('playlist', playlistSummary('RDCLAK5uy_chill', 'Чилл вечером', subtitle: 'Тихие песни')),
      typed('playlist', playlistSummary('RDCLAK5uy_90', 'Хиты 90-х', subtitle: 'Мумий Тролль, Би-2, Земфира')),
    ],
  },
];

final moodsJson = <J>[
  {
    'title': 'Настроения',
    'moods': [
      {'title': 'Бодрое', 'params': 'p1', 'color': 0xFFE57373},
      {'title': 'Для тренировки', 'params': 'p2', 'color': 0xFF81C784},
      {'title': 'Спокойствие', 'params': 'p3', 'color': 0xFF64B5F6},
      {'title': 'Для вечеринки', 'params': 'p4', 'color': 0xFFBA68C8},
      {'title': 'Сосредоточенность', 'params': 'p5', 'color': 0xFFFFB74D},
    ],
  },
  {
    'title': 'Жанры',
    'moods': [
      {'title': 'Рок', 'params': 'g1', 'color': 0xFF90A4AE},
      {'title': 'Электроника', 'params': 'g2', 'color': 0xFF4DD0E1},
      {'title': 'Хип-хоп', 'params': 'g3', 'color': 0xFFFFD54F},
    ],
  },
];

/// A [FakeTransport] answering every browse command with the fixtures and
/// an in-memory library seeded with likes, playlists, pins and history.
(FakeTransport, FakeLibrary) browseTransport({bool images = true}) {
  final t = FakeTransport();
  t.on('catalog_album', (a) => albumJson);
  t.on('catalog_artist', (a) => artistJson);
  t.on('catalog_playlist', (a) => playlistJson);
  t.on('catalog_playlist_more', (a) => playlistPage2);
  t.on('catalog_search', (a) => a['filter'] == 'all' ? searchAllJson : searchSongsJson(0));
  t.on('catalog_search_more', (a) => searchSongsJson(int.parse((a['continuation'] as String).split('-').last) + 1));
  t.on('catalog_suggestions', (a) => suggestionsJson);
  t.on('catalog_home', (a) => homeJson);
  t.on('catalog_moods', (a) => moodsJson);
  t.on('catalog_mood', (a) => [homeJson[3]]);
  FakePlayer(t);

  final lib = FakeLibrary(t);
  final pl = (playlistJson['tracks'] as List).cast<J>();
  final at = lib.now;
  lib.likes['track']!.addAll([
    for (final x in [pl[2], pl[0], pl[6], pl[1], pl[4], pl[9]]) {'item': typed('track', x), 'addedAt': at},
  ]);
  lib.likes['album']!.addAll([
    for (final x in (homeJson[1]['items'] as List).cast<J>().take(5)) {'item': x, 'addedAt': at},
  ]);
  lib.likes['artist']!.addAll([
    for (final x in (homeJson[2]['items'] as List).cast<J>()) {'item': x, 'addedAt': at},
  ]);
  lib.likes['playlist']!.add({'item': (searchAllJson['items'] as List).last, 'addedAt': at});
  lib.playlists['pl-road'] = {'name': 'В дорогу', 'description': 'Для длинных поездок'};
  lib.playlistTracks['pl-road'] = [pl[0], pl[1], pl[3], pl[5], pl[7], pl[8]];
  lib.playlists['pl-night'] = {'name': 'Ночью', 'description': ''};
  lib.playlistTracks['pl-night'] = [pl[4], pl[6]];
  lib.pins.addAll([
    {'kind': 'user_playlist', 'id': 'pl-road', 'title': 'В дорогу', 'thumbnail': null, 'pinnedAt': at},
    {'kind': 'artist', 'id': 'UCzemfira', 'title': 'Земфира', 'thumbnail': 'https://lh3.googleusercontent.com/UCzemfira=w544-h544', 'pinnedAt': at},
    {'kind': 'album', 'id': 'MPREb_prosti', 'title': 'Прости меня моя любовь', 'thumbnail': 'https://lh3.googleusercontent.com/MPREb_prosti=w544-h544', 'pinnedAt': at},
  ]);
  final now = kTestNow;
  const hour = 3600 * 1000;
  lib.history.addAll([
    {'track': pl[3], 'playedAt': now - hour ~/ 4},
    {'track': pl[0], 'playedAt': now - hour},
    {'track': pl[6], 'playedAt': now - 2 * hour},
    {'track': pl[1], 'playedAt': now - 26 * hour},
    {'track': pl[9], 'playedAt': now - 27 * hour},
    {'track': pl[4], 'playedAt': now - 5 * 24 * hour},
  ]);
  if (images) {
    t.protoFallback = (uri) {
      final u = Uri.parse(uri).queryParameters['u'] ?? uri;
      return ProtoResponse(200, const [('content-type', 'image/png')], fakeCover(u.split('=').first));
    };
  }
  return (t, lib);
}
