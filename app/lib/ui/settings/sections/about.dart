import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../../ipc/transport.dart';
import '../../../state/connection.dart';
import '../widgets.dart';

/// The app version shown in «О программе» (keep in step with pubspec.yaml).
const appVersion = '0.1.0';

/// Settings → «О программе».
class AboutSection extends ConsumerWidget {
  const AboutSection({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final status = ref.watch(connectionStatusProvider).value ?? TransportStatus.connecting;
    final base = ref.watch(transportProvider).streamBase;
    final c = Theme.of(context).colorScheme;
    final t = Theme.of(context).textTheme;
    final (tone, text) = switch (status) {
      TransportStatus.connected => (Tone.good, 'Подключено'),
      TransportStatus.connecting => (Tone.busy, 'Подключаюсь'),
      TransportStatus.disconnected => (Tone.bad, 'Нет связи'),
      TransportStatus.closed => (Tone.neutral, 'Отключено'),
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SectionHeader('О программе'),
        SettingsCard(
          padding: const EdgeInsets.all(24),
          title: 'Volna',
          subtitle: 'Версия $appVersion',
          trailing: StatusPill(tone, text),
          children: [
            Row(
              children: [
                Container(
                  width: 56,
                  height: 56,
                  decoration: BoxDecoration(
                    gradient: LinearGradient(colors: [c.primary, c.tertiary]),
                    borderRadius: BorderRadius.circular(16),
                  ),
                  child: Icon(Icons.waves, color: c.onPrimary, size: 32),
                ),
                const SizedBox(width: 16),
                Expanded(
                  child: Text(
                    'Неофициальный плеер YouTube Music с «Моей волной». '
                    'Без своего сервера: всё работает на вашем компьютере.',
                    style: t.bodyMedium,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 16),
            KeyValueTable([
              ('Демон', text, tone == Tone.busy ? null : tone),
              ('Аудиосервер', base ?? '—', null),
              ('Интерфейс', 'Flutter', null),
            ]),
            const SizedBox(height: 12),
            Text(
              'Не связан с YouTube, Google или Яндексом. Прежний интерфейс основан на Monochrome '
              '(Apache-2.0).',
              style: t.bodySmall?.copyWith(color: c.onSurfaceVariant),
            ),
            ButtonRow([
              OutlinedButton(
                onPressed: () => showLicensePage(
                  context: context,
                  applicationName: 'Volna',
                  applicationVersion: appVersion,
                ),
                child: const Text('Лицензии'),
              ),
            ]),
          ],
        ),
      ],
    );
  }
}
