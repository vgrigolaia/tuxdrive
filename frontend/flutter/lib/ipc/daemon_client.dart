import 'dart:async';
import 'dart:convert';
import 'dart:io';

/// Snapshot of daemon status returned by `get_status`.
class DaemonStatus {
  final String status;
  final int queued;
  final String accountEmail;
  final bool paused;
  /// Estimated seconds remaining to drain the queue, based on recent
  /// throughput. `null` when there's not enough recent history yet.
  final int? etaSeconds;

  const DaemonStatus({
    required this.status,
    required this.queued,
    required this.accountEmail,
    required this.paused,
    this.etaSeconds,
  });

  factory DaemonStatus.fromJson(Map<String, dynamic> json) => DaemonStatus(
        status: json['status'] as String? ?? 'unknown',
        queued: (json['queued'] as int?) ?? 0,
        accountEmail: json['account_email'] as String? ?? '',
        paused: (json['paused'] as bool?) ?? false,
        etaSeconds: json['eta_seconds'] as int?,
      );

  static DaemonStatus get disconnected => const DaemonStatus(
        status: 'disconnected',
        queued: 0,
        accountEmail: '',
        paused: false,
      );
}

/// Progress of a GUI-driven login, returned by `get_login_status`.
///
/// `phase` mirrors the daemon's `LoginState` variants: idle,
/// awaiting_browser, exchanging_code, conflict_pending, resolving_conflict,
/// complete, failed.
class LoginStatus {
  final String phase;
  final String? authUrl;
  final String? accountEmail;
  final int? knownCount;
  final int? missingCount;
  final List<String>? missingPaths;
  final int? resolvedDone;
  final int? resolvedTotal;
  final String? error;

  const LoginStatus({
    required this.phase,
    this.authUrl,
    this.accountEmail,
    this.knownCount,
    this.missingCount,
    this.missingPaths,
    this.resolvedDone,
    this.resolvedTotal,
    this.error,
  });

  factory LoginStatus.fromJson(Map<String, dynamic> json) => LoginStatus(
        phase: json['phase'] as String? ?? 'idle',
        authUrl: json['auth_url'] as String?,
        accountEmail: json['account_email'] as String?,
        knownCount: json['known_count'] as int?,
        missingCount: json['missing_count'] as int?,
        missingPaths: (json['missing_paths'] as List<dynamic>?)?.cast<String>(),
        resolvedDone: json['resolved_done'] as int?,
        resolvedTotal: json['resolved_total'] as int?,
        error: json['error'] as String?,
      );

  static const idle = LoginStatus(phase: 'idle');
}

/// Result of a `set_sync_folder` request.
class SetSyncFolderResult {
  final bool success;
  final String? localRoot;
  final String? error;

  const SetSyncFolderResult({required this.success, this.localRoot, this.error});
}

/// Whether a custom (advanced/self-host) OAuth client is configured, and its
/// client_id if so. The client_secret is never sent back over IPC.
class AuthConfigInfo {
  final String? clientId;
  final bool isCustom;

  const AuthConfigInfo({this.clientId, required this.isCustom});

  static const unknown = AuthConfigInfo(isCustom: false);
}

/// Result of a `set_auth_config` request.
class SetAuthConfigResult {
  final bool success;
  final String? clientId;
  final bool isCustom;
  final String? error;

  const SetAuthConfigResult({
    required this.success,
    this.clientId,
    this.isCustom = false,
    this.error,
  });
}

/// Represents a single file or folder entry returned by `list_files`.
class FileEntry {
  final String name;
  final String relativePath;
  final int size;
  final String syncStatus;
  final bool isFolder;

  const FileEntry({
    required this.name,
    required this.relativePath,
    required this.size,
    required this.syncStatus,
    required this.isFolder,
  });

  factory FileEntry.fromJson(Map<String, dynamic> json) => FileEntry(
        name: json['name'] as String? ?? '',
        relativePath: json['relative_path'] as String? ?? '',
        size: (json['size'] as int?) ?? 0,
        syncStatus: json['sync_status'] as String? ?? 'unknown',
        isFolder: (json['is_folder'] as bool?) ?? false,
      );
}

