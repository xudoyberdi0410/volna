/// Development entry for the browse screens without the app shell:
/// `flutter run -d linux -t lib/main_browse_dev.dart` (uses `VOLNA_SOCKET`).
library;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import 'ipc/socket_transport.dart';
import 'state/connection.dart';
import 'ui/browse/routes.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  final transport = SocketTransport()..start();
  final router = GoRouter(
    initialLocation: '/home',
    routes: [
      ShellRoute(builder: (context, state, child) => _DevShell(child: child), routes: browseRoutes),
    ],
  );
  runApp(ProviderScope(
    overrides: [transportProvider.overrideWithValue(transport)],
    child: MaterialApp.router(
      title: 'Volna (browse dev)',
      debugShowCheckedModeBanner: false,
      themeMode: ThemeMode.dark,
      darkTheme: ThemeData(colorSchemeSeed: const Color(0xFF6E5BFF), brightness: Brightness.dark),
      theme: ThemeData(colorSchemeSeed: const Color(0xFF6E5BFF)),
      routerConfig: router,
    ),
  ));
}

class _DevShell extends StatelessWidget {
  final Widget child;
  const _DevShell({required this.child});

  static const _tabs = ['/home', '/search', '/library'];

  @override
  Widget build(BuildContext context) {
    final path = GoRouterState.of(context).uri.path;
    final i = _tabs.indexWhere(path.startsWith);
    final router = GoRouter.of(context);
    return Scaffold(
      body: Row(children: [
        NavigationRail(
          selectedIndex: i < 0 ? null : i,
          labelType: NavigationRailLabelType.all,
          leading: IconButton(
            tooltip: 'Назад',
            onPressed: router.canPop() ? router.pop : null,
            icon: const Icon(Icons.arrow_back_rounded),
          ),
          onDestinationSelected: (j) => context.go(_tabs[j]),
          destinations: const [
            NavigationRailDestination(icon: Icon(Icons.home_rounded), label: Text('Главная')),
            NavigationRailDestination(icon: Icon(Icons.search_rounded), label: Text('Поиск')),
            NavigationRailDestination(icon: Icon(Icons.library_music_rounded), label: Text('Библиотека')),
          ],
        ),
        Expanded(child: child),
      ]),
    );
  }
}
