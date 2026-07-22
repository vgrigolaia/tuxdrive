import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'ipc/daemon_client.dart';
import 'providers/sync_provider.dart';
import 'screens/login_screen.dart';
import 'screens/main_screen.dart';

void main() {
  runApp(const TuxDriveApp());
}

class TuxDriveApp extends StatelessWidget {
  const TuxDriveApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MultiProvider(
      providers: [
        ChangeNotifierProvider(create: (_) => SyncProvider(DaemonClient())),
      ],
      child: MaterialApp(
        title: 'TuxDrive',
        theme: ThemeData(
          colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xFF1A73E8)),
          useMaterial3: true,
        ),
        darkTheme: ThemeData.dark(useMaterial3: true).copyWith(
          colorScheme: ColorScheme.fromSeed(
            seedColor: const Color(0xFF1A73E8),
            brightness: Brightness.dark,
          ),
        ),
        themeMode: ThemeMode.system,
        home: const AppRoot(),
      ),
    );
  }
}

class AppRoot extends StatefulWidget {
  const AppRoot({super.key});

  @override
  State<AppRoot> createState() => _AppRootState();
}

class _AppRootState extends State<AppRoot> {
  @override
  void initState() {
    super.initState();
    // Start connection attempt after the first frame.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<SyncProvider>().connect();
      context.read<SyncProvider>().checkForUpdate();
    });
  }

  @override
  Widget build(BuildContext context) {
    return Consumer<SyncProvider>(
      builder: (context, sync, _) {
        if (!sync.isConnected || sync.accountEmail.isEmpty) {
          return const LoginScreen();
        }
        return const MainScreen();
      },
    );
  }
}
