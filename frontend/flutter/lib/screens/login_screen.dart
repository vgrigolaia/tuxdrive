import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:url_launcher/url_launcher.dart';
import '../providers/sync_provider.dart';
import '../widgets/change_sync_folder_dialog.dart';
import '../widgets/sync_conflict_dialog.dart';

/// Shown when the daemon is not reachable or no account is logged in.
/// Drives the whole OAuth login (and any sync-conflict resolution) through
/// the GUI — no terminal required.
class LoginScreen extends StatefulWidget {
  const LoginScreen({super.key});

  @override
  State<LoginScreen> createState() => _LoginScreenState();
}

class _LoginScreenState extends State<LoginScreen> {
  bool _conflictDialogShown = false;

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Scaffold(
      backgroundColor: colorScheme.surface,
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 480),
          child: Card(
            elevation: 0,
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(16),
              side: BorderSide(color: colorScheme.outlineVariant),
            ),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 40, vertical: 48),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Container(
                    width: 80,
                    height: 80,
                    decoration: BoxDecoration(
                      color: colorScheme.primaryContainer,
                      shape: BoxShape.circle,
                    ),
                    child: Icon(
                      Icons.cloud_sync,
                      size: 44,
                      color: colorScheme.onPrimaryContainer,
                    ),
                  ),
                  const SizedBox(height: 24),
                  Text(
                    'Connect Google Drive',
                    style: Theme.of(context).textTheme.headlineSmall?.copyWith(
                          fontWeight: FontWeight.w600,
                        ),
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 8),
                  Text(
                    'TuxDrive syncs your Google Drive files locally.',
                    style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                          color: colorScheme.onSurfaceVariant,
                        ),
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 36),
                  _Body(onConflictPending: _maybeShowConflictDialog),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  void _maybeShowConflictDialog(SyncProvider sync) {
    if (_conflictDialogShown) return;
    _conflictDialogShown = true;
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      if (!mounted) return;
      await SyncConflictDialog.show(context, sync);
      _conflictDialogShown = false;
    });
  }
}

class _Body extends StatelessWidget {
  final void Function(SyncProvider sync) onConflictPending;

  const _Body({required this.onConflictPending});

  @override
  Widget build(BuildContext context) {
    return Consumer<SyncProvider>(
      builder: (context, sync, _) {
        if (!sync.isConnected) {
          return _NotRunning(sync: sync);
        }

        switch (sync.loginPhase) {
          case 'awaiting_browser':
            return _AwaitingBrowser(sync: sync);
          case 'exchanging_code':
            return _Spinner(label: 'Finishing sign-in...', sync: sync);
          case 'conflict_pending':
            onConflictPending(sync);
            return _Spinner(label: 'Checking sync history...', sync: sync);
          case 'resolving_conflict':
            return _ResolvingConflict(sync: sync);
          case 'failed':
            return _Failed(sync: sync);
          case 'idle':
          default:
            return _Idle(sync: sync);
        }
      },
    );
  }
}

class _NotRunning extends StatelessWidget {
  final SyncProvider sync;
  const _NotRunning({required this.sync});

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(
              sync.isConnecting ? Icons.hourglass_top : Icons.cloud_off,
              size: 16,
              color: sync.isConnecting ? Colors.blue : Colors.grey,
            ),
            const SizedBox(width: 6),
            Text(
              sync.isConnecting ? 'Connecting...' : 'Daemon not running',
              style: TextStyle(
                color: sync.isConnecting ? Colors.blue : Colors.grey,
                fontSize: 13,
              ),
            ),
          ],
        ),
        const SizedBox(height: 16),
        FilledButton.icon(
          onPressed: sync.isConnecting ? null : () => sync.connect(),
          icon: const Icon(Icons.refresh),
          label: const Text('Reconnect'),
          style: FilledButton.styleFrom(minimumSize: const Size(160, 44)),
        ),
      ],
    );
  }
}

