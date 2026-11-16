# Installing Lockin

Lockin is distributed as a single self-contained executable per platform. No
installer, admin rights, account or network connection is required.

Download the latest files from
[GitHub Releases](https://github.com/appuzlotatheog/lockin-final-hope/releases).

| Platform | Asset |
| --- | --- |
| Linux x86_64 | `lockin-final-hope-linux-x86_64` |
| Windows x86_64 | `lockin-final-hope-windows-x86_64.exe` |

## Linux

### Run without installing

```sh
chmod +x lockin-final-hope-linux-x86_64
./lockin-final-hope-linux-x86_64
```

### Install for the current user

This installs the binary to `~/.local/bin` and adds it to your application
launcher. It does not use `sudo` and does not modify system directories.

```sh
chmod +x lockin-final-hope-linux-x86_64
mkdir -p ~/.local/bin ~/.local/share/applications
install -m 755 lockin-final-hope-linux-x86_64 ~/.local/bin/lockin-final-hope

cat > ~/.local/share/applications/lockin-final-hope.desktop <<EOF
[Desktop Entry]
Type=Application
Name=Lockin — study OS
Comment=Local-first study and exam OS
Exec=$HOME/.local/bin/lockin-final-hope
Icon=applications-education
Terminal=false
Categories=Education;
StartupNotify=true
EOF

update-desktop-database ~/.local/share/applications 2>/dev/null || true
```

If `~/.local/bin` is not on your `PATH`, add this to your shell profile:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Then launch **Lockin — study OS** from your app menu, or run:

```sh
lockin-final-hope
```

### Uninstall

```sh
rm -f ~/.local/bin/lockin-final-hope
rm -f ~/.local/share/applications/lockin-final-hope.desktop
```

Your data is kept in `~/.local/share/lockin-final-hope/`. Delete that folder
only if you also want to remove all plans, sessions, revision history and
settings.

## Windows

1. Download `lockin-final-hope-windows-x86_64.exe`.
2. Put it anywhere you like, for example `C:\Users\<you>\Apps\Lockin\`.
3. Double-click the executable.

Windows SmartScreen may warn because the executable is not code-signed. Click
**More info** and then **Run anyway**. This is expected for an unsigned
open-source build.

To create a Start Menu shortcut, right-click the executable and choose
**Pin to Start**, or create a shortcut in
`%APPDATA%\Microsoft\Windows\Start Menu\Programs`.

Data is stored in `%APPDATA%\lockin-final-hope\` (or the equivalent
`directories` data directory on your account).

## Build from source

Requirements:

- Rust stable 1.92 or newer
- A C toolchain (for the bundled SQLite build)
- Linux only: `fontconfig`, `libxkbcommon`, and Wayland/X11 development headers

Linux (Debian/Ubuntu):

```sh
sudo apt-get install -y build-essential pkg-config libfontconfig1-dev \
  libxkbcommon-dev libwayland-dev
```

Then:

```sh
git clone https://github.com/appuzlotatheog/lockin-final-hope.git
cd lockin-final-hope
cargo test
cargo build --release
```

Output binaries:

- Linux: `target/release/lockin-final-hope`
- Windows: `target\release\lockin-final-hope.exe`

## Verification

The release workflow runs on every `v*` tag and builds both platforms:

- Linux: `x86_64-unknown-linux-gnu`
- Windows: `x86_64-pc-windows-msvc`

Source, schema and example plan are in the repository root.
