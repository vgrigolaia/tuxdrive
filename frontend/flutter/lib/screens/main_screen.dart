import 'dart:io';

import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../providers/sync_provider.dart';
import '../widgets/sync_status_badge.dart';
import 'tabs/files_tab.dart';
import 'tabs/logs_tab.dart';
import 'tabs/about_tab.dart';

/// Primary application window. Contains the AppBar, bottom NavigationBar,
/// and the three content tabs (Files, Activity/Logs, About).
class MainScreen extends StatefulWidget {
  const MainScreen({super.key});

  @override
  State<MainScreen> createState() => _MainScreenState();
}

class _MainScreenState extends State<MainScreen> {
  int _selectedIndex = 0;

  static const List<_TabDef> _tabs = [
    _TabDef(label: 'Files', icon: Icons.folder_outlined, activeIcon: Icons.folder),
    _TabDef(label: 'Activity', icon: Icons.terminal_outlined, activeIcon: Icons.terminal),
    _TabDef(label: 'About', icon: Icons.info_outlined, activeIcon: Icons.info),
  ];

  static const List<Widget> _screens = [
    FilesTab(),
    LogsTab(),
    AboutTab(),
  ];

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: _buildAppBar(context),
      body: IndexedStack(
        index: _selectedIndex,
        children: _screens,
      ),
      bottomNavigationBar: NavigationBar(
        selectedIndex: _selectedIndex,
        onDestinationSelected: (i) => setState(() => _selectedIndex = i),
        destinations: _tabs
            .map(
              (t) => NavigationDestination(
                icon: Icon(t.icon),
                selectedIcon: Icon(t.activeIcon),
                label: t.label,
              ),
            )
            .toList(),
      ),
    );
  }

  PreferredSizeWidget _buildAppBar(BuildContext context) {
    return AppBar(
      title: const Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.cloud_sync, size: 22),
          SizedBox(width: 8),
          Text('TuxDrive'),
        ],
      ),
      actions: [
        // Sync status badge
        Consumer<SyncProvider>(
          builder: (_, sync, __) => Padding(
            padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 10),
            child: SyncStatusBadge(
              status: sync.isPaused ? 'paused' : sync.syncStatus,
            ),
          ),
        ),

        // Queued count badge
        Consumer<SyncProvider>(
          builder: (_, sync, __) {
            if (sync.queuedCount == 0) return const SizedBox.shrink();
            final eta = _formatEta(sync.etaSeconds);
            final label = eta == null
                ? '${sync.queuedCount} queued'
                : '${sync.queuedCount} queued · ~$eta left';
            return Padding(
              padding: const EdgeInsets.only(right: 4),
              child: Chip(
                label: Text(label),
                visualDensity: VisualDensity.compact,
                padding: const EdgeInsets.symmetric(horizontal: 4),
              ),
            );
          },
        ),

        // Account email chip
        Consumer<SyncProvider>(
          builder: (_, sync, __) {
            if (sync.accountEmail.isEmpty) return const SizedBox.shrink();
            return Padding(
              padding: const EdgeInsets.symmetric(vertical: 10, horizontal: 4),
              child: InputChip(
                avatar: const CircleAvatar(
                  radius: 10,
                  child: Icon(Icons.person, size: 14),
                ),
                label: Text(
                  sync.accountEmail,
                  style: const TextStyle(fontSize: 12),
                ),
                onDeleted: () => _confirmLogout(context, sync),
                deleteIcon: const Icon(Icons.logout, size: 14),
                deleteButtonTooltipMessage: 'Log out',
              ),
            );
          },
        ),

        // Open sync folder in the system file manager
        Consumer<SyncProvider>(
          builder: (_, sync, __) => IconButton(
            onPressed: sync.syncFolder == null
                ? null
                : () => _openSyncFolder(context, sync.syncFolder!),
            icon: const Icon(Icons.folder_open),
            tooltip: 'Open TuxDrive Folder',
          ),
        ),

        // Pause / Resume
        Consumer<SyncProvider>(
          builder: (_, sync, __) => IconButton(
            onPressed: sync.isConnected
                ? () => sync.isPaused ? sync.resume() : sync.pause()
                : null,
            icon: Icon(sync.isPaused ? Icons.play_arrow : Icons.pause),
            tooltip: sync.isPaused ? 'Resume sync' : 'Pause sync',
          ),
        ),

        // Settings / overflow menu
        Consumer<SyncProvider>(
          builder: (_, sync, __) => PopupMenuButton<_MenuAction>(
            icon: const Icon(Icons.more_vert),
            tooltip: 'More options',
            onSelected: (action) => _handleMenuAction(context, sync, action),
            itemBuilder: (_) => [
              const PopupMenuItem(
                value: _MenuAction.reconnect,
                child: ListTile(
                  leading: Icon(Icons.refresh),
                  title: Text('Reconnect to daemon'),
                  contentPadding: EdgeInsets.zero,
                ),
              ),
              const PopupMenuItem(
                value: _MenuAction.logout,
                child: ListTile(
                  leading: Icon(Icons.logout),
                  title: Text('Log out'),
                  contentPadding: EdgeInsets.zero,
                ),
              ),
              const PopupMenuDivider(),
              const PopupMenuItem(
                value: _MenuAction.shutdown,
                child: ListTile(
                  leading: Icon(Icons.power_settings_new),
                  title: Text('Stop daemon'),
                  contentPadding: EdgeInsets.zero,
                ),
              ),
            ],
          ),
        ),

        const SizedBox(width: 4),
      ],
    );
  }

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------

  Future<void> _openSyncFolder(BuildContext context, String path) async {
    try {
      final result = await Process.run('xdg-open', [path]);
      if (result.exitCode != 0 && context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Could not open $path')),
        );
      }
    } catch (e) {
      if (context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Could not open folder: $e')),
        );
      }
    }
  }

  Future<void> _confirmLogout(BuildContext context, SyncProvider sync) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (_) => AlertDialog(
        title: const Text('Log out?'),
        content: Text(
          'This will disconnect the account "${sync.accountEmail}" '
          'from TuxDrive. Syncing will stop until you log in again.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Log out'),
          ),
        ],
      ),
    );
    if (ok == true && context.mounted) {
      await sync.logout();
    }
  }

  Future<void> _handleMenuAction(
      BuildContext context, SyncProvider sync, _MenuAction action) async {
    switch (action) {
      case _MenuAction.reconnect:
        await sync.connect();
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text(sync.isConnected
                  ? 'Connected to daemon'
                  : 'Could not connect to daemon'),
              duration: const Duration(seconds: 2),
            ),
          );
        }
      case _MenuAction.logout:
        if (context.mounted) await _confirmLogout(context, sync);
      case _MenuAction.shutdown:
        final ok = await showDialog<bool>(
          context: context,
          builder: (_) => AlertDialog(
            title: const Text('Stop daemon?'),
            content: const Text(
              'This will stop the tuxdrive-daemon process. '
              'Sync will not run until you restart it.',
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(context, false),
                child: const Text('Cancel'),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(context, true),
                style: FilledButton.styleFrom(
                    backgroundColor: Colors.red),
                child: const Text('Stop'),
              ),
            ],
          ),
        );
        if (ok == true && context.mounted) {
          await sync.shutdownDaemon();
        }
    }
  }
}

/// Format an ETA in seconds as a short human string ("45s", "3m", "2h 10m").
/// Returns `null` when there isn't a meaningful estimate yet.
String? _formatEta(int? seconds) {
  if (seconds == null) return null;
  if (seconds < 60) return '${seconds}s';
  final minutes = seconds ~/ 60;
  if (minutes < 60) return '${minutes}m';
  final hours = minutes ~/ 60;
  return '${hours}h ${minutes % 60}m';
}

// ---------------------------------------------------------------------------
// Data classes
// ---------------------------------------------------------------------------

class _TabDef {
  final String label;
  final IconData icon;
  final IconData activeIcon;

  const _TabDef({
    required this.label,
    required this.icon,
    required this.activeIcon,
  });
}

enum _MenuAction { reconnect, logout, shutdown }
