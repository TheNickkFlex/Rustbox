# Rustbox

A fluxbox-based window manager written entirely in **Rust** (X11).

## Beta Stable Preview

![Rustbox in action](screenshotbeta.png)

## What is Rustbox?

Rustbox is a **pure-Rust, X11 window manager** that replaces the original
fluxbox C++ codebase piece by piece. It manages windows, workspaces, a
toolbar, a slit for dock apps, and a system tray — plus a built-in,
dunst-compatible notification daemon so you don't need a separate daemon
running.

- **No autotools, no make, no gcc.** Just `cargo build`.
- **No cairo, no pango.** Text is rendered with `fontdb` + `ab_glyph`
  (anti-aliased TrueType) plus emoji support (`skrifa` / `ttf-parser`).
- **EWMH-compliant.** Clients like kitty, alacritty, and Firefox get
  proper `_NET_WORKAREA` and `_NET_SUPPORTED` hints.
- **Built-in notifications.** `org.freedesktop.Notifications` + dunstctl,
  rules, markup, icons, progress bars, stack tags — all inside the WM.

## Resource usage

Average footprint of the release build measured **idle** (no managed windows)
on a virtual X server (Xephyr, 1024×768). VRAM is the X server-side footprint
(windows, pixmaps, GCs) — read from the X server process, since all server
resources live there.

| Resource | Average (idle) | Notes                                                                             |
|----------|----------------|-----------------------------------------------------------------------------------|
| RAM      | ~29 MB (RSS)   | Stable; no leak over 10 s. Dominated by the font DB + emoji font loaded at startup. |
| CPU      | ~0.6 %         | Event loop blocks while idle; essentially idle.                                   |
| VRAM     | ~0.15 MB       | Root + toolbar + tray windows and GCs. Grows only with notifications (≈4 KB icon pixmap each) and managed window frames. |

## Features

| Area              | What's working                                              |
|-------------------|-------------------------------------------------------------|
| Window management | create, destroy, move, resize, focus, minimize, maximize, fullscreen, shade |
| Workspaces        | multiple virtual desktops with rename                       |
| Toolbar           | clock, workspace list, window taskbar                       |
| Slit              | dock-app container (Window Maker / bbtools style)           |
| System tray       | `_NET_SYSTEM_TRAY_S0` with overflow popup + chevron         |
| Root menu         | right-click menu with workspace navigation, run dialog      |
| Keybindings       | configurable via `~/.config/rustbox/keys`                   |
| RandR             | screen resize / multi-monitor reflow                        |
| EWMH              | `_NET_WORKAREA`, `_NET_SUPPORTED`, etc.                     |
| Notifications     | dunst-compatible, full-featured (see below)                 |
| Font system       | TrueType, anti-aliased, emoji, bitmap fallback              |
| D-Bus tray        | StatusNotifierItem (modern tray apps via `zbus` + `smol`)   |

### Notification daemon (built-in)

- **Interface**: `org.freedesktop.Notifications`
- **dunstctl**: `org.dunstproject.cmd0` (ping, pause/resume, close,
  closeAll, history, context, closeLast, clearHistory, removeFromHistory,
  popHistory, historyCount, configReload)
- **Rules**: regex-based (appname, summary, body, urgency, category,
  stack_tag, desktop_entry); override timeout/urgency/icon, skip_display,
  set_transient, history_ignore
- **Config**: dunstrc-like INI at `~/.config/rustbox/notifications.conf`
- **Markup**: `<b>`, `<i>`, `<u>`, `<a>`, `<img>` stripping
- **Icons**: `image-path`, `image-data` (RGBA8), desktop-entry →
  theme icon lookup across freedesktop directories
- **Progress bar**: hint `value` rendered in the popup
- **Stack tags**: same-tag notifications replace each other in-place

## Building

Rustbox is a pure Rust project. You only need the Rust toolchain and the
X11 client libraries — **no gcc, make, autotools or C compiler are
required**.

### Prerequisites

- X11 development libraries
- Git (only to clone)

#### Install Rust

Pick either your system package manager or the official rustup installer:

**Via distro repositories (simplest):**

| Distro        | Command                             |
|---------------|--------------------------------------|
| Debian/Ubuntu | `sudo apt-get install rustc cargo`  |
| Arch Linux    | `sudo pacman -S rust`               |
| Fedora/RHEL   | `sudo dnf install rust cargo`       |
| Alpine        | `sudo apk add rust cargo`           |

**Via rustup (official, works everywhere):**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Make sure `cargo` is on your `PATH` (`source "$HOME/.cargo/env"` or
log out/in).

#### Install X11 dev libraries

| Distro        | Command                                                                                                       |
|---------------|---------------------------------------------------------------------------------------------------------------|
| Debian/Ubuntu | `sudo apt-get install libx11-dev libxinerama-dev libxrandr-dev libxext-dev libxft-dev libxdamage-dev`          |
| Arch Linux    | `sudo pacman -S libx11 libxinerama libxrandr libxext libxft libxdamage`                                       |
| Fedora/RHEL   | `sudo dnf install libX11-devel libXinerama-devel libXrandr-devel libXext-devel libXft-devel libXdamage-devel`  |

### Build & install

```bash
git clone https://github.com/TheNickkFlex/Rustbox.git
cd Rustbox

# Debug build
cargo build

# Release build (optimized, recommended for daily use)
cargo build --release

# Install to ~/.cargo/bin (optional)
cargo install --path .
```

## Running

Rustbox is a window manager — it replaces your current WM on a given X
display:

```bash
# From the build directory
./target/release/rustbox

# Or, if installed via cargo
rustbox
```

Rustbox uses `$DISPLAY` by default. You can override with `-display :1`.
The `-socket` flag also works for direct X socket paths.

The root menu launches **kitty** as the default terminal emulator. Make sure
it is installed.

## Configuration

| File                                   | Purpose                                             |
|----------------------------------------|------------------------------------------------------|
| `~/.config/rustbox/keys`               | Keybindings                                         |
| `~/.config/rustbox/workspaces.conf`    | Workspace names                                     |
| `~/.config/rustbox/notifications.conf` | Notification rules and theming (dunstrc-like INI)    |

## Dependencies (Rust crates)

All Rust dependencies are fetched and pinned by Cargo — you never install
them manually.

- **X11**: [`x11rb`](https://crates.io/crates/x11rb) (pure-Rust protocol)
- **Fonts**: `fontdb` (discovery), `ab_glyph` (rasterization),
  `ttf-parser` / `skrifa` (emoji, COLRv1)
- **Images**: [`image`](https://crates.io/crates/image) crate (PNG, XPM, etc.)
- **D-Bus**: `dbus` (notifications), `zbus` + `smol` (SNI tray)
- **Other**: `regex`, `serde`, `anyhow`, `env_logger`, `glob-match`,
  `signal-hook`, `libc`

## License

Rustbox is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Credits

- **[fluxbox](https://github.com/fluxbox/fluxbox)** — *Recreated in Rust*
- **[dunst](https://github.com/dunst-project/dunst)** — *Recreated in Rust*
- **[zbus](https://github.com/z-galaxy/zbus)** — *Made in Rust*
