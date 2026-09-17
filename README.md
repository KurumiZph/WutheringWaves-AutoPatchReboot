# WutheringWaves-AutoPatchReboot

Tired of manually restarting Wuthering Waves after a patch, or making a coffee only to come back to the damn restart prompt and now you have to wait more?

This app watches the game window for you and automatically restarts it when
it sees:

> Patching complete. Please restart the game

It keeps doing that (usualy only one cycle) until it detects:

> Tap to land in Solaris-3

...at which point it stops and leaves the game running.

Building it from source or want to contribute? See
[Development](DEVELOPMENT.md)

## Table of Contents

- [Requirements](#requirements)
- [Download & Run](#download--run)
- [First Run](#first-run)
- [Using the Tray Icon](#using-the-tray-icon)
- [Where Your Data Lives](#where-your-data-lives)
- [Troubleshooting](#troubleshooting)
- [Limitations](#limitations)
- [Disclaimer](#disclaimer)
- [AI Assistance](#ai-assistance)
- [License](#license)

## Requirements

- Windows 10/11
- [Tesseract-OCR](https://github.com/UB-Mannheim/tesseract/wiki), the app
  will prompt you to install this if it's missing, see below
- Wuthering Waves, installed via Steam or the official launcher

Python and any Python packages are bundled inside the `.exe`.

## Download & Run

Grab the latest archive from the [Releases page](../../releases) and
extract it anywhere you like, for example `Documents\WuWa AutoPatchReboot\`.
Avoid extracting it into the Wuthering Waves install folder itself, since
game patches may touch that directory.

You'll get:

```
WuWa-AutoPatchReboot\
├── wuwa-apr.exe        <- run this
└── _internal\
```

Keep `wuwa-apr.exe` and `_internal\` together in the same folder. Then make
a shortcut to `wuwa-apr.exe` (right-click > Send to > Desktop) and use that
shortcut in place of your usual Wuthering Waves shortcut. No installer, no
setup wizard.

## First Run

1. **A UAC ("Do you want to allow this app...") prompt appears.** Click
   Yes. The app needs admin rights to reliably capture the game window.
   This happens on every launch, not just the first.

2. **If Tesseract-OCR isn't installed**, a prompt opens leading to the download page.
   Install it normally (default install location recommended), then click OK back in the app to continue.

3. **It looks for your Wuthering Waves install automatically** checking
   Steam, the Windows registry, and a few common install folders. If it
   genuinely can't find it, a file picker opens and asks you to browse to
   it yourself. Whatever it finds (automatically or by hand) is remembered
   for next time, so this only happens once.

4. The application minimizes to the system tray and the game launches with the watchdog active.

## Using the Tray Icon

Right-click the tray icon for:

- **Status line**: shows what it's currently doing | (waiting for the window, monitoring, restarting, etc.)
- **Show Live Log**: opens a window with real-time log output.
- **Show Debug Screenshot**: opens the last screenshot it OCR'd, useful for debugging.
- **Open Data Folder**: opens the log/config folder.
- **Quit**: stops the watchdog. This does **not** close Wuthering Waves.

The app closes itself automatically once it detects the login screen, usually you don't need to quit it manually.

## Where Your Data Lives

Everything the app writes (log file, last debug screenshot, and configured game path) is stored in:

```
%APPDATA%\WuWaWatchdog\
```

Nothing is written next to the `.exe`, so you can move or re-extract the
app folder freely without losing your settings.

## Troubleshooting

- **Detection seems wrong / restarts happening when they shouldn't:**
  check tray > Show Debug Screenshot and Show Live Log to see what it's
  actually reading off your screen.
- **It picked the wrong exe / can't find the game:** delete
  `%APPDATA%\WuWaWatchdog\wuwa_watchdog_config.json` and relaunch to force
  it to re-detect (or re-browse).
- **The app won't start, or complains about missing files:** make sure you
  extracted the whole archive and that `_internal\` is still sitting next
  to `wuwa-apr.exe`. Moving the exe out on its own won't work.
- **Don't share debug logs/screenshots publicly.** They might contain
  private information.

## Limitations

This project relies on OCR and the current Wuthering Waves UI text.
Future game updates may change wording or layout and break detection.

OCR accuracy can also vary with resolution and display scaling.

The app works if the game is not in focus but not when the game window is minimized.

## Disclaimer

This is an unofficial community project and is not affiliated with Kuro
Games or Wuthering Waves. Use it at your own risk and make sure you're
following the game's Terms of Service.

## AI Assistance

Parts of this project's implementation were developed with AI assistance.
The project is maintained and tested by the repository owner.

## License

MIT License
