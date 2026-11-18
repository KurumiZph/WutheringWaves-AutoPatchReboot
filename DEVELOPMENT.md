# Build it yourself

Everything you need to compile WuWa APR on your own PC. Needs **Rust 1.79 or newer** (rustup gives you that).

## 1. Set up

### Installers:
**Windows 10 / 11 only.** Install these once:

1. **C++ build tools**: needed by Rust to link on Windows.
   Get [Build Tools for Visual Studio](https://visualstudio.microsoft.com/visual-cpp-build-tools/) and tick
   **Desktop development with C++**.
2. **Rust**: install from [rustup.rs](https://rustup.rs) (the default options are fine).
3. **Git**: from [git-scm.com](https://git-scm.com).

**or**

### Winget:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install Rustlang.Rustup
winget install Git.Git
```

## 2. Get the code

```powershell
git clone https://github.com/KurumiZph/WuWa-AutoPatchReboot.git
cd WuWa-AutoPatchReboot
```

## 3. Build

```powershell
.\build.ps1
```

Your exe is at `dist\wuwa-apr.exe`. The first build takes a few minutes while the dependencies compile; after that
it's fast.

Prefer plain Cargo? `cargo build --release` gives you `target\release\wuwa-apr.exe`.

## Run and test

APR requires admin rights, so `cargo run` only works from a terminal opened **as administrator**.

```powershell
cargo test          # unit tests
```

## Something odd?

- **`link.exe not found`**: the C++ build tools aren't installed, or the terminal was opened before you installed
  them. Reinstall step 1 and open a new terminal.
- **Stale build / wrong icon**: `cargo clean`, delete `dist\`, run `.\build.ps1` again.
- **Can't overwrite `wuwa-apr.exe`**: quit APR from the tray first.

## Project layout

Everything is in `src/`. The UI is plain Win32 + GDI (no UI framework).

| File | What it does |
| --- | --- |
| `main.rs` | Entry point. APR runs on its own thread; the main thread owns the tray icon and windows. |
| `monitor.rs` | APR itself: launch the game, scan with OCR, restart after a patch. |
| `detect.rs`, `textmatch.rs`, `cadence.rs` | Login / patch / UID detection, fuzzy text matching, and scan pacing (pure logic, unit-tested). |
| `ocr.rs`, `capture.rs`, `image_ops.rs` | Windows OCR, grabbing the game window, pixel helpers. |
| `game.rs`, `installs.rs`, `autologin.rs` | Game process handling, finding Steam / launcher installs, Auto Login input. |
| `languages.rs` | Game languages and `languages.json`. |
| `settings.rs`, `config.rs` | Presets and values (`settings.json`), paths and `config.json`. |
| `log.rs`, `logstyle.rs`, `shots.rs` | Logs, log colouring, saved screenshots. |
| `ui.rs`, `appwin.rs`, `gfx.rs`, `ui_types.rs` | Tray, main window, GDI drawing helpers. |
| `tab_logs.rs`, `tab_settings.rs`, `tab_misc.rs` | The three tabs. |
| `overlay.rs` | The "Patch detected" notification box. |
| `selftest.rs`, `sys.rs`, `state.rs` | Test tools, OS helpers, shared flags. |

## Icons

- `assets/icon.ico` is the **exe icon** (embedded by `build.rs`).
- `assets/tray.ico` is compiled into the program and used for the tray, the window icon (Alt+Tab) and the header logo.

Use a multi-size `.ico` (16 to 256 px). Windows caches exe icons, so if one looks stale, copy or rename the exe.

## Adding or fixing a language

Texts per language live in `languages.json` in `%APPDATA%\WuWa-APR\` (open it with **Misc > Languages file**, apply
with **Misc > Reload languages**). Each language has a name, an OCR language, login text, UID labels and the patch
notice. Leave the patch notice empty (`[]`) if you don't have it yet; patch detection is then off for that language.
To make it permanent for everyone, add it to the built-in list in `src/languages.rs` and open a pull request.

Test it with **Misc > Test an image** on a screenshot, or **Run self-test**, without waiting for a real patch.

## Contributing

Pull requests and issues are welcome. Run `cargo test` first. For a detection problem, include a log and, if you
can, a screenshot (turn on *Save screenshots* in Settings > Debugging; your UID is masked).
