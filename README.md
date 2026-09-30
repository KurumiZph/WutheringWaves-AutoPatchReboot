### < WIP: Resource tier resolving and launch fix >
Update 3.7 introduced resource tiers and game requires dynamic arguments to launch, current launch logic needs a rework.
**Error: Fatal error: [File:Unknown] [Line:54] kuro: Use launcher to start the game!**

# WutheringWaves-AutoPatchReboot

A small Windows watchdog for Wuthering Waves that uses OCR to detect the
patch/restart screen and automatically restarts the game.

It stops when it detects the login screen:

> Tap to land in Solaris-3

Building from source or contributing? See [DEVELOPMENT.md](DEVELOPMENT.md).

## Requirements

- Windows 10/11
- [Tesseract-OCR](https://github.com/UB-Mannheim/tesseract/wiki)
- Wuthering Waves, installed through Steam or the official launcher

Python dependencies are bundled with the packaged executable.

## Download & Run

Download the latest release from the [Releases page](../../releases) and
extract the archive somewhere convenient.

A normal `--onedir` release contains:

```text
WuWa-AutoPatchReboot\
├── wuwa-apr.exe
└── _internal\
```

Keep the executable and `_internal\` together.

## First Run

1. The application requests administrator permission. Click **Yes**.
2. If Tesseract-OCR is missing, install it when prompted.
3. The application searches for the Wuthering Waves executable automatically.
   If it cannot find it, select it manually. The selected path is remembered.
4. The watchdog minimizes to the system tray and starts monitoring the game.

## Tray Menu

Right-click the tray icon to access:

- **Status** — current watchdog state.
- **Show Live Log** — opens the live application log when you want to inspect
  what the watchdog is doing.
- **Show Debug Screenshot** — opens the most recent OCR screenshot when you
  want to check what the watchdog is seeing.
- **Open Data Folder** — opens the application's data directory.
- **Quit** — stops the watchdog without closing Wuthering Waves.

The application normally runs quietly in the tray. Diagnostic options are
available when needed, so there is no separate Debug version for users.

The watchdog normally exits automatically after it has handled the patch/restart
cycle, detected the login screen, or detected an already-running User ID.

## Data Location

Runtime files are stored in:

```text
%APPDATA%\WuWaWatchdog\
```

Typical files include:

```text
ocr_log.txt
ocr_debug.png
wuwa_watchdog_config.json
```

The debug log and screenshot may contain game/account information. Review them
before sharing.

## Troubleshooting

### Detection is not working

Open **Show Live Log** and **Show Debug Screenshot** from the tray.

If the screenshot itself is incomplete or cropped, the problem is with window
capture rather than Tesseract recognition.

### Wrong Wuthering Waves executable

Delete:

```text
%APPDATA%\WuWaWatchdog\wuwa_watchdog_config.json
```

and launch again to select the game executable.

### Missing files / application will not start

Make sure the entire `--onedir` release was extracted and that `_internal\`
is beside the executable.

### Tesseract errors

Make sure Tesseract-OCR is installed. The application checks common installation
locations and `PATH`.

## Limitations

- Detection depends on the current Wuthering Waves UI text and layout.
- Game updates may change the UI and require detection changes.
- OCR accuracy can vary with rendering, resolution, and display scaling.
- Minimized game windows are not expected to capture reliably.

## Disclaimer

This is an unofficial community project and is not affiliated with Kuro Games or
Wuthering Waves. Use it at your own risk and make sure you follow the game's
Terms of Service.

## AI Assistance

Parts of the implementation were developed with AI assistance. The project is
maintained and tested by the repository owner.

## License

MIT License