/// Low-level IPC client that speaks JSON-lines over a Unix domain socket
/// to the `tuxdrive-daemon` backend.
class DaemonClient {
  static String get _socketPath {
    final home = Platform.environment['HOME'] ?? '';
    return '$home/.local/share/tuxdrive/daemon.sock';
  }

  Socket? _socket;
  StreamSubscription<String>? _subscription;
  final _responseController =
      StreamController<Map<String, dynamic>>.broadcast();
  final _pendingCompleters = <Completer<Map<String, dynamic>>>[];
  String _buffer = '';

  bool get isConnected => _socket != null;

  /// Attempt to connect to the daemon socket. Returns `true` on success.
  Future<bool> connect() async {
    if (_socket != null) return true;
    try {
      _socket = await Socket.connect(
        InternetAddress(_socketPath, type: InternetAddressType.unix),
        0,
        timeout: const Duration(seconds: 5),
      );
      _subscription = utf8.decoder
          .bind(_socket!)
          .listen(_onData, onError: _onError, onDone: _onDone);
      return true;
    } catch (_) {
      _socket = null;
      return false;
    }
  }

  void _onData(String data) {
    _buffer += data;
    while (_buffer.contains('\n')) {
      final idx = _buffer.indexOf('\n');
      final line = _buffer.substring(0, idx).trim();
      _buffer = _buffer.substring(idx + 1);
      if (line.isEmpty) continue;
      try {
        final parsed = jsonDecode(line) as Map<String, dynamic>;
        _responseController.add(parsed);
        if (_pendingCompleters.isNotEmpty) {
          _pendingCompleters.removeAt(0).complete(parsed);
        }
      } catch (_) {
        // Ignore malformed lines.
      }
    }
  }

  void _onError(Object error) => disconnect();

  void _onDone() => disconnect();

  /// Close the socket and fail any pending requests.
  void disconnect() {
    _subscription?.cancel();
    _subscription = null;
    _socket?.destroy();
    _socket = null;
    _buffer = '';
    for (final c in _pendingCompleters) {
      c.completeError(const SocketException('Disconnected'));
    }
    _pendingCompleters.clear();
  }