class _Idle extends StatelessWidget {
  final SyncProvider sync;
  const _Idle({required this.sync});

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        _FolderPickerRow(sync: sync),
        const SizedBox(height: 24),
        FilledButton.icon(
          onPressed: () => sync.startLogin(),
          icon: const Icon(Icons.login),
          label: const Text('Connect Google Drive'),
          style: FilledButton.styleFrom(minimumSize: const Size(220, 44)),
        ),
      ],
    );
  }
}

/// Lets the user pick where files will be synced before they even log in,
/// reusing the same picker/move flow exposed later in the About tab.
class _FolderPickerRow extends StatelessWidget {
  final SyncProvider sync;
  const _FolderPickerRow({required this.sync});

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      decoration: BoxDecoration(
        color: colorScheme.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(8),
      ),
      child: Row(
        children: [
          Icon(Icons.folder_outlined, size: 18, color: colorScheme.onSurfaceVariant),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              sync.syncFolder ?? 'Loading...',
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
            ),
          ),
          TextButton(
            onPressed: () => ChangeSyncFolderDialog.pickAndConfirm(context, sync),
            child: const Text('Change'),
          ),
        ],
      ),
    );
  }
}

class _AwaitingBrowser extends StatelessWidget {
  final SyncProvider sync;
  const _AwaitingBrowser({required this.sync});

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final url = sync.loginAuthUrl ?? '';

    return Column(
      children: [
        Container(
          width: double.infinity,
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
          decoration: BoxDecoration(
            color: colorScheme.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(6),
          ),
          child: SelectableText(
            url,
            style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
          ),
        ),
        const SizedBox(height: 16),
        FilledButton.icon(
          onPressed: () => launchUrl(Uri.parse(url)),
          icon: const Icon(Icons.open_in_browser),
          label: const Text('Open in Browser'),
          style: FilledButton.styleFrom(minimumSize: const Size(220, 44)),
        ),
        const SizedBox(height: 16),
        const SizedBox(
          width: 18,
          height: 18,
          child: CircularProgressIndicator(strokeWidth: 2),
        ),
        const SizedBox(height: 8),
        Text(
          'Waiting for you to finish in the browser...',
          style: Theme.of(context).textTheme.bodySmall,
        ),
        const SizedBox(height: 12),
        TextButton(
          onPressed: () => sync.cancelLogin(),
          child: const Text('Cancel'),
        ),
      ],
    );
  }
}

class _Spinner extends StatelessWidget {
  final String label;
  final SyncProvider? sync;
  const _Spinner({required this.label, this.sync});

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        const SizedBox(
          width: 18,
          height: 18,
          child: CircularProgressIndicator(strokeWidth: 2),
        ),
        const SizedBox(height: 12),
        Text(label, style: Theme.of(context).textTheme.bodySmall),
        if (sync != null) ...[
          const SizedBox(height: 12),
          TextButton(
            onPressed: () => sync!.cancelLogin(),
            child: const Text('Cancel'),
          ),
        ],
      ],
    );
  }
}

class _ResolvingConflict extends StatelessWidget {
  final SyncProvider sync;
  const _ResolvingConflict({required this.sync});

  @override
  Widget build(BuildContext context) {
    final done = sync.loginResolvedDone ?? 0;
    final total = sync.loginResolvedTotal ?? 1;
    return Column(
      children: [
        LinearProgressIndicator(value: total > 0 ? done / total : null),
        const SizedBox(height: 12),
        Text('Downloading $done of $total file(s)...'),
      ],
    );
  }
}

class _Failed extends StatelessWidget {
  final SyncProvider sync;
  const _Failed({required this.sync});

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        const Icon(Icons.error_outline, color: Colors.red, size: 32),
        const SizedBox(height: 8),
        Text(
          sync.loginError ?? 'Login failed',
          textAlign: TextAlign.center,
          style: const TextStyle(fontSize: 13),
        ),
        const SizedBox(height: 16),
        FilledButton(
          onPressed: () => sync.startLogin(),
          child: const Text('Try again'),
        ),
      ],
    );
  }
}
