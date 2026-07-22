// Basic smoke test — replace with real unit/widget tests.
import 'package:flutter_test/flutter_test.dart';
import 'package:tuxdrive_flutter/main.dart';

void main() {
  testWidgets('App builds without crashing', (WidgetTester tester) async {
    await tester.pumpWidget(const TuxDriveApp());
    // The SetupScreen should appear when the daemon is not running.
    expect(find.text('Connect Google Drive'), findsOneWidget);
  });
}
