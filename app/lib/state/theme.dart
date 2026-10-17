/// UI-only appearance prefs, stored by Flutter (`shared_preferences`), not
/// in the daemon settings (`docs/flutter-ui-plan.md` 3d).
///
/// - [themeModeProvider]: system / light / dark.
/// - [coverColorsProvider]: tint the app with the current cover's colours.
library;

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

const _themeKey = 'volna.themeMode';
const _coverColorsKey = 'volna.coverColors';

final themeModeProvider =
    NotifierProvider<ThemeModeController, ThemeMode>(ThemeModeController.new);

class ThemeModeController extends Notifier<ThemeMode> {
  @override
  ThemeMode build() {
    unawaited(_load());
    return ThemeMode.system;
  }

  Future<void> _load() async {
    try {
      final p = await SharedPreferences.getInstance();
      final v = p.getString(_themeKey);
      final mode = ThemeMode.values.where((m) => m.name == v).firstOrNull;
      if (mode != null && ref.mounted) state = mode;
    } catch (_) {
      // No prefs backend (tests without mocks): keep the default.
    }
  }

  Future<void> set(ThemeMode mode) async {
    state = mode;
    try {
      final p = await SharedPreferences.getInstance();
      await p.setString(_themeKey, mode.name);
    } catch (_) {}
  }
}

final coverColorsProvider =
    NotifierProvider<CoverColorsController, bool>(CoverColorsController.new);

class CoverColorsController extends Notifier<bool> {
  @override
  bool build() {
    unawaited(_load());
    return true;
  }

  Future<void> _load() async {
    try {
      final p = await SharedPreferences.getInstance();
      final v = p.getBool(_coverColorsKey);
      if (v != null && ref.mounted) state = v;
    } catch (_) {}
  }

  Future<void> set(bool on) async {
    state = on;
    try {
      final p = await SharedPreferences.getInstance();
      await p.setBool(_coverColorsKey, on);
    } catch (_) {}
  }
}
