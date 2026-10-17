import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Dark / light / system. A UI-only preference, so it lives in
/// shared_preferences (key `theme_mode`), not in the daemon.
class ThemeModeNotifier extends Notifier<ThemeMode> {
  static const key = 'theme_mode';

  @override
  ThemeMode build() {
    _load();
    return ThemeMode.system;
  }

  Future<void> _load() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final saved = prefs.getString(key);
      final mode = ThemeMode.values.where((m) => m.name == saved).firstOrNull;
      if (mode != null && ref.mounted) state = mode;
    } catch (_) {
      // No storage (tests, a broken prefs file): keep the system theme.
    }
  }

  Future<void> set(ThemeMode mode) async {
    state = mode;
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString(key, mode.name);
    } catch (_) {}
  }
}

final themeModeProvider = NotifierProvider<ThemeModeNotifier, ThemeMode>(ThemeModeNotifier.new);

const _coverColorsKey = 'volna.coverColors';

/// Tint the app with the current cover's colours (default on).
final coverColorsProvider =
    NotifierProvider<CoverColorsController, bool>(CoverColorsController.new);

class CoverColorsController extends Notifier<bool> {
  @override
  bool build() {
    _load();
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
