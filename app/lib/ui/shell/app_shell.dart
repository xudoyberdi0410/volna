/// The window frame: sidebar, content, mini player over the animated
/// background, plus global keys (space, arrows), error snackbars and the
/// "no daemon" banner.
library;

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../../ipc/transport.dart';
import '../../state/connection.dart';
import '../../state/player.dart';
import '../player/mini_player.dart';
import 'background.dart';
import 'sidebar.dart';

class AppShell extends ConsumerStatefulWidget {
  final Widget child;
  const AppShell({super.key, required this.child});

  @override
  ConsumerState<AppShell> createState() => _AppShellState();
}

class _AppShellState extends ConsumerState<AppShell> {
  StreamSubscription<String>? _messages;

  @override
  void initState() {
    super.initState();
    _messages = ref.read(playerMessagesProvider).stream.listen(_showMessage);
  }

  @override
  void dispose() {
    _messages?.cancel();
    super.dispose();
  }

  void _showMessage(String text) {
    final m = ScaffoldMessenger.maybeOf(context);
    if (m == null) return;
    m
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(text), duration: const Duration(seconds: 4)));
  }

  @override
  Widget build(BuildContext context) {
    final location = GoRouterState.of(context).uri.path;
    final width = MediaQuery.sizeOf(context).width;
    final status = ref.watch(connectionStatusProvider).value ?? TransportStatus.connecting;
    return PlayerKeys(
      child: Scaffold(
        backgroundColor: Colors.transparent,
        body: AppBackground(
          child: Row(
            children: [
              Sidebar(
                location: location,
                // Settings bring their own section list: a rail avoids two
                // sidebars side by side.
                compact: width < 820 || location.startsWith('/settings'),
                onSelect: (p) => context.go(p),
              ),
              Expanded(
                child: Column(
                  children: [
                    _ConnectionBanner(status: status),
                    Expanded(
                      child: ClipRect(child: widget.child),
                    ),
                    const MiniPlayer(),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _ConnectionBanner extends StatelessWidget {
  final TransportStatus status;
  const _ConnectionBanner({required this.status});

  @override
  Widget build(BuildContext context) {
    final t = Theme.of(context);
    final show = status == TransportStatus.disconnected || status == TransportStatus.closed;
    return AnimatedSize(
      duration: const Duration(milliseconds: 250),
      curve: Curves.easeOutCubic,
      child: !show
          ? const SizedBox(width: double.infinity)
          : Container(
              width: double.infinity,
              margin: const EdgeInsets.fromLTRB(12, 12, 12, 0),
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
              decoration: BoxDecoration(
                color: t.colorScheme.error.withValues(alpha: 0.14),
                borderRadius: BorderRadius.circular(12),
              ),
              child: Row(
                children: [
                  Icon(Icons.cloud_off_rounded, size: 18, color: t.colorScheme.error),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      status == TransportStatus.closed
                          ? 'Соединение с плеером закрыто'
                          : 'Нет связи с плеером Volna. Переподключаюсь…',
                      style: t.textTheme.bodyMedium,
                    ),
                  ),
                ],
              ),
            ),
    );
  }
}

/// Space: play/pause; ←/→: seek 5 s (Shift: 15 s); ↑/↓: volume. Ignored
/// while a text field has focus. Media keys go through MPRIS in the daemon.
class PlayerKeys extends ConsumerStatefulWidget {
  final Widget child;
  const PlayerKeys({super.key, required this.child});

  @override
  ConsumerState<PlayerKeys> createState() => _PlayerKeysState();
}

class _PlayerKeysState extends ConsumerState<PlayerKeys> {
  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    super.dispose();
  }

  /// A text field, menu or dialog has focus: keys are theirs.
  static bool _focusBusy() {
    final ctx = FocusManager.instance.primaryFocus?.context;
    if (ctx == null || !ctx.mounted) return false;
    if (ctx.widget is EditableText || ctx.findAncestorWidgetOfExactType<EditableText>() != null) {
      return true;
    }
    return ModalRoute.of(ctx) is PopupRoute;
  }

  bool _onKey(KeyEvent e) {
    if (e is KeyUpEvent || !mounted) return false;
    if (_focusBusy()) return false;
    final kb = HardwareKeyboard.instance;
    if (kb.isControlPressed || kb.isAltPressed || kb.isMetaPressed) return false;
    final ctl = ref.read(playerViewProvider.notifier);
    final step = kb.isShiftPressed ? 15.0 : 5.0;
    final k = e.logicalKey;
    if (k == LogicalKeyboardKey.space) {
      if (e is KeyRepeatEvent) return true;
      ctl.toggle();
      return true;
    }
    if (k == LogicalKeyboardKey.arrowRight) {
      ctl.seekBy(step);
      return true;
    }
    if (k == LogicalKeyboardKey.arrowLeft) {
      ctl.seekBy(-step);
      return true;
    }
    if (k == LogicalKeyboardKey.arrowUp || k == LogicalKeyboardKey.arrowDown) {
      final v = ref.read(playerViewProvider).state.volume;
      ctl.setVolume(v + (k == LogicalKeyboardKey.arrowUp ? 0.05 : -0.05));
      return true;
    }
    return false;
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
