/// Russian text helpers for the browse screens.
library;

import '../../api/models.dart';

/// `3:07`, `1:02:03`.
String formatDuration(int? seconds) {
  if (seconds == null || seconds < 0) return '';
  final h = seconds ~/ 3600, m = (seconds % 3600) ~/ 60, s = seconds % 60;
  String two(int v) => v.toString().padLeft(2, '0');
  return h > 0 ? '$h:${two(m)}:${two(s)}' : '$m:${two(s)}';
}

/// `1 ч 12 мин`, `42 мин`.
String formatLongDuration(int seconds) {
  final h = seconds ~/ 3600, m = (seconds % 3600) ~/ 60;
  if (h > 0) return m > 0 ? '$h ч $m мин' : '$h ч';
  return '${m < 1 ? 1 : m} мин';
}

/// Russian plural: `plural(5, 'трек', 'трека', 'треков')` → `5 треков`.
String plural(int n, String one, String few, String many) {
  final m10 = n % 10, m100 = n % 100;
  final w = (m10 == 1 && m100 != 11)
      ? one
      : (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14))
          ? few
          : many;
  return '$n $w';
}

String tracksCount(int n) => plural(n, 'трек', 'трека', 'треков');

String artistNames(List<ArtistRef> a) => a.map((x) => x.name).where((n) => n.isNotEmpty).join(', ');

String albumKindLabel(AlbumKind k) => switch (k) {
      AlbumKind.album => 'Альбом',
      AlbumKind.single => 'Сингл',
      AlbumKind.ep => 'EP',
    };

/// `Сегодня, 14:05`, `Вчера`, `12 марта`, `12 марта 2024`.
String formatDay(DateTime t, {DateTime? now}) {
  final n = now ?? DateTime.now();
  final d = DateTime(t.year, t.month, t.day);
  final today = DateTime(n.year, n.month, n.day);
  final diff = today.difference(d).inDays;
  if (diff == 0) return 'Сегодня';
  if (diff == 1) return 'Вчера';
  const months = [
    'января', 'февраля', 'марта', 'апреля', 'мая', 'июня', //
    'июля', 'августа', 'сентября', 'октября', 'ноября', 'декабря',
  ];
  final base = '${t.day} ${months[t.month - 1]}';
  return t.year == n.year ? base : '$base ${t.year}';
}

String formatTime(DateTime t) =>
    '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}';
