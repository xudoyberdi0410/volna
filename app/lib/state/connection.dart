import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ipc/transport.dart';

/// The daemon transport; overridden in `main` with the platform one.
final transportProvider = Provider<Transport>(
  (ref) => throw UnimplementedError('transportProvider must be overridden'),
);

final connectionStatusProvider = StreamProvider<TransportStatus>((ref) async* {
  final t = ref.watch(transportProvider);
  yield t.status;
  yield* t.statusChanges;
});
