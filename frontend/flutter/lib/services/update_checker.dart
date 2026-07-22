import 'dart:convert';
import 'dart:io';

/// Result of a GitHub release version check.
class UpdateCheckResult {
  final bool updateAvailable;
  final String? latestVersion;
  final String? releaseUrl;

  const UpdateCheckResult({
    required this.updateAvailable,
    this.latestVersion,
    this.releaseUrl,
  });

  static const none = UpdateCheckResult(updateAvailable: false);
}

/// Checks GitHub Releases for a TuxDrive version newer than the one running.
///
/// Lightweight by design: just a version comparison against the latest
/// GitHub release tag, surfaced as a dismissible banner in the GUI — no
/// downloading or auto-installing. Never throws; network failures, rate
/// limiting, or unparseable responses all just resolve to "no update
/// available" so a flaky connection can't disrupt startup.
class UpdateChecker {
  static const _apiUrl =
      'https://api.github.com/repos/vgrigolaia/tuxdrive/releases/latest';

  Future<UpdateCheckResult> check(String currentVersion) async {
    HttpClient? client;
    try {
      client = HttpClient()..connectionTimeout = const Duration(seconds: 5);
      final request = await client.getUrl(Uri.parse(_apiUrl));
      request.headers.set('Accept', 'application/vnd.github+json');
      request.headers.set('User-Agent', 'TuxDrive');
      final response = await request.close().timeout(const Duration(seconds: 5));
      if (response.statusCode != 200) return UpdateCheckResult.none;

      final body = await response.transform(utf8.decoder).join();
      final json = jsonDecode(body) as Map<String, dynamic>;
      final tag = json['tag_name'] as String?;
      if (tag == null) return UpdateCheckResult.none;

      final latest = tag.startsWith('v') ? tag.substring(1) : tag;
      if (!_isNewer(latest, currentVersion)) return UpdateCheckResult.none;

      return UpdateCheckResult(
        updateAvailable: true,
        latestVersion: latest,
        releaseUrl: json['html_url'] as String?,
      );
    } catch (_) {
      return UpdateCheckResult.none;
    } finally {
      client?.close();
    }
  }

  /// Simple x.y.z numeric comparison. Anything that doesn't parse as exactly
  /// three numeric parts (pre-release tags, malformed versions, ...) is
  /// treated as "not newer" to avoid false-positive notifications.
  bool _isNewer(String latest, String current) {
    final l = _parseVersion(latest);
    final c = _parseVersion(current);
    if (l == null || c == null) return false;
    for (var i = 0; i < 3; i++) {
      if (l[i] != c[i]) return l[i] > c[i];
    }
    return false;
  }

  List<int>? _parseVersion(String version) {
    final parts = version.split('.');
    if (parts.length != 3) return null;
    final nums = <int>[];
    for (final part in parts) {
      final n = int.tryParse(part);
      if (n == null) return null;
      nums.add(n);
    }
    return nums;
  }
}
