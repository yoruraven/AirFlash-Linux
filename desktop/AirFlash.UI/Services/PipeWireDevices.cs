using System.Diagnostics;
using System.Text.Json;

namespace AirFlash.UI.Services;

/// <summary>One capturable PipeWire node, for the capture-device picker.</summary>
public sealed record CaptureDeviceOption(string? NodeName, string Label)
{
    public override string ToString() => Label;
}

/// <summary>
/// Lists capturable PipeWire nodes by parsing <c>pw-dump</c>, mirroring the
/// engine's own graph parsing: sinks are captured through their monitor,
/// sources (microphones, USB audio inputs, Bluetooth A2DP sources) are
/// captured directly. Any failure (no pw-dump, PipeWire down) yields an
/// empty list and the picker keeps only its default entry.
/// </summary>
public static class PipeWireDevices
{
    public static IReadOnlyList<CaptureDeviceOption> List()
    {
        var sinks = new List<CaptureDeviceOption>();
        var sources = new List<CaptureDeviceOption>();
        try
        {
            using var process = Process.Start(new ProcessStartInfo("pw-dump")
            {
                RedirectStandardOutput = true,
                UseShellExecute = false,
                CreateNoWindow = true,
            });
            if (process is null) return [];
            var json = process.StandardOutput.ReadToEnd();
            if (!process.WaitForExit(5000))
            {
                try { process.Kill(); } catch { /* best effort */ }
                return [];
            }
            using var document = JsonDocument.Parse(json);
            foreach (var item in document.RootElement.EnumerateArray())
            {
                if (!item.TryGetProperty("type", out var type) ||
                    !(type.GetString() ?? "").EndsWith("Node")) continue;
                if (!item.TryGetProperty("info", out var info) ||
                    !info.TryGetProperty("props", out var props)) continue;
                var mediaClass = props.TryGetProperty("media.class", out var mc) ? mc.GetString() : null;
                var isSink = mediaClass == "Audio/Sink";
                var isSource = mediaClass is not null && mediaClass.StartsWith("Audio/Source");
                if (!isSink && !isSource) continue;
                var name = props.TryGetProperty("node.name", out var nn) ? nn.GetString() : null;
                if (string.IsNullOrEmpty(name)) continue;
                var description = props.TryGetProperty("node.description", out var nd) ? nd.GetString() : null;
                var label = string.IsNullOrWhiteSpace(description) ? name : description;
                (isSink ? sinks : sources).Add(new CaptureDeviceOption(name, label));
            }
        }
        catch { /* no usable PipeWire graph: default entry only */ }
        return [.. sinks, .. sources];
    }
}
