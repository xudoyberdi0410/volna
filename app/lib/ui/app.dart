import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../state/palette.dart';
import '../state/player.dart';
import '../state/theme.dart';
import 'router.dart';
import 'theme/app_theme.dart';

class VolnaApp extends ConsumerWidget {
  const VolnaApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Keep the player mirror alive for the whole app.
    ref.watch(playerViewProvider.select((_) => null));
    final mode = ref.watch(themeModeProvider);
    final palette = ref.watch(currentPaletteProvider);
    return MaterialApp.router(
      title: 'Volna',
      debugShowCheckedModeBanner: false,
      themeMode: mode,
      theme: buildTheme(Brightness.light, accent: accentFor(palette, Brightness.light)),
      darkTheme: buildTheme(Brightness.dark, accent: accentFor(palette, Brightness.dark)),
      themeAnimationDuration: const Duration(milliseconds: 900),
      themeAnimationCurve: Curves.easeInOut,
      routerConfig: ref.watch(routerProvider),
      locale: const Locale('ru'),
      supportedLocales: const [Locale('ru')],
      localizationsDelegates: GlobalMaterialLocalizations.delegates,
    );
  }
}
