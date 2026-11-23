using Xunit;

namespace AirFlash.UI.Tests;

/// <summary>
/// Autostart launches with --tray and must stay in the tray. The window pop
/// on login came from Avalonia auto-showing an assigned MainWindow, which
/// App now avoids; these tests pin the argument detection that decides
/// whether ShowPanel runs at startup at all.
/// </summary>
public sealed class TrayStartTests
{
    [Fact]
    public void Tray_flag_means_start_in_tray()
    {
        Assert.True(App.StartsInTray(["--tray"]));
        Assert.True(App.StartsInTray(["/home/dev/.local/opt/airflash/AirFlash.UI", "--tray"]));
    }

    [Fact]
    public void No_tray_flag_means_show_the_panel()
    {
        Assert.False(App.StartsInTray([]));
        Assert.False(App.StartsInTray(null));
        Assert.False(App.StartsInTray(["--tray-icon"]));
        Assert.False(App.StartsInTray(["--Tray"]));
    }
}
