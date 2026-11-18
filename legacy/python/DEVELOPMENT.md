# Development

Development and build notes for the current WutheringWaves-AutoPatchReboot source.

For normal installation and usage, see [README.md](README.md).

## Requirements

- Windows 10/11
- Python 3.13+
- Tesseract-OCR
- Git
- PyInstaller
- 7-Zip, if creating release archives

## Setup

```powershell
git clone https://github.com/KurumiZph/WuWa-AutoPatchReboot.git
cd WuWa-AutoPatchReboot
python -m pip install -r requirements.txt
```

Run the source with:

```powershell
python wuwa-apr.py
```

Tesseract must be installed on the development machine. The source contains
dependency/elevation handling, but a frozen PyInstaller build relies on the
dependencies available at build time.

## Project Files

```text
WuWa-AutoPatchReboot/
├── wuwa-apr-new.py
├── requirements.txt
├── icon.png
├── wuwa.ico
├── README.md
├── DEVELOPMENT.md
├── LICENSE
└── .gitignore
```

A PyInstaller `--onedir` build contains the executable plus `_internal\`; both are required.

## Current Configuration

Important runtime settings currently include:

```python
RESTART_WAIT_SECONDS = 15            # Time to wait after detecting patch notice (seconds).
CHECK_INTERVAL_SECONDS = 1.0         # Time to wait before performing the next OCR (seconds).

LOGIN_CONFIRMATIONS_REQUIRED = 2     # Number of time the login prompt needs to be detected before exiting.
PATCH_CONFIRMATIONS_REQUIRED = 2     # Number of time the patch restart prompt needs to be detected before restarting.

LOGIN_MATCH_THRESHOLD = 0.72         # Higher value = better matching. Too high or low values may not trigger reliably.
PATCH_MATCH_THRESHOLD = 0.68         # ^

PATCH_CENTER_X_RANGE = (0.15, 0.85)  # [X min, X max] bounding range to detect for patch notice. (range 0 to 1 as a fraction of screen)
PATCH_CENTER_Y_RANGE = (0.20, 0.80)  # [Y min, Y max] bounding range to detect for patch notice. (range 0 to 1 as a fraction of screen)

MAX_OCR_DIMENSION = 2200             # Screenshots downsampling resolution

DEBUG = True                         # If True, dumps all the info in logs.
DEBUG_SCREENSHOT_EVERY_N = 3         # Time to wait before performing next screen capture (seconds).
```

Current OCR targets:

```python
LOGIN_TARGET = "tap to land in solaris 3"
PATCH_TARGETS = [
    "patching complete",
    "please restart the game",
]
```

Patch detection requires both patch phrases and checks their approximate screen
position to reduce false positives.

## Building the Application

There is one normal packaged build. Diagnostic features are part of the same
application rather than a separate Debug executable. `DEBUG = True` remains enabled for the current build becausee performance and storage impact is minimal.

Build from a clean tree:

```powershell
pyinstaller --clean --onefile --windowed --icon=wuwa.ico --uac-admin --add-data "icon.png;." wuwa-apr.py
```

### PyInstaller Notes

- `--onedir` keeps the executable and `_internal\` together.
- `--windowed` prevents a console window for the packaged tray application.
- `--uac-admin` adds the administrator manifest used by the current packaged
  build.
- `--add-data "icon.png;."` includes the tray icon required by the application.
- `--clean` removes previous PyInstaller build artifacts before packaging.

If the elevation mechanism or resource loading changes, re-check whether the
current PyInstaller options are still necessary.

The build result should be:

```text
dist\wuwa-apr\
├── wuwa-apr.exe
└── _internal\
```

The distributed application normally runs quietly in the tray. Users can open
**Show Live Log** or **Show Debug Screenshot** when they need diagnostics.

## Testing

### Syntax

```powershell
python -c "import ast, pathlib; ast.parse(pathlib.Path('wuwa-apr-new.py').read_text(encoding='utf-8')); print('syntax OK')"
```

### Functional states

Test:

1. Patch/restart screen.
2. Login screen.
3. Already-logged-in game with visible User ID.
4. Game restart after confirmed patch detection.
5. Tray Quit.
6. Multi-installation discovery and installation selection.
7. Remembered installation selection enabled and disabled.
8. Resource tier resolving for SD/HD/UHD installations.
10. Tesseract detection/error handling.
11. Live Log and Debug Screenshot tray actions.

### DPI regression

Test at least:

- 1920x1080 @ 100% | 125% | 150%
- 1680x1050 @ 100% | 125% | 150%
- 1366x768  @ 100% | 125% | 150%
- 1280x720  @ 100% | 125% | 150%

If available, also test higher resolutions and scaling levels.

For diagnostics, inspect:

```text
%APPDATA%\WuWaWatchdog\ocr_debug.png
```

The screenshot should contain the complete WuWa client.

## Capture and OCR

The capture path is DPI-aware and initializes Windows DPI awareness once:

```python
ctypes.windll.shcore.SetProcessDpiAwareness(2)
```

with fallback:

```python
ctypes.windll.user32.SetProcessDPIAware()
```

Keep only one DPI-awareness initialization block.
The current window capture uses:

```python
PrintWindow(..., 2)
```

`2` is `PW_RENDERFULLCONTENT`
`3` is `PW_RENDERFULLCONTENT | PW_CLIENTONLY` (might have high-DPI capture problem)

The intended processing order is:

```text
physical window capture
        ↓
complete full-resolution image
        ↓
OCR preprocessing
        ↓
downscale when needed
        ↓
Tesseract
        ↓
login / patch / UID detection
```

`MAX_OCR_DIMENSION` limits OCR processing size; it should not limit the initial
window capture.

## UID Handling

The app looks for a likely User ID token following an `id`, `1d`, or `ld`
label.

When recognized:

- the UID is censored in OCR/log output;
- the corresponding region is blacked out in the diagnostic screenshot;
- the app treats the game as already running and exits without restarting.

Diagnostic screenshots and logs should still be treated as potentially sensitive.

## Game Detection and Restart

The application:

1. Finds Wuthering Waves processes by known process names.
2. Finds visible top-level windows belonging to those processes.
3. Selects the largest matching window.
4. Captures the client area.
5. Runs OCR.
6. Requires consecutive detections before acting.
7. On confirmed patch detection, terminates the matching game process(es), waits,
   resolves the installed resource tier, and launches the selected executable again.

Installation selections, the last-used game path, and the remember-selection preference are stored in:

```text
%APPDATA%\WuWaapp\wuwa_watchdog_config.json
```

## Packaging Checklist

Before publishing a release:

1. Run the syntax check.
2. Build the application with `--clean`.
3. Test the packaged executable, not only the Python source.
4. Test patch, login, and existing-UID behavior.
5. Test at least one display-scaling configuration above 100%.
6. Verify Live Log and Debug Screenshot.
7. Verify tray Quit.
8. Verify multi-installation detection and installation selection.
9. Verify remembered selection both enabled and disabled.
10. Verify resource tier resolving.
11. Verify log cleanup and versioning.
12. Archive the complete release directory, including `_internal\`.

Example:

```powershell
7z a WuWa-AutoPatchReboot-vX.Y.Z.7z .\dist\wuwa-apr\*
```

Do not distribute only the `.exe` from an `--onedir` build.