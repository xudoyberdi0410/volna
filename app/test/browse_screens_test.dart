import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:volna/ui/browse/routes.dart';

import 'support/browse_fixtures.dart';
import 'support/browse_harness.dart';

Future<void> pumpFor(WidgetTester tester, [int frames = 6]) async {
  for (var i = 0; i < frames; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

Future<GoRouter> mount(WidgetTester tester, String location, {Size size = const Size(1280, 900)}) async {
  setSize(tester, size);
  final router = GoRouter(initialLocation: location, routes: browseRoutes);
  addTearDown(router.dispose);
  return router;
}

void main() {
  testWidgets('search: suggestions, filter chips, paging with the same filter', (tester) async {
    final (t, _) = browseTransport(images: false);
    final router = await mount(tester, '/search');
    await tester.pumpWidget(BrowseTestApp(transport: t, location: '/search', router: router));
    await pumpFor(tester);
    expect(find.text('Что послушаем?'), findsOneWidget);

    await tester.enterText(find.byType(TextField), 'земф');
    await pumpFor(tester);
    expect(t.callsOf('catalog_suggestions').single.args['input'], 'земф');
    expect(find.text('земфира хочешь'), findsOneWidget);

    await tester.tap(find.text('земфира хочешь'));
    await pumpFor(tester);
    final search = t.callsOf('catalog_search').single.args;
    expect(search, {'query': 'земфира хочешь', 'filter': 'all'});
    expect(find.text('Лучший результат'), findsOneWidget);
    expect(find.text('Прости меня моя любовь'), findsWidgets);

    await tester.tap(find.widgetWithText(ChoiceChip, 'Треки'));
    await pumpFor(tester);
    expect(t.callsOf('catalog_search').last.args['filter'], 'songs');
    await tester.drag(find.byType(CustomScrollView), const Offset(0, -3000));
    await pumpFor(tester);
    final more = t.callsOf('catalog_search_more');
    expect(more, isNotEmpty);
    expect(more.first.args, {'continuation': 'SONGS-0', 'filter': 'songs'});
  });

  testWidgets('album: a row plays the album from it; like and play next', (tester) async {
    final (t, _) = browseTransport(images: false);
    final router = await mount(tester, '/album/MPREb_prosti');
    await tester.pumpWidget(BrowseTestApp(transport: t, location: '', router: router));
    await pumpFor(tester);
    expect(find.text('Прости меня моя любовь'), findsWidgets);
    expect(t.callsOf('catalog_album').single.args['id'], 'MPREb_prosti');

    await tester.tap(find.text('Искала'));
    await pumpFor(tester);
    final q = t.callsOf('player_set_queue').single.args;
    expect(q['startIndex'], 2);
    expect(q['play'], true);
    final tracks = (q['tracks'] as List).cast<Map>();
    expect(tracks, hasLength(12));
    // Covers and album links come from the album when tracks lack them.
    expect(tracks[2]['cover'], contains('MPREb_prosti'));
    expect((tracks[2]['album'] as Map)['id'], 'MPREb_prosti');
    expect((tracks[2]['ui'] as Map)['video_id'], 'prosti2');

    await tester.tap(find.byTooltip('В любимые').first);
    await pumpFor(tester);
    final like = t.callsOf('library_like').single.args;
    expect(like['kind'], 'album');
    expect((like['item'] as Map)['browse_id'], 'MPREb_prosti');
    expect(find.byTooltip('Убрать из любимых'), findsWidgets);

    await tester.tap(find.byTooltip('Ещё').at(1)); // first track row menu
    await pumpFor(tester);
    await tester.tap(find.text('Играть следующим'));
    await pumpFor(tester);
    expect(t.callsOf('player_enqueue').single.args['at'], 'next');
    expect(find.text('Сыграет следующим'), findsOneWidget);
  });

  testWidgets('playlist pages in with catalog_playlist_more', (tester) async {
    final (t, _) = browseTransport(images: false);
    final router = await mount(tester, '/playlist/RDCLAK5uy_rusrock', size: const Size(1280, 2000));
    await tester.pumpWidget(BrowseTestApp(transport: t, location: '', router: router));
    await pumpFor(tester, 10);
    expect(t.callsOf('catalog_playlist_more').single.args['continuation'], 'CONT-1');
    expect(find.text('Орбит без сахара'), findsOneWidget);
  });

  testWidgets('library: liked tracks follow library_changed', (tester) async {
    final (t, lib) = browseTransport(images: false);
    final router = await mount(tester, '/library');
    await tester.pumpWidget(BrowseTestApp(transport: t, location: '', router: router));
    await pumpFor(tester);
    expect(find.text('6 треков'), findsOneWidget);
    expect(find.text('В дорогу'), findsOneWidget, reason: 'pinned');

    lib.likes['track']!.removeAt(0);
    t.emit('library_changed', {'kind': 'track'});
    await pumpFor(tester);
    expect(find.text('5 треков'), findsOneWidget);

    await tester.tap(find.text('Мои плейлисты'));
    await pumpFor(tester);
    expect(find.text('Ночью'), findsOneWidget);
    await tester.tap(find.text('Ночью'));
    await pumpFor(tester);
    expect(router.state.uri.path, '/library/playlist/pl-night');
  });

  testWidgets('user playlist: rename and remove a track', (tester) async {
    final (t, lib) = browseTransport(images: false);
    final router = await mount(tester, '/library/playlist/pl-road');
    await tester.pumpWidget(BrowseTestApp(transport: t, location: '', router: router));
    await pumpFor(tester);
    expect(find.text('В дорогу'), findsOneWidget);

    await tester.tap(find.byTooltip('Переименовать'));
    await pumpFor(tester);
    await tester.enterText(find.byType(TextField), 'В пути');
    await tester.tap(find.text('Сохранить'));
    await pumpFor(tester);
    expect(t.callsOf('playlist_rename').single.args, {'id': 'pl-road', 'name': 'В пути'});
    expect(find.text('В пути'), findsOneWidget);

    await tester.tap(find.byTooltip('Ещё').at(1));
    await pumpFor(tester);
    await tester.tap(find.text('Убрать из плейлиста'));
    await pumpFor(tester);
    expect(t.callsOf('playlist_remove').single.args, {'id': 'pl-road', 'videoIds': ['k1']});
    expect(lib.playlistTracks['pl-road'], hasLength(5));
    expect(find.text('Группа крови'), findsNothing);
  });

  testWidgets('add to playlist from a track menu', (tester) async {
    final (t, lib) = browseTransport(images: false);
    final router = await mount(tester, '/album/MPREb_prosti');
    await tester.pumpWidget(BrowseTestApp(transport: t, location: '', router: router));
    await pumpFor(tester);
    await tester.tap(find.byTooltip('Ещё').at(1));
    await pumpFor(tester);
    await tester.tap(find.text('Добавить в плейлист…'));
    await pumpFor(tester);
    await tester.tap(find.text('Ночью'));
    await pumpFor(tester);
    final add = t.callsOf('playlist_add').single.args;
    expect(add['id'], 'pl-night');
    expect(((add['tracks'] as List).single as Map)['video_id'], 'prosti0');
    expect(lib.playlistTracks['pl-night'], hasLength(3));
    expect(find.text('Добавлено в «Ночью»'), findsOneWidget);
  });

  testWidgets('history groups by day and clears', (tester) async {
    final (t, lib) = browseTransport(images: false);
    final router = await mount(tester, '/history');
    await tester.pumpWidget(BrowseTestApp(transport: t, location: '', router: router));
    await pumpFor(tester);
    expect(find.text('Сегодня'), findsOneWidget);
    expect(find.text('Вчера'), findsOneWidget);
    await tester.tap(find.text('Очистить'));
    await pumpFor(tester);
    await tester.tap(find.widgetWithText(FilledButton, 'Очистить'));
    await pumpFor(tester);
    expect(t.callsOf('history_clear'), hasLength(1));
    expect(lib.history, isEmpty);
    expect(find.text('История пуста'), findsOneWidget);
  });

  testWidgets('a failed page shows an error with retry', (tester) async {
    final (t, _) = browseTransport(images: false);
    var fail = true;
    t.on('catalog_artist', (a) => fail ? throw Exception('boom') : artistJson);
    final router = await mount(tester, '/artist/UCzemfira');
    await tester.pumpWidget(BrowseTestApp(transport: t, location: '', router: router));
    await pumpFor(tester);
    expect(find.text('Повторить'), findsOneWidget);
    fail = false;
    await tester.tap(find.text('Повторить'));
    await pumpFor(tester);
    expect(find.text('Популярные треки'), findsOneWidget);
  });
}
