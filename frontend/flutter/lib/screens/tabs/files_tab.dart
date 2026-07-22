import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../../providers/sync_provider.dart';
import '../../ipc/daemon_client.dart';

/// Displays the list of synced files / folders from the daemon.
class FilesTab extends StatefulWidget {
  const FilesTab({super.key});

  @override
  State<FilesTab> createState() => _FilesTabState();
}

class _FilesTabState extends State<FilesTab> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<SyncProvider>().refreshFiles();
    });
  }

  @override
  Widget build(BuildContext context) {
    return Consumer<SyncProvider>(
      builder: (context, sync, _) {
        final files = sync.files;

        return RefreshIndicator(
          onRefresh: () => sync.refreshFiles(),
          child: files.isEmpty
              ? _EmptyFilesPlaceholder(onRefresh: () => sync.refreshFiles())
              : ListView.separated(
                  padding: const EdgeInsets.symmetric(vertical: 8),
                  itemCount: files.length,
                  separatorBuilder: (_, __) => const Divider(height: 1),
                  itemBuilder: (context, index) =>
                      _FileEntryTile(entry: files[index]),
                ),
        );
      },
    );
  }
}

// ---------------------------------------------------------------------------
// Tile
// ---------------------------------------------------------------------------

class _FileEntryTile extends StatelessWidget {
  final FileEntry entry;

  const _FileEntryTile({required this.entry});

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: Icon(
        entry.isFolder ? Icons.folder : _fileIcon(entry.name),
        color: entry.isFolder
            ? Theme.of(context).colorScheme.primary
            : Theme.of(context).colorScheme.onSurfaceVariant,
        size: 28,
      ),
      title: Text(
        entry.name,
        style: const TextStyle(fontWeight: FontWeight.w500),
        overflow: TextOverflow.ellipsis,
      ),
      subtitle: entry.isFolder
          ? null
          : Text(
              _formatSize(entry.size),
              style: TextStyle(
                fontSize: 12,
                color: Theme.of(context).colorScheme.onSurfaceVariant,
              ),
            ),
      trailing: _SyncStatusChip(status: entry.syncStatus),
    );
  }

  IconData _fileIcon(String name) {
    final ext = name.contains('.') ? name.split('.').last.toLowerCase() : '';
    switch (ext) {
      case 'pdf':
        return Icons.picture_as_pdf;
      case 'jpg':
      case 'jpeg':
      case 'png':
      case 'gif':
      case 'webp':
      case 'bmp':
        return Icons.image;
      case 'mp4':
      case 'mkv':
      case 'avi':
      case 'mov':
        return Icons.video_file;
      case 'mp3':
      case 'flac':
      case 'wav':
      case 'ogg':
        return Icons.audio_file;
      case 'zip':
      case 'tar':
      case 'gz':
      case '7z':
      case 'rar':
        return Icons.folder_zip;
      case 'doc':
      case 'docx':
        return Icons.description;
      case 'xls':
      case 'xlsx':
        return Icons.table_chart;
      case 'ppt':
      case 'pptx':
        return Icons.slideshow;
      default:
        return Icons.insert_drive_file;
    }
  }

  String _formatSize(int bytes) {
    if (bytes < 1024) return '$bytes B';
    if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
    if (bytes < 1024 * 1024 * 1024) {
      return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
    }
    return '${(bytes / (1024 * 1024 * 1024)).toStringAsFixed(2)} GB';
  }
}

// ---------------------------------------------------------------------------
// Sync status chip (file-level)
// ---------------------------------------------------------------------------

class _SyncStatusChip extends StatelessWidget {
  final String status;

  const _SyncStatusChip({required this.status});

  @override
  Widget build(BuildContext context) {
    final (icon, color) = _resolve();
    return Chip(
      avatar: icon,
      label: Text(
        _label(),
        style: TextStyle(fontSize: 11, color: color),
      ),
      backgroundColor: color.withOpacity(0.1),
      side: BorderSide(color: color.withOpacity(0.3)),
      padding: EdgeInsets.zero,
      materialTapTargetSize: MaterialTapTargetSize.shrinkWrap,
      visualDensity: VisualDensity.compact,
    );
  }

  (Widget, Color) _resolve() {
    switch (status) {
      case 'synced':
        return (
          const Icon(Icons.check_circle, size: 14, color: Colors.green),
          Colors.green
        );
      case 'syncing':
        return (
          const SizedBox(
            width: 12,
            height: 12,
            child: CircularProgressIndicator(strokeWidth: 1.5, color: Colors.blue),
          ),
          Colors.blue
        );
      case 'conflict':
        return (
          const Icon(Icons.warning, size: 14, color: Colors.orange),
          Colors.orange
        );
      case 'error':
        return (
          const Icon(Icons.error, size: 14, color: Colors.red),
          Colors.red
        );
      case 'unsupported':
        return (
          const Icon(Icons.block, size: 14, color: Colors.grey),
          Colors.grey
        );
      default:
        return (
          const Icon(Icons.help_outline, size: 14, color: Colors.grey),
          Colors.grey
        );
    }
  }

  String _label() {
    switch (status) {
      case 'synced':
        return 'Synced';
      case 'syncing':
        return 'Syncing';
      case 'conflict':
        return 'Conflict';
      case 'error':
        return 'Error';
      case 'unsupported':
        return 'Not synced (Google format)';
      default:
        return status;
    }
  }
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

class _EmptyFilesPlaceholder extends StatelessWidget {
  final VoidCallback onRefresh;

  const _EmptyFilesPlaceholder({required this.onRefresh});

  @override
  Widget build(BuildContext context) {
    return ListView(
      children: [
        SizedBox(
          height: 360,
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Icon(
                Icons.folder_open,
                size: 64,
                color: Theme.of(context).colorScheme.outlineVariant,
              ),
              const SizedBox(height: 16),
              Text(
                'No files yet',
                style: Theme.of(context).textTheme.titleMedium?.copyWith(
                      color: Theme.of(context).colorScheme.onSurfaceVariant,
                    ),
              ),
              const SizedBox(height: 8),
              Text(
                'Files synced from Google Drive will appear here.',
                style: Theme.of(context).textTheme.bodySmall?.copyWith(
                      color: Theme.of(context).colorScheme.outlineVariant,
                    ),
              ),
              const SizedBox(height: 24),
              OutlinedButton.icon(
                onPressed: onRefresh,
                icon: const Icon(Icons.refresh),
                label: const Text('Refresh'),
              ),
            ],
          ),
        ),
      ],
    );
  }
}
