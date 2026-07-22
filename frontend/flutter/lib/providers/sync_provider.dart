import 'dart:async';
import 'package:flutter/foundation.dart';
import '../ipc/daemon_client.dart';

/// Central state provider. Wraps [DaemonClient] and exposes reactive state to
/// the widget tree via [ChangeNotifier].
class SyncProvider extends ChangeNotifier {
  final DaemonClient _client;
  Timer? _pollTimer;
  Timer? _loginPollTimer;

  DaemonStatus _status = DaemonStatus.disconnected;
  bool _isConnected = false;
  bool _isConnecting = false;
  List<FileEntry> _files = [];
  List<String> _logs = [];
  LoginStatus _loginStatus = LoginStatus.idle;
  String? _syncFolder;

  SyncProvider(this._client);

  // ---------------------------------------------------------------------------
  // Accessors
  // ---------------------------------------------------------------------------

  bool get isConnected => _isConnected;
  bool get isConnecting => _isConnecting;
  String get syncStatus => _status.status;
  int get queuedCount => _status.queued;
  int? get etaSeconds => _status.etaSeconds;
  String get accountEmail => _status.accountEmail;
  bool get isPaused => _status.paused;
  List<FileEntry> get files => List.unmodifiable(_files);
  List<String> get logs => List.unmodifiable(_logs);

  String get loginPhase => _loginStatus.phase;
  String? get loginAuthUrl => _loginStatus.authUrl;
  int? get loginKnownCount => _loginStatus.knownCount;
  int? get loginMissingCount => _loginStatus.missingCount;
  List<String>? get loginMissingPaths => _loginStatus.missingPaths;
  int? get loginResolvedDone => _loginStatus.resolvedDone;
  int? get loginResolvedTotal => _loginStatus.resolvedTotal;
  String? get loginError => _loginStatus.error;
  String? get syncFolder => _syncFolder;

  // ---------------------------------------------------------------------------
  // Connection management
  // ---------------------------------------------------------------------------

  /// Attempt to connect to the daemon and start the polling loop.
  Future<void> connect() async {
    if (_isConnecting) return;
    _isConnecting = true;
    notifyListeners();

    _isConnected = await _client.connect();
    _isConnecting = false;

    if (_isConnected) {
      await _refresh();
      await loadSyncFolder();
      _startPolling();
    } else {
      notifyListeners();
    }
  }

  void _startPolling() {
    _pollTimer?.cancel();
    _pollTimer = Timer.periodic(const Duration(seconds: 3), (_) async {
      if (!_isConnected) {
        _isConnected = await _client.connect();
      }
      if (_isConnected) {
        await _refresh();
      } else {
        notifyListeners();
      }
    });
  }

  Future<void> _refresh() async {
    _status = await _client.getStatus();
    _isConnected = _status.status != 'disconnected';
    notifyListeners();
  }

  // ---------------------------------------------------------------------------
  // Login
  // ---------------------------------------------------------------------------

  /// Kick off a GUI-driven OAuth login and start polling for progress.
  /// Returns `true` if the daemon accepted the request.
  Future<bool> startLogin() async {
    final authUrl = await _client.startLogin();
    if (authUrl == null) {
      _loginStatus = const LoginStatus(
        phase: 'failed',
        error: 'Could not start login — is the daemon running?',
      );
      notifyListeners();
      return false;
    }
    _loginStatus = LoginStatus(phase: 'awaiting_browser', authUrl: authUrl);
    notifyListeners();
    _startLoginPolling();
    return true;
  }

  void _startLoginPolling() {
    _loginPollTimer?.cancel();
    _loginPollTimer = Timer.periodic(const Duration(seconds: 1), (_) async {
      _loginStatus = await _client.getLoginStatus();
      notifyListeners();
      if (_loginStatus.phase == 'complete') {
        _loginPollTimer?.cancel();
        await _refresh();
      } else if (_loginStatus.phase == 'failed') {
        _loginPollTimer?.cancel();
      }
    });
  }

  /// Resolve a pending sync-history conflict (`'download'` or
  /// `'delete_confirmed'`); polling picks up the resulting progress.
  Future<void> resolveSyncConflict(String action) async {
    await _client.resolveSyncConflict(action);
    _loginStatus = await _client.getLoginStatus();
    notifyListeners();
    if (_loginStatus.phase == 'resolving_conflict' && _loginPollTimer == null) {
      _startLoginPolling();
    }
  }

  Future<void> cancelLogin() async {
    _loginPollTimer?.cancel();
    _loginPollTimer = null;
    await _client.cancelLogin();
    _loginStatus = LoginStatus.idle;
    notifyListeners();
  }

  // ---------------------------------------------------------------------------
  // Commands
  // ---------------------------------------------------------------------------

  Future<void> pause() async {
    await _client.pause();
    await _refresh();
  }

  Future<void> resume() async {
    await _client.resume();
    await _refresh();
  }

  Future<void> logout() async {
    await _client.logout(accountEmail);
    _status = DaemonStatus.disconnected;
    _isConnected = false;
    _files = [];
    _logs = [];
    _loginStatus = LoginStatus.idle;
    _pollTimer?.cancel();
    _loginPollTimer?.cancel();
    notifyListeners();
  }

  Future<void> refreshFiles({String folderPath = ''}) async {
    _files = await _client.listFiles(folderPath: folderPath);
    notifyListeners();
  }

  Future<void> refreshLogs() async {
    _logs = await _client.getLogs(lines: 200);
    notifyListeners();
  }

  Future<void> shutdownDaemon() async {
    await _client.shutdown();
    _status = DaemonStatus.disconnected;
    _isConnected = false;
    _pollTimer?.cancel();
    notifyListeners();
  }

  Future<void> loadSyncFolder() async {
    _syncFolder = await _client.getSyncFolder();
    notifyListeners();
  }

  /// Relocate the sync folder to `path`, moving existing synced files there.
  /// Returns an error message on failure, or `null` on success — on success
  /// the daemon restarts itself to apply the change, which briefly
  /// disconnects this client; the normal polling loop reconnects it.
  Future<String?> changeSyncFolder(String path) async {
    final result = await _client.setSyncFolder(path);
    if (!result.success) {
      return result.error ?? 'Unknown error';
    }
    _syncFolder = result.localRoot;
    notifyListeners();
    return null;
  }

  @override
  void dispose() {
    _pollTimer?.cancel();
    _loginPollTimer?.cancel();
    _client.dispose();
    super.dispose();
  }
}
