/// The app's look: Inter, quiet surfaces, one accent taken from the cover.
///
/// Surfaces stay neutral (the animated background carries the colour);
/// the accent tints controls, the progress bar and the selected nav item.
library;

import 'package:flutter/material.dart';

import '../../api/color.dart';

/// Accent used before any cover is known.
const kDefaultSeed = Color(0xff7d6bf0);

const kRadiusS = 8.0;
const kRadiusM = 14.0;
const kRadiusL = 22.0;

/// Base canvas colours (under the background gradient).
Color canvasColor(Brightness b) =>
    b == Brightness.dark ? const Color(0xff0c0c0f) : const Color(0xfff6f5f2);

/// [c] lifted or pressed so it reads as an accent on [b] backgrounds.
Color readableAccent(Color c, Brightness b) {
  final hsl = HSLColor.fromColor(c);
  final s = hsl.saturation < 0.25 ? hsl.saturation : (hsl.saturation * 1.05).clamp(0.0, 0.85);
  final l = b == Brightness.dark
      ? hsl.lightness.clamp(0.62, 0.78)
      : hsl.lightness.clamp(0.32, 0.46);
  return hsl.withSaturation(s).withLightness(l).toColor();
}

/// The accent for [palette] (or the default) on [b].
Color accentFor(Palette? palette, Brightness b) {
  if (palette == null) return readableAccent(kDefaultSeed, b);
  // A grey accent (black-and-white cover) falls back to the dominant hue.
  final a = HSLColor.fromColor(palette.accent);
  final d = HSLColor.fromColor(palette.dominant);
  final pick = a.saturation >= 0.18 || d.saturation < a.saturation ? palette.accent : palette.dominant;
  return readableAccent(pick, b);
}

ThemeData buildTheme(Brightness b, {Color? accent}) {
  final dark = b == Brightness.dark;
  final acc = accent ?? readableAccent(kDefaultSeed, b);
  final base = ColorScheme.fromSeed(
    seedColor: acc,
    brightness: b,
    dynamicSchemeVariant: DynamicSchemeVariant.tonalSpot,
  );
  final onSurface = dark ? const Color(0xfff4f3f6) : const Color(0xff16151a);
  final scheme = base.copyWith(
    primary: acc,
    onPrimary: ThemeData.estimateBrightnessForColor(acc) == Brightness.dark
        ? Colors.white
        : const Color(0xff111014),
    surface: canvasColor(b),
    onSurface: onSurface,
    onSurfaceVariant: onSurface.withValues(alpha: dark ? 0.62 : 0.6),
    outlineVariant: onSurface.withValues(alpha: 0.1),
    surfaceContainerLowest: onSurface.withValues(alpha: 0.02),
    surfaceContainerLow: onSurface.withValues(alpha: 0.04),
    surfaceContainer: onSurface.withValues(alpha: 0.06),
    surfaceContainerHigh: onSurface.withValues(alpha: 0.09),
    surfaceContainerHighest: onSurface.withValues(alpha: 0.12),
  );

  final text = _textTheme(onSurface);
  return ThemeData(
    useMaterial3: true,
    brightness: b,
    colorScheme: scheme,
    fontFamily: 'Inter',
    textTheme: text,
    scaffoldBackgroundColor: Colors.transparent,
    canvasColor: canvasColor(b),
    splashFactory: InkSparkle.splashFactory,
    visualDensity: VisualDensity.standard,
    iconTheme: IconThemeData(color: onSurface, size: 22),
    dividerTheme: DividerThemeData(color: scheme.outlineVariant, thickness: 1, space: 1),
    tooltipTheme: TooltipThemeData(
      waitDuration: const Duration(milliseconds: 500),
      decoration: BoxDecoration(
        color: dark ? const Color(0xff2a2930) : const Color(0xff222126),
        borderRadius: BorderRadius.circular(kRadiusS),
      ),
      textStyle: text.bodySmall?.copyWith(color: Colors.white),
    ),
    snackBarTheme: SnackBarThemeData(
      behavior: SnackBarBehavior.floating,
      backgroundColor: dark ? const Color(0xff26252b) : const Color(0xff222126),
      contentTextStyle: text.bodyMedium?.copyWith(color: Colors.white),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(kRadiusM)),
      width: 420,
    ),
    iconButtonTheme: IconButtonThemeData(
      style: IconButton.styleFrom(
        foregroundColor: onSurface,
        hoverColor: onSurface.withValues(alpha: 0.07),
        highlightColor: onSurface.withValues(alpha: 0.1),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        textStyle: text.labelLarge,
        padding: const EdgeInsets.symmetric(horizontal: 22, vertical: 16),
        shape: const StadiumBorder(),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        foregroundColor: onSurface.withValues(alpha: 0.8),
        textStyle: text.labelLarge,
        shape: const StadiumBorder(),
      ),
    ),
    sliderTheme: SliderThemeData(
      trackHeight: 3,
      activeTrackColor: onSurface.withValues(alpha: 0.85),
      inactiveTrackColor: onSurface.withValues(alpha: 0.16),
      thumbColor: onSurface,
      overlayShape: SliderComponentShape.noOverlay,
      thumbShape: const RoundSliderThumbShape(enabledThumbRadius: 5, elevation: 0),
    ),
    pageTransitionsTheme: const PageTransitionsTheme(builders: {
      TargetPlatform.linux: FadeForwardsPageTransitionsBuilder(),
      TargetPlatform.windows: FadeForwardsPageTransitionsBuilder(),
      TargetPlatform.android: FadeForwardsPageTransitionsBuilder(),
    }),
  );
}

TextTheme _textTheme(Color c) {
  TextStyle s(double size, FontWeight w, {double ls = 0, double h = 1.3}) => TextStyle(
        fontFamily: 'Inter',
        fontSize: size,
        fontWeight: w,
        letterSpacing: ls,
        height: h,
        color: c,
      );
  return TextTheme(
    displayLarge: s(56, FontWeight.w700, ls: -2.0, h: 1.05),
    displayMedium: s(44, FontWeight.w700, ls: -1.4, h: 1.08),
    displaySmall: s(34, FontWeight.w700, ls: -0.9, h: 1.12),
    headlineLarge: s(30, FontWeight.w700, ls: -0.7, h: 1.15),
    headlineMedium: s(26, FontWeight.w700, ls: -0.55, h: 1.18),
    headlineSmall: s(22, FontWeight.w600, ls: -0.35, h: 1.22),
    titleLarge: s(19, FontWeight.w600, ls: -0.25, h: 1.25),
    titleMedium: s(15, FontWeight.w600, ls: -0.1, h: 1.3),
    titleSmall: s(13.5, FontWeight.w600, ls: -0.05, h: 1.3),
    bodyLarge: s(15, FontWeight.w400, ls: -0.1, h: 1.45),
    bodyMedium: s(13.5, FontWeight.w400, ls: -0.05, h: 1.4),
    bodySmall: s(12, FontWeight.w400, h: 1.35),
    labelLarge: s(13.5, FontWeight.w600, ls: -0.05),
    labelMedium: s(12, FontWeight.w500),
    labelSmall: s(11, FontWeight.w600, ls: 0.6),
  );
}

/// `m:ss` (or `h:mm:ss`).
String formatDuration(double? seconds) {
  if (seconds == null || seconds.isNaN || seconds < 0) return '–:––';
  final t = seconds.floor();
  final h = t ~/ 3600, m = (t % 3600) ~/ 60, s = t % 60;
  final ss = s.toString().padLeft(2, '0');
  return h > 0 ? '$h:${m.toString().padLeft(2, '0')}:$ss' : '$m:$ss';
}
