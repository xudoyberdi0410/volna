/// Screenshots of the browse screens with fixture data. Regenerate with
/// `flutter test --update-goldens test/goldens/browse_golden_test.dart`; the
/// screens are drawn inside the real app shell with the bundled Inter.
@Tags(['golden'])
library;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../support/browse_fixtures.dart';
import '../support/browse_harness.dart';
import '../support/test_app.dart';

void main() {
  setUpAll(loadAppFonts);

  const desktop = Size(1440, 900);
  const phone = Size(400, 860);

  Future<void> shot(WidgetTester tester, String location, String name,
      {Size size = desktop, Brightness brightness = Brightness.dark, Future<void> Function()? before}) async {
    // Real shadows; the binding checks the flag is back before the test ends.
    debugDisableShadows = false;
    try {
      final (t, _) = browseTransport();
      await pumpBrowse(tester, t, location,
          size: size, theme: brightness == Brightness.dark ? ThemeMode.dark : ThemeMode.light);
      if (before != null) {
        await before();
        await settle(tester);
      }
      await expectLater(find.byType(MaterialApp), matchesGoldenFile('browse/$name.png'));
    } finally {
      debugDisableShadows = true;
    }
  }

  testWidgets('home', (tester) => shot(tester, '/home', 'home'));
  testWidgets('search all', (tester) => shot(tester, '/search?q=земфира', 'search_all'));
  testWidgets('search songs', (tester) => shot(tester, '/search?q=земфира', 'search_songs', before: () async {
        await tester.tap(find.text('Треки').first);
      }));
  testWidgets('search suggestions', (tester) => shot(tester, '/search', 'search_suggestions', before: () async {
        await tester.enterText(find.byType(TextField), 'земф');
      }));
  testWidgets('album', (tester) => shot(tester, '/album/MPREb_prosti', 'album'));
  testWidgets('album light', (tester) => shot(tester, '/album/MPREb_prosti', 'album_light', brightness: Brightness.light));
  testWidgets('album phone', (tester) => shot(tester, '/album/MPREb_prosti', 'album_phone', size: phone));
  testWidgets('artist', (tester) => shot(tester, '/artist/UCzemfira', 'artist'));
  testWidgets('playlist', (tester) => shot(tester, '/playlist/RDCLAK5uy_rusrock', 'playlist'));
  testWidgets('library', (tester) => shot(tester, '/library', 'library'));
  testWidgets('library albums', (tester) => shot(tester, '/library?tab=1', 'library_albums'));
  testWidgets('library my playlists', (tester) => shot(tester, '/library?tab=4', 'library_playlists'));
  testWidgets('library phone', (tester) => shot(tester, '/library', 'library_phone', size: phone));
  testWidgets('user playlist', (tester) => shot(tester, '/library/playlist/pl-road', 'user_playlist'));
  testWidgets('history', (tester) => shot(tester, '/history', 'history'));
}
