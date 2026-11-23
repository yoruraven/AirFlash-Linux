using AirFlash.Core;
using AirFlash.UI.Services;
using AirFlash.UI.ViewModels;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Platform;
using Avalonia.Markup.Xaml;
using Avalonia.Styling;
using Avalonia.Threading;

namespace AirFlash.UI;

public partial class App : Application
{
    private TrayIcon? _tray;
    private Views.MainWindow? _mainWindow;
    private WindowIcon? _icon;
    private SingleInstance? _single;
    private IClassicDesktopStyleApplicationLifetime? _desktop;
    public AppViewModel? ViewModel { get; private set; }

    public override void Initialize() => AvaloniaXamlLoader.Load(this);

    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is not IClassicDesktopStyleApplicationLifetime desktop)
        {
            base.OnFrameworkInitializationCompleted();
            return;
        }
        _desktop = desktop;
        // The tray keeps the app alive with no window shown, so nothing but
        // Quit may end the process: never tie shutdown to a window closing.
        desktop.ShutdownMode = ShutdownMode.OnExplicitShutdown;
        // The lock is taken in Program.Main, before Avalonia starts.
        _single = AppHost.Instance ?? new SingleInstance();
        _single.Listen(() => Dispatcher.UIThread.Post(ShowPanel));

        L.Initialize(CulturePreferences());
        _icon = new WindowIcon(OpenIcon());
        ViewModel = new AppViewModel(
            new SettingsStore(AppPaths.SettingsPath),
            new LinuxDiscovery(),
            new LinuxAutostart(),
            new LinuxAudioService(),
            new ProcessEngineFactory(AppPaths.EnginePath(), AppPaths.Log));
        ApplyTheme();
        ActualThemeVariantChanged += (_, _) => ApplyTheme();

        var window = new Views.MainWindow { DataContext = ViewModel, Icon = _icon };
        window.Closing += (_, args) =>
        {
            // Closing keeps the sender in the tray; only Quit exits.
            if (!ViewModel.IsClosing)
            {
                args.Cancel = true;
                window.Hide();
            }
        };
        // Do NOT assign desktop.MainWindow here. Avalonia shows whatever is
        // assigned to MainWindow automatically as soon as this method
        // returns, regardless of IsVisible or a later Hide() — that is what
        // made every --tray autostart pop the panel on login. The window is
        // kept in _mainWindow and only becomes MainWindow in ShowPanel(),
        // when it is actually meant to be seen.
        _mainWindow = window;
        ConfigureTray();
        ViewModel.Start();
        // The autostart entry launches with --tray: stay in the tray and
        // only show the panel on demand (tray menu or a second launch).
        var startInTray = StartsInTray(desktop.Args);
        ViewModel.Startup.ContinueWith(
            _ => Dispatcher.UIThread.Post(() => { if (!startInTray) ShowPanel(); }),
            TaskScheduler.Default);
        base.OnFrameworkInitializationCompleted();
    }

    private static System.IO.Stream OpenIcon() =>
        AssetLoader.Open(new Uri("avares://AirFlash.UI/Assets/app.png"));

    /// <summary>Desktop locale from the environment; the Windows build reads the OS languages.</summary>
    private static IEnumerable<string> CulturePreferences()
    {
        foreach (var name in new[] { "LC_ALL", "LC_MESSAGES", "LANG" })
        {
            var value = Environment.GetEnvironmentVariable(name);
            if (!string.IsNullOrWhiteSpace(value)) yield return value;
        }
    }

    private void ConfigureTray()
    {
        if (_desktop is null) return;
        var menu = new NativeMenu();
        menu.Items.Add(new NativeMenuItem(L.Get("Show AirFlash")) { Command = new ShowCommand(this) });
        menu.Items.Add(new NativeMenuItemSeparator());
        menu.Items.Add(new NativeMenuItem(L.Get("Quit")) { Command = new QuitCommand(this) });
        _tray = new TrayIcon
        {
            Icon = _icon ?? new WindowIcon(OpenIcon()),
            ToolTipText = "AirFlash",
            Menu = menu,
            IsVisible = true,
        };
        TrayIcon.SetIcons(this, [_tray]);
    }

    private void ApplyTheme()
    {
        if (ViewModel is null) return;
        RequestedThemeVariant = ViewModel.Settings.Theme switch
        {
            "dark" => ThemeVariant.Dark,
            "light" => ThemeVariant.Light,
            _ => ActualThemeVariant,
        };
    }

    /// <summary>True when the launch asks for tray-only start (autostart).</summary>
    internal static bool StartsInTray(string[]? args) =>
        args is not null && Array.IndexOf(args, "--tray") >= 0;

    public void ShowPanel()
    {
        if (_mainWindow is not { } window) return;
        // Becoming MainWindow is safe now: the framework's automatic show
        // only happens for a MainWindow assigned during initialization.
        if (_desktop is not null && _desktop.MainWindow is null)
            _desktop.MainWindow = window;
        window.Show();
        window.WindowState = WindowState.Normal;
        window.Activate();
    }

    public async void Quit()
    {
        if (ViewModel is not null)
        {
            try { await ViewModel.DisposeAsync(); }
            catch (Exception error) { AppPaths.Log(error.ToString()); }
        }
        _single?.Dispose();
        _desktop?.Shutdown();
    }

    private sealed class ShowCommand(App app) : System.Windows.Input.ICommand
    {
#pragma warning disable CS0067 // Raised only by the platform when the item list changes.
        public event EventHandler? CanExecuteChanged;
#pragma warning restore CS0067
        public bool CanExecute(object? parameter) => true;
        public void Execute(object? parameter) => app.ShowPanel();
    }

    private sealed class QuitCommand(App app) : System.Windows.Input.ICommand
    {
#pragma warning disable CS0067 // Raised only by the platform when the item list changes.
        public event EventHandler? CanExecuteChanged;
#pragma warning restore CS0067
        public bool CanExecute(object? parameter) => true;
        public async void Execute(object? parameter) => app.Quit();
    }
}
