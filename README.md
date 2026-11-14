# yoink

A small clipboard history manager for Linux on X11. It keeps text and images
you copy, searches them, and puts an item back on the clipboard when you pick it.
Built for personal use on Linux Mint 22.3 with Cinnamon.

I wanted a smaller alternative to CopyQ. Text, images, search, and a shortcut
owned by the desktop environment are the scope.

![yoink with synthetic clipboard history](docs/screenshot.png)

**Platform:** Linux x86_64 on X11 with XFixes, a Vulkan driver, and a session
D-Bus connection. Ubuntu 24.04 is the oldest tested release. A
StatusNotifierWatcher is needed for the tray. Native Wayland, macOS, and
Windows are not supported.

**Privacy:** clipboard history is stored unencrypted on your computer.
Passwords and tokens can enter history unless the copying application marks
them as secret. Read [Storage and privacy](#storage-and-privacy) before use.

## Install

Install the latest release into `~/.local`:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/garyj/yoink/main/install.sh | sh
~/.local/bin/yoink --version
```

Pass `--version 0.2.0` to install a specific release or `--prefix /absolute/path`
to use another installation prefix:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/garyj/yoink/main/install.sh | \
  sh -s -- --version 0.2.0
```

The installer verifies the release archive against its published SHA-256
checksum. It installs the executable, desktop entry, icon, README, source
commit identifier, and licence notices. To remove those files while keeping
clipboard history and configuration:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/garyj/yoink/main/install.sh | \
  sh -s -- --uninstall
```

Alternatively, use mise's built-in
[GitHub backend](https://mise.jdx.dev/dev-tools/backends/github.html):

```sh
mise use -g github:garyj/yoink@0.2.0
mise exec github:garyj/yoink@0.2.0 -- yoink --version
mise reshim
```

No custom mise plugin is needed. These examples use `garyj/yoink`; forks need
their own repository name. Use a current mise version with the GitHub backend.

On Ubuntu 24.04 and compatible distributions, install the runtime libraries:

```sh
sudo apt install libxkbcommon-x11-0 libvulkan1 mesa-vulkan-drivers
```

`mesa-vulkan-drivers` covers Intel and AMD graphics and software rendering.
The NVIDIA driver brings its own Vulkan driver. yoink also needs a common
desktop font such as Ubuntu, Noto Sans, or DejaVu Sans. The installer and mise
do not install these packages.

You can also download `yoink-v0.2.0-x86_64-unknown-linux-gnu.tar.gz` and
`SHA256SUMS` from the GitHub Release, then extract them in an empty directory:

```sh
sha256sum --check --ignore-missing SHA256SUMS
tar -xzf yoink-v0.2.0-x86_64-unknown-linux-gnu.tar.gz
./yoink --version
```

The expected output is `yoink 0.2.0`. Keep the licence files with the binary.

## Start and summon

Run `yoink` in your graphical session to show the window. If yoink is not
running, this starts it, and it captures the clipboard until you quit it. Run
`yoink` again while the window has focus to hide the window. The tray icon
also shows and hides the window.

In Cinnamon, open **System Settings**, **Keyboard**, **Shortcuts**, then
**Custom Shortcuts**. Bind your chosen key to the full path of the executable.
The installer places it here by default:

```text
/home/YOUR_USER/.local/bin/yoink
```

For a mise installation, use the stable shim path after `mise reshim`:

```text
/home/YOUR_USER/.local/share/mise/shims/yoink
```

Replace `YOUR_USER` with your username. If you set `MISE_DATA_DIR`, use its
`shims/yoink` path instead. Test that exact command from the desktop shortcut.
There is no built-in global shortcut.

To capture clipboard history from login, add the same path with `--hidden` to
your startup applications. `--hidden` starts yoink without showing the window.
Startup entries made for yoink 0.1.0 have no `--hidden`, so add it after an
upgrade, or the window opens at every login.

Quit from the tray menu or run `yoink --quit`. The close button hides the
window without stopping capture. The window stays open when focus moves
elsewhere.

After an upgrade, quit the old process and start yoink again. Installing a
new version does not replace the running process.

## Use the window

The window has a list of history items and a preview of the selected item. It
follows the desktop's light or dark setting.

| Key | Result |
| --- | --- |
| Type | Filters text, ignoring case and accents |
| Up, Down, Page Up, Page Down | Moves the selection |
| Enter, or a double-click | Copies the selected item and hides the window |
| Ctrl+Enter, or Ctrl+double-click | Copies the selected item and keeps the window open |
| Shift+Delete | Deletes the selected item |
| Ctrl+Shift+Delete, or Delete all | Asks to delete all history |
| Shift+Enter, or the red button | Confirms deleting all history. Enter alone never does |
| Escape | Cancels a pending delete, then clears the search, then hides the window |

The search field always has focus, so Ctrl+C copies selected search text.

Copying an existing item moves it to the top without a duplicate. Images
appear when the search is empty. Text previews are shortened, but copying
uses the full stored text. Read, copy, and deletion failures appear in the
window. A clipboard write must succeed before a successful copy hides it.

## Configure

The optional config file is `~/.config/dev.garyj.yoink/config.toml`:

```toml
max_items = 500
```

Restart yoink to apply changes. Lowering the limit removes the oldest items
beyond it at startup. A limit of zero stores nothing and removes existing
history. Invalid values or unknown keys prevent startup and print an error
to stderr.

`XDG_CONFIG_HOME` and `XDG_DATA_HOME` override the corresponding default
directories. The `dev.garyj.yoink` identifier stays fixed so upgrades retain
existing history.

## Storage and privacy

History lives in `~/.local/share/dev.garyj.yoink/history.sqlite`. The app
sets its data directory to mode `0700` and the database to `0600`, including
existing installations. It does not upload clipboard contents or add telemetry.

yoink skips clipboard data marked `x-kde-passwordManagerHint=secret`, which
some password managers provide. An application that omits the marker can
still put sensitive content in history. Clearing the system clipboard later
does not remove an item already saved in yoink.

Deletion overwrites deleted SQLite content with secure deletion enabled.
Clear all also compacts the database to remove previously freed pages.
This cannot erase copies in backups, filesystem snapshots, swap, or storage
hardware. **Clearing history does not clear the system clipboard.**

Quit yoink before manually replacing or removing the database file. A
database from a newer, unsupported schema is rejected without rewriting it.

Capture limits are 1 MiB of UTF-8 text, 64 MiB of decoded RGBA pixels,
8192 pixels per image dimension, and 10 MiB per encoded PNG. Oversized
content is skipped and logged to stderr. arboard must receive clipboard data
before these limits can be checked, so the limits are not a hard bound on
memory used during the initial clipboard transfer. Data is fetched on X11
ownership changes, rather than repeatedly decoding an unchanged clipboard.

## Build from source

Install Rust through [rustup](https://rustup.rs), which reads the pinned
`rust-toolchain.toml`. The tested toolchain is Rust 1.98.0.

On Ubuntu or Debian derivatives, install the build libraries, then build:

```sh
sudo apt install build-essential pkg-config libxkbcommon-x11-dev libfontconfig-dev
cargo build --locked --release -p yoink
```

The executable is `target/release/yoink`. `just build` runs the same build.

For development, run `cargo run -p yoink`. Development uses the same data
directory unless you set different XDG directories before starting the
process.

`just package` also needs `cargo-about`. Run `mise install` in this repository
to install the pinned version.

## Develop and test

`crates/yoink-core` owns SQLite persistence, search, retention, migrations,
and config parsing. It has no GUI dependencies. `crates/yoink-clipboard` owns
clipboard access and X11 change tracking. `crates/yoink` is the GPUI app: the
window, the tray, and the socket that hands a summon to the running instance.

Run the configured checks:

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

With `xvfb` and `xclip` installed, run the clipboard integration tests on their own X server:

```sh
xvfb-run -a cargo test --locked --workspace x11_ -- --ignored --test-threads=1
```

The integration test changes its display's clipboard, so do not run it
against your normal desktop. On machines without the GUI development
libraries, `cargo test --locked -p yoink-core` runs the storage tests alone.

`just check` wraps formatting, Clippy, and tests. A GPUI test drives search,
copy, and delete from the keyboard without a display. Exercise changed UI
behavior in the real app before release.

See [dependency review notes](docs/dependencies.md) for known upstream
advisories and [the release guide](docs/releasing.md) for packaging and
publishing. Read [AGENTS.md](AGENTS.md) before changing clipboard ownership
or window focus behavior.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md). The licence
choice conservatively covers potential CopyQ adaptations identified by the
existing source notes. Studying CopyQ's behavior alone does not require GPL.
