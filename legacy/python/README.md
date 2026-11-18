# WutheringWaves-AutoPatchReboot

A small Windows app for Wuthering Waves that uses OCR to detect the
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

Download the latest release from the [Releases page](../../releases) and extract the archive somewhere convenient.

A normal `--onedir` release contains:

```text
WuWa-AutoPatchReboot\
├── wuwa-apr.exe
└── _internal\
```

> [!IMPORTANT]
> Keep `wuwa-apr.exe` and `_internal\` together.

## Tray Menu

Right-click the tray icon to access:

- **Status:** current app state.
- **Show Live Log:** opens the live application log when you want to inspect
  what the app is doing.
- **Show Debug Screenshot:** opens the most recent OCR screenshot / what the app is seeing.
- **Open Data Folder:** opens the application's config directory.
- **Quit:** stops the app **without** closing the game.

The app exits automatically after it detects the login screen, or User ID.

## Data Location

Runtime files are stored in: 

```text
%APPDATA%\WuWaWatchdog\
├── ocr_debug.png
├── ocr_log_full.txt
├── ocr_log_min.txt
└── wuwa_watchdog_config.json
```

The config stores discovered installations, last selected executable, and the **'Remember this selection'** preference.

Runtime logs are overwritten on future launch.
Review the log and screenshot before sharing them.

## Troubleshooting

### Detection is not working

- Open **Show Live Log** and **Show Debug Screenshot** from the tray.

  - If the screenshot itself is incomplete or cropped, the problem is with windows scaling, try changing it.

### Wrong game installation selected

- If **'Remember this selection'** is enabled, the last-used executable is reused on
subsequent runs. To reset this, delete:

```text
%APPDATA%\WuWaWatchdog\wuwa_watchdog_config.json
```

### Missing files / application will not start
- Be sure to keep `wuwa-apr.exe` and `_internal\` together in one directory.

### Tesseract errors
- Make sure **Tesseract-OCR** is installed. The application checks common installation locations and `PATH`.

## Limitations

- Detection depends on the current Wuthering Waves UI text and layout.
- Game updates may change the UI or launch behaviour thus requiring changes.
- OCR accuracy can vary with rendering, resolution, and display scaling.
- **Minimized** game windows are not expected to capture reliably.

## Disclaimer

This is an unofficial community project and is not affiliated with Kuro Games or
Wuthering Waves. Use it at your own risk.

## AI Assistance

Parts of the implementation were developed with AI assistance. The project is
maintained and tested by the repository owner.

## License

This project is licensed under the [MIT License](LICENSE) - see the [LICENSE](LICENSE) file for details.

