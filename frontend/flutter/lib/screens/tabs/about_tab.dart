import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../../providers/sync_provider.dart';
import '../../widgets/change_sync_folder_dialog.dart';

/// Static information screen: version, license, links, and daemon connection
/// stats pulled from [SyncProvider].
class AboutTab extends StatelessWidget {
  const AboutTab({super.key});

  static const String _version = '0.1.0';
  static const String _githubUrl = 'https://github.com/your-org/tuxdrive';
  static const String _license = 'MIT';

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 560),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // ---- App header ----
              _AppHeader(),
              const SizedBox(height: 32),

              // ---- Info cards ----
              _SectionCard(
                title: 'Version',
                children: [
                  _InfoRow(label: 'App version', value: _version),
                  _InfoRow(label: 'License', value: _license),
                ],
              ),
              const SizedBox(height: 16),

              // ---- Sync folder ----
              Consumer<SyncProvider>(
                builder: (context, sync, _) => _SectionCard(
                  title: 'Sync Folder',
                  children: [
                    _InfoRow(
                      label: 'Location',
                      value: sync.syncFolder ?? 'Loading...',
                      isCode: true,
                    ),
                    const SizedBox(height: 8),
                    OutlinedButton.icon(
                      onPressed: sync.isConnected
                          ? () => ChangeSyncFolderDialog.pickAndConfirm(
                              context, sync)
                          : null,
                      icon: const Icon(Icons.folder_open, size: 16),
                      label: const Text('Change location...'),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 16),

              // ---- Daemon status ----
              Consumer<SyncProvider>(
                builder: (context, sync, _) => _SectionCard(
                  title: 'Daemon Connection',
                  children: [
                    _InfoRow(
                      label: 'Status',
                      value: sync.isConnected ? 'Connected' : 'Disconnected',
                      valueColor:
                          sync.isConnected ? Colors.green : Colors.grey,
                    ),
                    if (sync.accountEmail.isNotEmpty)
                      _InfoRow(
                          label: 'Account', value: sync.accountEmail),
                    _InfoRow(
                      label: 'Sync state',
                      value: sync.syncStatus,
                    ),
                    if (sync.queuedCount > 0)
                      _InfoRow(
                        label: 'Queued',
                        value: '${sync.queuedCount} items',
                      ),
                    _InfoRow(
                      label: 'Socket path',
                      value: '~/.local/share/tuxdrive/daemon.sock',
                      isCode: true,
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 16),

              // ---- Links ----
              _SectionCard(
                title: 'Links',
                children: [
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: 6),
                    child: Row(
                      children: [
                        const Icon(Icons.code, size: 16),
                        const SizedBox(width: 8),
                        const Text('Source code:  '),
                        Expanded(
                          child: SelectableText(
                            _githubUrl,
                            style: TextStyle(
                              color:
                                  Theme.of(context).colorScheme.primary,
                              decoration: TextDecoration.underline,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 32),

              // ---- Copyright ----
              Center(
                child: Text(
                  'Copyright © 2024 TuxDrive contributors.\n'
                  'Released under the MIT License.',
                  textAlign: TextAlign.center,
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                        color:
                            Theme.of(context).colorScheme.onSurfaceVariant,
                      ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Internal widgets
// ---------------------------------------------------------------------------

class _AppHeader extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Row(
      children: [
        Container(
          width: 56,
          height: 56,
          decoration: BoxDecoration(
            color: colorScheme.primaryContainer,
            borderRadius: BorderRadius.circular(12),
          ),
          child: Icon(
            Icons.cloud_sync,
            size: 32,
            color: colorScheme.onPrimaryContainer,
          ),
        ),
        const SizedBox(width: 16),
        Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'TuxDrive',
              style: Theme.of(context).textTheme.headlineSmall?.copyWith(
                    fontWeight: FontWeight.w700,
                  ),
            ),
            Text(
              'Google Drive sync client for Linux',
              style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                    color: colorScheme.onSurfaceVariant,
                  ),
            ),
          ],
        ),
      ],
    );
  }
}

class _SectionCard extends StatelessWidget {
  final String title;
  final List<Widget> children;

  const _SectionCard({required this.title, required this.children});

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Card(
      elevation: 0,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(12),
        side: BorderSide(color: colorScheme.outlineVariant),
      ),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              title,
              style: Theme.of(context).textTheme.labelLarge?.copyWith(
                    color: colorScheme.primary,
                    fontWeight: FontWeight.w600,
                  ),
            ),
            const SizedBox(height: 8),
            ...children,
          ],
        ),
      ),
    );
  }
}

class _InfoRow extends StatelessWidget {
  final String label;
  final String value;
  final Color? valueColor;
  final bool isCode;

  const _InfoRow({
    required this.label,
    required this.value,
    this.valueColor,
    this.isCode = false,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 5),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 120,
            child: Text(
              label,
              style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                    color: Theme.of(context).colorScheme.onSurfaceVariant,
                  ),
            ),
          ),
          Expanded(
            child: SelectableText(
              value,
              style: isCode
                  ? TextStyle(
                      fontFamily: 'monospace',
                      fontSize: 13,
                      color: valueColor ??
                          Theme.of(context).colorScheme.onSurface,
                    )
                  : TextStyle(
                      fontWeight: FontWeight.w500,
                      color: valueColor ??
                          Theme.of(context).colorScheme.onSurface,
                    ),
            ),
          ),
        ],
      ),
    );
  }
}
