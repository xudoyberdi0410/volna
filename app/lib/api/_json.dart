/// Small helpers for hand-written `fromJson`.
library;

typedef Json = Map<String, Object?>;

int? asInt(Object? v) => (v as num?)?.toInt();

double? asDouble(Object? v) => (v as num?)?.toDouble();

List<T> listOf<T>(Object? v, T Function(Json) f) =>
    [for (final e in (v as List? ?? const [])) f(e as Json)];

T? objOf<T>(Object? v, T? Function(Json) f) => v == null ? null : f(v as Json);

/// Parses a wire enum by name, falling back to [fallback] for unknown values.
T enumOf<T extends Enum>(List<T> values, Object? s, T fallback) =>
    values.firstWhere((v) => v.name == s, orElse: () => fallback);
