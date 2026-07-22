import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import '../../providers/sync_provider.dart';

/// Displays daemon log output in a scrollable, monospace view.
class LogsTab extends StatefulWidget {
  const LogsTab({super.key});

  @override
  State<LogsTab> createState() => _LogsTabState();
}

class _LogsTabState extends State<LogsTab> {
  final ScrollController _scrollController = ScrollController();
  bool _autoScroll = true;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<SyncProvider>().refreshLogs();
    });
    _scrollController.addListener(_onScroll);
  }

  @override
  void dispose() {
    _scrollController.removeListener(_onScroll);
    _scrollController.dispose();
    super.dispose();
  }

  void _onScroll() {
    if (!_scrollController.hasClients) return;
    final atBottom = _scrollController.position.pixels >=
        _scrollController.position.maxScrollExtent - 40;
    if (_autoScroll != atBottom) {
      setState(() => _autoScroll = atBottom);
    }
  }

  void _scrollToBottom() {
    if (_scrollController.hasClients) {
      _scrollController.animateTo(
        _scrollController.position.maxScrollExtent,
        duration: const Duration(milliseconds: 300),
        curve: Curves.easeOut,
      );
    }
  }

  Future<void> _refresh(SyncProvider sync) async {
    await sync.refreshLogs();
    if (_autoScroll) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _scrollToBottom());
    }
  }

  @override
  Widget build(BuildContext context) {
    return Consumer<SyncProvider>(
      builder: (context, sync, _) {
        final logs = sync.logs;

        return Column(
          children: [
            // ---- Toolbar ----
            _LogsToolbar(
              onRefresh: () => _refresh(sync),
              onCopy: logs.isEmpty ? null : () => _copyToClipboard(logs),
              onScrollToBottom: _scrollToBottom,
              autoScroll: _autoScroll,
            ),

            // ---- Log lines ----
            Expanded(
              child: logs.isEmpty
                  ? _EmptyLogsPlaceholder(onRefresh: () => _refresh(sync))
                  : _LogListView(
                      logs: logs,
                      scrollController: _scrollController,
                    ),
            ),
          ],
        );
      },
    );
  }

  void _copyToClipboard(List<String> logs) {
    Clipboard.setData(ClipboardData(text: logs.join('\n')));
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(
        content: Text('Log lines copied to clipboard'),
        duration: Duration(seconds: 2),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------

class _LogsToolbar extends StatelessWidget {
  final VoidCallback onRefresh;
  final VoidCallback? onCopy;
  final VoidCallback onScrollToBottom;
  final bool autoScroll;

  const _LogsToolbar({
    required this.onRefresh,
    required this.onCopy,
    required this.onScrollToBottom,
    required this.autoScroll,
  });

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;

    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
      decoration: BoxDecoration(
        color: colorScheme.surfaceContainerLow,
        border: Border(
          bottom: BorderSide(color: colorScheme.outlineVariant),
        ),
      ),
      child: Row(
        children: [
          Text(
            'Daemon Logs',
            style: Theme.of(context).textTheme.labelLarge?.copyWith(
                  color: colorScheme.onSurfaceVariant,
                ),
          ),
          const Spacer(),
          if (!autoScroll)
            TextButton.icon(
              onPressed: onScrollToBottom,
              icon: const Icon(Icons.arrow_downward, size: 16),
              label: const Text('Jump to bottom'),
              style: TextButton.styleFrom(visualDensity: VisualDensity.compact),
            ),
          IconButton(
            onPressed: onCopy,
            icon: const Icon(Icons.copy, size: 18),
            tooltip: 'Copy all logs',
            visualDensity: VisualDensity.compact,
          ),
          IconButton(
            onPressed: onRefresh,
            icon: const Icon(Icons.refresh, size: 18),
            tooltip: 'Refresh logs',
            visualDensity: VisualDensity.compact,
          ),
        ],
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Log list
// ---------------------------------------------------------------------------

class _LogListView extends StatelessWidget {
  final List<String> logs;
  final ScrollController scrollController;

  const _LogListView({
    required this.logs,
    required this.scrollController,
  });

  Color _lineColor(BuildContext context, String line) {
    final lower = line.toLowerCase();
    if (lower.contains('error') || lower.contains('fatal')) {
      return Colors.red.shade300;
    }
    if (lower.contains('warn')) return Colors.orange.shade300;
    if (lower.contains('debug')) {
      return Theme.of(context).colorScheme.onSurfaceVariant;
    }
    return Theme.of(context).colorScheme.onSurface;
  }

  @override
  Widget build(BuildContext context) {
    return ListView.builder(
      controller: scrollController,
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      itemCount: logs.length,
      itemBuilder: (context, index) {
        final line = logs[index];
        return SelectableText(
          line,
          style: TextStyle(
            fontFamily: 'monospace',
            fontSize: 12,
            height: 1.6,
            color: _lineColor(context, line),
          ),
        );
      },
    );
  }
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

class _EmptyLogsPlaceholder extends StatelessWidget {
  final VoidCallback onRefresh;

  const _EmptyLogsPlaceholder({required this.onRefresh});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(
            Icons.terminal,
            size: 56,
            color: Theme.of(context).colorScheme.outlineVariant,
          ),
          const SizedBox(height: 12),
          Text(
            'No log entries',
            style: Theme.of(context).textTheme.titleMedium?.copyWith(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
          ),
          const SizedBox(height: 16),
          OutlinedButton.icon(
            onPressed: onRefresh,
            icon: const Icon(Icons.refresh),
            label: const Text('Load logs'),
          ),
        ],
      ),
    );
  }
}
