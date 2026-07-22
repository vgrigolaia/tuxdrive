import 'package:file_selector/file_selector.dart';
import 'package:flutter/material.dart';
import '../providers/sync_provider.dart';

/// Lets the user relocate the local sync folder: opens a native directory
/// picker, then confirms before doing anything destructive-adjacent (moving
/// the user's already-synced files and restarting the daemon).
class ChangeSyncFolderDialog {
  /// Opens the folder picker, then (if a folder was chosen) the confirmation
  /// dialog. Call this from a button's `onPressed`.
  static Future<void> pickAndConfirm(
    BuildContext context,
    SyncProvider sync,
  ) async {
    final chosen = await getDirectoryPath(
      confirmButtonText: 'Select folder',
    );
    if (chosen == null) return;
    if (!context.mounted) return;

    if (chosen == sync.syncFolder) return;

    await showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (_) => _ConfirmDialog(sync: sync, newPath: chosen),
    );
  }
}

class _ConfirmDialog extends StatefulWidget {
  final SyncProvider sync;
  final String newPath;

  const _ConfirmDialog({required this.sync, required this.newPath});

  @override
  State<_ConfirmDialog> createState() => _ConfirmDialogState();
}

class _ConfirmDialogState extends State<_ConfirmDialog> {
  bool _applying = false;
  String? _error;

  Future<void> _confirm() async {
    setState(() {
      _applying = true;
      _error = null;
    });
    final error = await widget.sync.changeSyncFolder(widget.newPath);
    if (!mounted) return;
    if (error == null) {
      Navigator.pop(context);
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text('Sync folder changed — restarting daemon...'),
          duration: Duration(seconds: 4),
        ),
      );
    } else {
      setState(() {
        _applying = false;
        _error = error;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final oldPath = widget.sync.syncFolder ?? '(unknown)';

    return AlertDialog(
      title: const Text('Change sync folder?'),
      content: SizedBox(
        width: 440,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text('Your synced files will be moved:'),
            const SizedBox(height: 12),
            _PathRow(label: 'From', path: oldPath),
            const SizedBox(height: 4),
            _PathRow(label: 'To', path: widget.newPath),
            const SizedBox(height: 12),
            Text(
              'The destination must be empty. Sync will pause while the '
              'files are moved, and the daemon will restart automatically '
              'to apply the change.',
              style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: Theme.of(context).colorScheme.onSurfaceVariant,
                  ),
            ),
            if (_error != null) ...[
              const SizedBox(height: 12),
              Text(
                _error!,
                style: const TextStyle(color: Colors.red, fontSize: 13),
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: _applying ? null : () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: _applying ? null : _confirm,
          child: _applying
              ? const SizedBox(
                  width: 16,
                  height: 16,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : const Text('Move & switch'),
        ),
      ],
    );
  }
}

class _PathRow extends StatelessWidget {
  final String label;
  final String path;

  const _PathRow({required this.label, required this.path});

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          width: 44,
          child: Text(
            label,
            style: Theme.of(context).textTheme.bodySmall?.copyWith(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
          ),
        ),
        Expanded(
          child: Text(
            path,
            style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
          ),
        ),
      ],
    );
  }
}
