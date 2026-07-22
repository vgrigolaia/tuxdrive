import 'package:flutter/material.dart';
import '../providers/sync_provider.dart';

/// Shown when the daemon detects that files recorded as already synced are
/// missing locally (folder wiped/moved/reinstalled). Re-downloading is the
/// safe default; declining requires typing DELETE, mirroring the CLI's
/// typed-confirmation safeguard so a stray click can't cause data loss.
class SyncConflictDialog extends StatefulWidget {
  final SyncProvider sync;

  const SyncConflictDialog({super.key, required this.sync});

  static Future<void> show(BuildContext context, SyncProvider sync) {
    return showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (_) => SyncConflictDialog(sync: sync),
    );
  }

  @override
  State<SyncConflictDialog> createState() => _SyncConflictDialogState();
}

class _SyncConflictDialogState extends State<SyncConflictDialog> {
  bool _showDeleteConfirm = false;
  final _deleteController = TextEditingController();
  bool _deleteMatches = false;

  @override
  void dispose() {
    _deleteController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final known = widget.sync.loginKnownCount ?? 0;
    final missing = widget.sync.loginMissingCount ?? 0;
    final paths = widget.sync.loginMissingPaths ?? const [];

    return AlertDialog(
      title: const Text('Sync history mismatch detected'),
      content: SizedBox(
        width: 440,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              '$known file(s) are recorded as already synced, but $missing of '
              'them are missing locally.',
            ),
            const SizedBox(height: 8),
            const Text(
              'This usually means the local folder was cleared, moved, or '
              'reinstalled — not that you deleted these files on purpose. '
              'If you continue without downloading them, they will be '
              'removed from Google Drive too.',
              style: TextStyle(fontSize: 13),
            ),
            const SizedBox(height: 12),
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 160),
              child: Container(
                decoration: BoxDecoration(
                  border: Border.all(
                    color: Theme.of(context).colorScheme.outlineVariant,
                  ),
                  borderRadius: BorderRadius.circular(8),
                ),
                padding: const EdgeInsets.all(8),
                child: ListView(
                  shrinkWrap: true,
                  children: paths
                      .map((p) => Text(p, style: const TextStyle(fontFamily: 'monospace', fontSize: 12)))
                      .toList(),
                ),
              ),
            ),
            if (_showDeleteConfirm) ...[
              const SizedBox(height: 16),
              Text(
                'Type DELETE to confirm you want tuxdrive to remove these '
                '$missing file(s) from Google Drive:',
                style: const TextStyle(fontSize: 13),
              ),
              const SizedBox(height: 8),
              TextField(
                controller: _deleteController,
                decoration: const InputDecoration(
                  hintText: 'DELETE',
                  border: OutlineInputBorder(),
                  isDense: true,
                ),
                onChanged: (v) => setState(() => _deleteMatches = v == 'DELETE'),
              ),
            ],
          ],
        ),
      ),
      actions: _showDeleteConfirm
          ? [
              TextButton(
                onPressed: () => setState(() {
                  _showDeleteConfirm = false;
                  _deleteController.clear();
                  _deleteMatches = false;
                }),
                child: const Text('Back'),
              ),
              FilledButton(
                onPressed: _deleteMatches
                    ? () {
                        Navigator.pop(context);
                        widget.sync.resolveSyncConflict('delete_confirmed');
                      }
                    : null,
                style: FilledButton.styleFrom(backgroundColor: Colors.red),
                child: const Text('Skip & allow deletion on Drive'),
              ),
            ]
          : [
              TextButton(
                onPressed: () => setState(() => _showDeleteConfirm = true),
                child: const Text("Don't download"),
              ),
              FilledButton(
                onPressed: () {
                  Navigator.pop(context);
                  widget.sync.resolveSyncConflict('download');
                },
                child: Text('Re-download $missing file(s)'),
              ),
            ],
    );
  }
}