  Future<Map<String, dynamic>> _send(Map<String, dynamic> command) async {
    if (_socket == null) throw const SocketException('Not connected');
    final completer = Completer<Map<String, dynamic>>();
    _pendingCompleters.add(completer);
    _socket!.write('${jsonEncode(command)}\n');
    return completer.future.timeout(
      const Duration(seconds: 10),
      onTimeout: () {
        _pendingCompleters.remove(completer);
        throw TimeoutException('Daemon did not respond in time');
      },
    );
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  Future<DaemonStatus> getStatus() async {
    try {
      final resp = await _send({'cmd': 'get_status'});
      return DaemonStatus.fromJson(resp);
    } catch (_) {
      return DaemonStatus.disconnected;
    }
  }

  Future<bool> pause() async {
    try {
      await _send({'cmd': 'pause'});
      return true;
    } catch (_) {
      return false;
    }
  }

  Future<bool> resume() async {
    try {
      await _send({'cmd': 'resume'});
      return true;
    } catch (_) {
      return false;
    }
  }

  Future<bool> logout(String email) async {
    try {
      await _send({'cmd': 'logout', 'email': email});
      return true;
    } catch (_) {
      return false;
    }
  }

  Future<List<FileEntry>> listFiles({String folderPath = ''}) async {
    try {
      final resp =
          await _send({'cmd': 'list_files', 'folder_path': folderPath});
      final entries = resp['entries'] as List<dynamic>? ?? [];
      return entries
          .map((e) => FileEntry.fromJson(e as Map<String, dynamic>))
          .toList();
    } catch (_) {
      return [];
    }
  }

  Future<List<String>> getLogs({int lines = 100}) async {
    try {
      final resp = await _send({'cmd': 'get_logs', 'lines': lines});
      return (resp['lines'] as List<dynamic>? ?? []).cast<String>();
    } catch (_) {
      return [];
    }
  }

  Future<bool> shutdown() async {
    try {
      await _send({'cmd': 'shutdown'});
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Begin a GUI-driven OAuth login. Returns the URL to open in a browser,
  /// or `null` if the daemon rejected the request (e.g. a login is already
  /// in progress) or the call failed.
  Future<String?> startLogin() async {
    try {
      final resp = await _send({'cmd': 'start_login'});
      if (resp['type'] == 'error') return null;
      return resp['auth_url'] as String?;
    } catch (_) {
      return null;
    }
  }

  Future<LoginStatus> getLoginStatus() async {
    try {
      final resp = await _send({'cmd': 'get_login_status'});
      if (resp['type'] == 'error') {
        return LoginStatus(
          phase: 'failed',
          error: resp['message'] as String? ?? 'unknown error',
        );
      }
      return LoginStatus.fromJson(resp);
    } catch (_) {
      return LoginStatus.idle;
    }
  }

  /// Resolve a pending sync-history conflict. `action` is `'download'` or
  /// `'delete_confirmed'`.
  Future<bool> resolveSyncConflict(String action) async {
    try {
      final resp = await _send({
        'cmd': 'resolve_sync_conflict',
        'action': action,
      });
      return resp['type'] != 'error';
    } catch (_) {
      return false;
    }
  }

  Future<bool> cancelLogin() async {
    try {
      await _send({'cmd': 'cancel_login'});
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Current sync folder location, or `null` if the request failed.
  Future<String?> getSyncFolder() async {
    try {
      final resp = await _send({'cmd': 'get_sync_settings'});
      if (resp['type'] == 'error') return null;
      return resp['local_root'] as String?;
    } catch (_) {
      return null;
    }
  }

  /// Relocate the sync folder to `path`. On success the daemon restarts
  /// itself to apply the change, which will briefly disconnect this client.
  Future<SetSyncFolderResult> setSyncFolder(String path) async {
    try {
      final resp = await _send({'cmd': 'set_sync_folder', 'path': path});
      if (resp['type'] == 'error') {
        return SetSyncFolderResult(
          success: false,
          error: resp['message'] as String? ?? 'Unknown error',
        );
      }
      return SetSyncFolderResult(
        success: true,
        localRoot: resp['local_root'] as String?,
      );
    } catch (e) {
      return SetSyncFolderResult(
        success: false,
        error: 'Could not reach the daemon: $e',
      );
    }
  }

  /// Whether a custom (advanced/self-host) OAuth client is configured.
  Future<AuthConfigInfo> getAuthConfig() async {
    try {
      final resp = await _send({'cmd': 'get_auth_config'});
      if (resp['type'] == 'error') return AuthConfigInfo.unknown;
      return AuthConfigInfo(
        clientId: resp['client_id'] as String?,
        isCustom: (resp['is_custom'] as bool?) ?? false,
      );
    } catch (_) {
      return AuthConfigInfo.unknown;
    }
  }

  /// Set a custom OAuth client_id/client_secret (or clear the override if
  /// both are empty). On success the daemon restarts itself to apply the
  /// change, which will briefly disconnect this client.
  Future<SetAuthConfigResult> setAuthConfig(
    String clientId,
    String clientSecret,
  ) async {
    try {
      final resp = await _send({
        'cmd': 'set_auth_config',
        'client_id': clientId,
        'client_secret': clientSecret,
      });
      if (resp['type'] == 'error') {
        return SetAuthConfigResult(
          success: false,
          error: resp['message'] as String? ?? 'Unknown error',
        );
      }
      return SetAuthConfigResult(
        success: true,
        clientId: resp['client_id'] as String?,
        isCustom: (resp['is_custom'] as bool?) ?? false,
      );
    } catch (e) {
      return SetAuthConfigResult(
        success: false,
        error: 'Could not reach the daemon: $e',
      );
    }
  }

  void dispose() {
    disconnect();
    _responseController.close();
  }
}
