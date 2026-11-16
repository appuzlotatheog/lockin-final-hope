# Lockin — study and exam OS

A native, local-first desktop application for planning study time, tracking
syllabus progress, running a focus timer, managing revision, mocks and mistakes,
and analysing real focus data. All data stays in a single SQLite database on
your machine. There is no login, cloud sync, telemetry, web view or AI service.

## Stack

- Rust (stable) + Slint 1.18
- SQLite via `rusqlite` (bundled)
- `chrono` / `chrono-tz`, `serde` / `serde_json`, `rfd`, `directories`,
  `thiserror`, `uuid`, `tracing`, `arboard`

## Build

```sh
cargo check
cargo test
cargo run
cargo build --release
```

The release binary is `target/release/lockin-final-hope`.

## Data location

`~/.local/share/lockin-final-hope/lockin.db` (Linux). Backups are plain JSON
snapshots written wherever you choose.

## Features

- Sidebar pages: Overview, Timetable, Syllabus, Focus, Revision Queue, Exams,
  Analytics, Import / Export, Settings.
- Seeded with a Kerala Plus One Christmas 2026 plan (52 chapters across
  Physics, Chemistry, Mathematics, Computer Science, English; Hindi with empty
  portions; 6 exams; weekday / Saturday / Sunday templates).
- Monotonic focus timer with checkpointing, pause exclusion and crash recovery.
- Revision policy of 1 / 3 / 7 days with Easy / Good / Difficult / Failed
  adaptation; history is never deleted.
- Lockin Plan Format (versioned JSON) import / export / backup with validation,
  merge and replace modes.

## Keyboard

Tab moves between controls; Space / Enter activates buttons, toggles and
selections.

## Limitations

- Visual verification was blocked in this environment by an active screen lock;
  the app was compiled and launched successfully and its window title verified.
- Only the seeded Christmas 2026 plan and wizard-created plans are supported;
  there is no cloud or multi-device sync.
- The example plan file is generated from the seeded database and uses the
  default weekday / Saturday / Sunday templates.

## Installation

See [docs/INSTALL.md](docs/INSTALL.md) for the full guide, including Windows
notes, uninstall steps and data locations. A user-level Linux installer is in
`packaging/install-linux.sh`.

### Linux

Download the latest release asset `lockin-final-hope-linux-x86_64` from GitHub
Releases, make it executable and run it:

```sh
chmod +x lockin-final-hope-linux-x86_64
./lockin-final-hope-linux-x86_64
```

Or install the desktop entry:

```sh
mkdir -p ~/.local/bin ~/.local/share/applications
cp lockin-final-hope-linux-x86_64 ~/.local/bin/lockin-final-hope
chmod +x ~/.local/bin/lockin-final-hope
```

Then create `~/.local/share/applications/lockin-final-hope.desktop` with:

```ini
[Desktop Entry]
Type=Application
Name=Lockin — study OS
Comment=Local-first study and exam OS
Exec=/home/YOURUSER/.local/bin/lockin-final-hope
Icon=applications-education
Terminal=false
Categories=Education;
```

### Windows

Download `lockin-final-hope-windows-x86_64.exe` from GitHub Releases and run it.
Windows SmartScreen may warn because the binary is unsigned; choose "More info"
then "Run anyway".

### Build from source

```sh
git clone https://github.com/appuzlotatheog/lockin-final-hope.git
cd lockin-final-hope
cargo build --release
```

The binary is `target/release/lockin-final-hope` (or `.exe` on Windows).
