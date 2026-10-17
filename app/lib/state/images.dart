/// The app's one image cache for `volna://img/` covers (memory + disk LRU).
/// Every screen should use this, so a cover is fetched and cached once.
library;

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../ipc/proto_image.dart';
import 'connection.dart';

final protoImageCacheProvider =
    Provider<ProtoImageCache>((ref) => ProtoImageCache(ref.watch(transportProvider)));
