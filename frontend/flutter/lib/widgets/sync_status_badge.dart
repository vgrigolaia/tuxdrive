import 'package:flutter/material.dart';

/// Compact chip-style badge that represents the current sync status with an
/// appropriate icon and colour.
class SyncStatusBadge extends StatelessWidget {
  final String status;

  const SyncStatusBadge({super.key, required this.status});

  @override
  Widget build(BuildContext context) {
    final (icon, color, label) = _resolve(context);

    return Chip(
      avatar: icon,
      label: Text(label, style: TextStyle(color: color, fontSize: 12)),
      backgroundColor: color.withOpacity(0.12),
      side: BorderSide(color: color.withOpacity(0.4)),
      padding: const EdgeInsets.symmetric(horizontal: 4),
      visualDensity: VisualDensity.compact,
    );
  }

  (Widget icon, Color color, String label) _resolve(BuildContext context) {
    switch (status) {
      case 'synced':
        return (
          const Icon(Icons.check_circle, color: Colors.green, size: 16),
          Colors.green,
          'Synced',
        );
      case 'syncing':
        return (
          const SizedBox(
            width: 14,
            height: 14,
            child: CircularProgressIndicator(
              strokeWidth: 2,
              color: Colors.blue,
            ),
          ),
          Colors.blue,
          'Syncing',
        );
      case 'paused':
        return (
          const Icon(Icons.pause_circle, color: Colors.orange, size: 16),
          Colors.orange,
          'Paused',
        );
      case 'error':
        return (
          const Icon(Icons.error, color: Colors.red, size: 16),
          Colors.red,
          'Error',
        );
      case 'conflict':
        return (
          const Icon(Icons.warning, color: Colors.orange, size: 16),
          Colors.orange,
          'Conflict',
        );
      case 'disconnected':
      default:
        return (
          const Icon(Icons.cloud_off, color: Colors.grey, size: 16),
          Colors.grey,
          status == 'disconnected' ? 'Disconnected' : status,
        );
    }
  }
}
