# <img src="../desktop/AirFlash.App/Assets/app.svg" width="36" height="36" alt="AirFlash 标志" style="vertical-align:middle" /> AirFlash Linux — 把 Linux 桌面声音投到 HomePod（AirPlay 2）

<p align="center">
  <a href="../README.md">English</a> ·
  <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/yoruraven/AirFlash-Linux/releases/latest"><img src="https://img.shields.io/github/v/release/yoruraven/AirFlash-Linux" alt="AirFlash Linux 最新版本" /></a>
  <img src="https://img.shields.io/badge/platform-Linux%20x86__64%20%7C%20arm64-blue" alt="平台：Linux x86_64 与 arm64" />
  <img src="https://img.shields.io/badge/AirPlay%202-sender-black" alt="Linux 上的 AirPlay 2 发送端" />
  <img src="https://img.shields.io/badge/license-GPLv3%2B%20or%20Commercial-green" alt="许可证：GPLv3+ 或商业授权" />
</p>

<p align="center">
  <img src="images/airflash-linux-gnome.png" alt="AirFlash Linux 在 GNOME 上的图形面板：把 Linux 桌面声音通过 AirPlay 2 投送到 HomePod" width="550" />
</p>

**AirFlash Linux 是一款原生 AirPlay 2 发送工具，可以把 Linux 桌面上的全部声音发送到 HomePod、HomePod mini 或已有的 HomePod 立体声组合。** 它是 [AirFlash](https://github.com/Ding-Kyoma/AirFlash) 的 Linux 版本，与 Windows 版共用同一套原生 Rust AirPlay 2 引擎，既有图形面板，也有无头 CLI。系统声音通过 PipeWire 采集；如果机器上只有 PulseAudio，则会回退到 `parec`。

想让 HomePod 给 Linux 电脑当音箱，或者在 Linux 上找 TuneBlade、Airfoil 的替代方案，可以试试它。它不是媒体服务器，不用先导入曲库：打开面板，选好接收端，桌面声音就能在 HomePod 上播放。

> **注意：它是发送端，不是接收端。** AirFlash Linux 负责把声音从 Linux 发到 AirPlay 2 音箱。Shairport Sync、UxPlay 做的是相反的事：它们让 Linux 变成 AirPlay 接收端。

## 快速开始

1. 从 [GitHub Releases](https://github.com/yoruraven/AirFlash-Linux/releases/latest)
   下载适合当前架构的 `-full` AppImage：

   ```bash
   chmod +x AirFlash-X.Y.Z-x86_64-full.AppImage
   ./AirFlash-X.Y.Z-x86_64-full.AppImage   # 打开图形面板
   ```

   不用安装，也不需要 root。AppImage 是自包含的单文件，面板、CLI、引擎和
   .NET 运行时都打包在里面。文件没有代码签名，下载后请用 Release 附带的
   `SHA256SUMS.txt` 校验完整性。

2. 让电脑和 HomePod 连到同一个局域网，打开 AirFlash，等接收端出现在列表里。

3. 选好接收端后点播放。立体声组合要选择完整的
   `HomePod stereo · 2/2` 条目；想播放桌面声音，还要在
   **设置 → 音频来源** 中选择系统声音。

也可以直接用终端：同一个 AppImage 内置了无头 CLI，带参数运行时启动的是
CLI，而不是图形面板：

```bash
./AirFlash-X.Y.Z-x86_64-full.AppImage discover
./AirFlash-X.Y.Z-x86_64-full.AppImage start --device "HomePod" --source loopback
```

或使用单独安装的 `airflash-cli`：

```bash
airflash-cli discover                                   # 发现接收端（mDNS）
airflash-cli pair --host <接收端IP>                      # 仅当设备要求 PIN 时需要
airflash-cli start --host <接收端IP> --source loopback --daemon
airflash-cli status
airflash-cli stop
```

依赖项、systemd `--user` 服务、完整命令说明，以及后文提到的 PTP 端口
设置，见 [LINUX.md](LINUX.md)（英文）；无头 CLI 包的安装和使用见
[HEADLESS.md](HEADLESS.md)（英文）。

## 主要功能

- **原生 AirPlay 2**——使用与 Windows 版相同的 Rust 引擎，支持 HAP
  配对、PTP 时钟和加密 RTP。音频编码支持 PCM 和 ALAC，并优先使用
  ALAC；如果接收端拒绝当前编码，会自动换一种重试。只用 PCM 无法在
  HomePod 上建立播放。
- **真正采集声音**——`--source loopback` 通过明确连接的 monitor
  端口采集默认 PipeWire 输出，不靠名称猜测，也不会误抓麦克风；只有
  PulseAudio 时会回退到 `parec`。用 `--monitor NAME`（或图形面板
  **设置 → 音频来源** 里的捕获设备选择器）还能选择其他采集设备：
  另一个输出设备的监听，或者直接录制音源——麦克风、USB 音频输入、
  蓝牙 A2DP 音源都可以。
- **已适配 HomePod OS 26**——支持苹果最新的 HomePod 系统。单个
  HomePod 和现有的双机立体声组合都能使用，每台设备的音量也可以分别
  调节。
- **图形面板和无头 CLI 都有**——Avalonia 面板与 Windows 版共用核心
  代码，提供接收端列表、均衡器、诊断和设置；`airflash-cli` 更适合脚本
  和自动化，支持守护进程模式、JSON 输出和 `systemd --user` 服务。
  另外还有精简的 `-cli` 无头包（只有引擎、CLI 和 systemd 服务，
  按 glibc 2.35+ 构建），可以装在树莓派这类无头设备上常驻运行。
- **延迟可以调节**——提供实时（目标 120 ms）、正常（200 ms，默认）、
  缓冲和自定义模式，也可以锁定固定延迟。这里的数值是调度目标，不等同
  于实际测得的端到端声学延迟。
- **10 段均衡器**——带预设并可即时试听；还支持自动重连和静音待机：
  没有声音时释放会话，再次出声时自动恢复。连接问题也可以通过详细的
  诊断信息排查。
- **自包含 AppImage**——x86_64 和 arm64 两种架构都有，不用另外安装
  .NET、Python 或其他运行时。arm64 版目前还没有经过实机验收。
- 界面支持英文和简体中文。

## 更多玩法

- **树莓派当常驻串流机。** 把 `-cli` 无头包装到树莓派上，启用自带的
  systemd 用户服务，再让 Pi 上的播放器（MPD、mopidy、浏览器等）往
  默认输出播放，Pi 就成了一台专门给 HomePod 供声的 AirPlay 发送机。
- **给 HomePod 加蓝牙输入。** 手机先和 Linux 机器配对蓝牙
  （BlueZ + PipeWire），手机就会以蓝牙音源的形式出现在采集设备里；
  选它之后，手机放什么，HomePod 就响什么。
- **麦克风和 USB 输入。** 麦克风可以做简单的喊话、广播；USB 唱机、
  调音台等设备通过 USB 声卡接入后，任何线性音源都能变成 AirPlay
  音源。
- **脚本播报。** 用本地 TTS 生成 WAV 文件，再用文件音源播放出去——
  和装在同一台机器上的 Home Assistant 配合，正好做全屋语音提醒。

## 实测矩阵

下面这些结果都来自真实硬件测试，不只是用模拟接收端跑通流程。
最后更新：2026 年 11 月。

| 环境 | 接收端 | 声音来源 | 结果 |
| --- | --- | --- | --- |
| Linux · x86_64 · PipeWire | HomePod mini | 系统声音（PipeWire 监听）、WAV 文件、模拟信号 | ✅ 端到端出声（ALAC 44.1 kHz），接收端反馈正常 |
| Ubuntu 26.04 LTS · x86_64 · GNOME · PipeWire | HomePod 立体声组合（全尺寸 HomePod） | 模拟信号、系统声音（桌面采集） | ✅ 端到端出声，数千个包零发送错误 |
| Ubuntu 26.04 LTS · x86_64 | 本机 / 模拟接收端 | Rust、.NET、Python 自动化测试套件 | ✅ 全部通过 |
| aarch64（仅构建） | — | — | ⚠️ 打包脚本可构建，尚未实机验收 |

其他基于 glibc 的发行版理论上也能运行，运行时主要需要 glibc，以及
PipeWire 或 PulseAudio；不过目前只有表中环境经过实机验证。也欢迎补充
其他发行版的实测结果。另外，从真实 PipeWire 音源（本机模拟麦克风
输入）采集已经在采集层验证过；麦克风、USB 和蓝牙音源的端到端出声
还没有实机验收。

## 和同类方案的区别

| | AirFlash Linux | OwnTone | Shairport Sync | TuneBlade |
| --- | --- | --- | --- | --- |
| 方向 | **发送端**（Linux → 音箱） | 发送端（媒体服务器） | 接收端（音箱 → Linux） | 发送端（Windows → 音箱） |
| 投送实时桌面声音 | ✅ 任何应用、全系统 | 要靠文件/FIFO 喂流 | — | ✅ |
| AirPlay 2 投 HomePod | ✅ 原生 | ✅ | 接收 AirPlay 2 | ❌ 只有 AirPlay 1 |
| 图形面板 + CLI | ✅ 两者都有 | Web 界面 / API | ❌ | ✅ 仅 Windows |
| 能在 Linux 上跑 | ✅ | ✅ | ✅ | ❌ |

## 使用说明

- **配对。** 大多数接收端可以直接配对。如果接收端要求 PIN，在面板
  （**设置 → 接收器**）里或用 `airflash-cli pair` 配对一次即可。如果
  出现 `pairing rejected (TLV error [2])`，请打开「家庭」App，把扬声器
  访问权限设为「所有人」后重试。
- **PTP 使用 319/320 端口。** PTP 更适合需要精确同步的 AirPlay 2
  播放，但 UDP 319/320 属于特权端口。如果端口无法绑定，引擎会给出
  警告并自动回退到 NTP，不会直接让播放失败。想要完整的 PTP 同步，
  可以任选一种方式授权：给二进制执行
  `sudo setcap cap_net_bind_service=+ep`、调低
  `net.ipv4.ip_unprivileged_port_start`，或使用随附 systemd 服务中的
  `AmbientCapabilities` 配置。
- **要在同一网络，中间不要经过 NAT。** 接收端需要能通过 UDP 回连
  这台电脑。桥接网络通常没问题，NAT 模式的虚拟机一般不行。详见
  [PTP 启动排障](PTP-TROUBLESHOOTING.md)（英文）。

## 已知限制

- 只支持音频，不支持屏幕镜像、视频或杜比 Atmos。
- 按应用单独采集目前只有 Windows 版支持。Linux 版可以采集任意输出
  设备的监听或任意音源设备，但还不能只抓单个应用的声音；需要隔离
  时，可以先把应用的声音路由到单独的输出设备再采集。
- 设备发现使用内置的精简版 mDNS 查询器，不支持持续浏览，也不会回应
  IPv6 link-local 查询。
- 部分桌面环境下没有托盘图标。
- 采集、重采样和格式转换不保证逐位无损，也不保证能采到受 DRM
  保护的内容。
- AppImage 没有代码签名，请核对 `SHA256SUMS.txt`。

## 常见问题

**能让 HomePod 当 Linux 电脑的音箱吗？**
可以，这正是 AirFlash Linux 的主要用途。它会采集桌面声音（PipeWire
或 PulseAudio），再通过 AirPlay 2 发送到 HomePod 或 HomePod mini。

**支持 HomePod 立体声组合吗？**
支持。已经组好的双 HomePod 立体声组合会显示为一个 `2/2` 条目，并
保持同步播放。共享引擎还带实验性的双接收端软件立体声功能，但 Linux
版还没有实机验证。

**它是 Linux 上的 AirPlay 接收端吗？**
不是。Shairport Sync、UxPlay 这类接收端会把 Linux 变成一个 AirPlay
音箱；AirFlash Linux 则相反，它是发送端：Linux 负责出声，HomePod
负责播放。

**必须用 PipeWire 吗？**
现在主流发行版基本都用 PipeWire，这是首选路径；只有 PulseAudio 时
会回退到 `parec`。音频采集不依赖合成器，所以 Wayland 和 X11 下的
用法相同。

**和 OwnTone 有什么区别？**
OwnTone 是媒体服务器，播放的是你放进曲库的文件。AirFlash Linux
采集的是整个桌面的实时声音——浏览器、播放器、通话和游戏都可以——
而且有桌面面板和 CLI，不用先配置一台服务器。

**延迟怎么样？**
实时模式的目标调度延迟为 120 ms，听音乐和日常使用桌面已经足够。
实际听到的延迟还受接收端和网络影响，应用不会测量这项数据；竞技游戏
这类对延迟特别敏感的场景并不适合。

## 技术栈

- **Rust**——原生 AirPlay 2 引擎：Linux 系统声音采集（PipeWire /
  PulseAudio 监听）、AirPlay 2 会话、HAP 配对、PTP、RTP 与音频传输
- **C# / .NET 10**——共享的 `AirFlash.Core`（会话控制、设置、接收端
  目录、均衡器、本地化）
- **Avalonia**——Linux 图形面板（`desktop/AirFlash.UI`）；Windows 版在
  同一核心上使用 WPF
- **内置 mDNS 查询器**——Linux 上的接收端发现（不依赖 Avahi/Bonjour）
- **PCM 与 ALAC**——通过加密 RTP 发送到 HomePod

## 从源码构建

需要：Rust stable 1.85+、.NET SDK 10.0+（构建面板时）、运行时有
`pw-record`（PipeWire）或 `parec`（PulseAudio）。

```bash
# 无头 CLI + 引擎
cargo build --release --features cli --manifest-path native/airflash-engine/Cargo.toml

# 图形面板
dotnet build desktop/AirFlash.UI/AirFlash.UI.csproj

# AppImage（backend，或 VARIANT=full 构建带面板的版本）
packaging/linux/appimage/build-appimage.sh
```

测试：

```bash
cargo test --features cli --manifest-path native/airflash-engine/Cargo.toml
dotnet test desktop/AirFlash.Tests/AirFlash.Tests.csproj
dotnet test desktop/AirFlash.UI.Tests/AirFlash.UI.Tests.csproj
uv run pytest
```

Linux 构建、打包与 systemd 的完整说明见 [LINUX.md](LINUX.md)（英文）。

## 与上游的关系

AirFlash Linux 是 [Ding-Kyoma/AirFlash](https://github.com/Ding-Kyoma/AirFlash)
（Windows 版）的 fork，并持续跟进上游。共享引擎和核心代码中与平台
无关的改进会双向合并。本仓库主要维护 Linux 打包、Avalonia 面板、
Linux 音频采集和 Linux 发布；Windows 版的安装和使用说明见上游仓库。

## 参与贡献

欢迎提交 PR，尤其是其他发行版、桌面环境和接收端型号的实测报告。
提交时请说明协议行为、配对限制和测量条件，并标注哪些结果来自真实
硬件，哪些只是模拟接收端测试。

## 许可证

[GPLv3+](../LICENSE-GPLv3) 或 [商业授权](../LICENSE-COMMERCIAL.md)。
