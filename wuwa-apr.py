import os
import re
import ctypes
from ctypes import wintypes
import sys
import time
import subprocess
import difflib
import importlib
import atexit
import winreg
import threading
import json
import queue
import shutil
from pathlib import Path

# ---------------- AUTO-INSTALL DEPENDENCIES ----------------
# Checks for required packages and pip-installs anything missing,
# so users don't have to run `pip install -r requirements.txt` by hand.
# NOTE: this only covers Python packages. Tesseract-OCR itself is a
# separate .exe and can't be installed this way -- see TESSERACT_PATH.

REQUIRED_PACKAGES = {
    "PIL": "Pillow>=11.0.0",
    "pytesseract": "pytesseract>=0.3.13",
    "psutil": "psutil>=6.0.0",
    "win32gui": "pywin32>=306",
    "pystray": "pystray>=0.19.5",
}


def _module_importable(module_name):
    try:
        importlib.import_module(module_name)
        return True
    except ImportError:
        return False


def _run_pywin32_postinstall():
    """
    pywin32 needs an extra post-install step (copying its runtime DLLs
    into place) that a plain `pip install pywin32` never runs on its
    own. Without it, `import win32gui` (and similar) fails with
    ModuleNotFoundError even though pip reports a successful install.
    """
    scripts_dir = os.path.join(os.path.dirname(sys.executable), "Scripts")
    postinstall_script = os.path.join(scripts_dir, "pywin32_postinstall.py")

    if not os.path.isfile(postinstall_script):
        return False

    try:
        print("[SETUP] Running pywin32 post-install step...")
        subprocess.check_call([sys.executable, postinstall_script, "-install"])
        return True
    except Exception as e:
        try:
            print(f"[SETUP] pywin32 post-install step failed: {e}")
        except Exception:
            pass
        return False


def ensure_dependencies():
    # A frozen PyInstaller exe already has every dependency bundled
    # inside it -- there's no system Python or pip to call, and
    # sys.executable is the exe itself, not a real interpreter. Only
    # relevant when running the raw .py with a normal Python install.
    if getattr(sys, "frozen", False):
        return

    missing_modules = []
    missing_specs = []
    for module_name, pip_spec in REQUIRED_PACKAGES.items():
        if not _module_importable(module_name):
            missing_modules.append(module_name)
            missing_specs.append(pip_spec)

    if not missing_specs:
        return

    try:
        print(f"[SETUP] Installing missing packages: {', '.join(missing_specs)}")
    except Exception:
        pass
    try:
        subprocess.check_call([sys.executable, "-m", "pip", "install", *missing_specs])
    except subprocess.CalledProcessError as e:
        try:
            print(f"[SETUP] Failed to install dependencies: {e}")
        except Exception:
            pass
        try:
            input("\nPress Enter to exit...")
        except Exception:
            pass
        sys.exit(1)

    if "win32gui" in missing_modules:
        _run_pywin32_postinstall()

    # pywin32 in particular can still need the postinstall step above
    # before its modules are actually importable -- verify instead of
    # letting the real `import win32gui` further down crash with a
    # bare traceback.
    still_missing = [m for m in missing_modules if not _module_importable(m)]
    if still_missing:
        try:
            print(f"[SETUP] Still missing after install: {', '.join(still_missing)}")
            if "win32gui" in still_missing:
                print("Try running these manually, then re-run this script:")
                print(f'  "{sys.executable}" -m pip install --force-reinstall pywin32')
                print(f'  "{sys.executable}" Scripts\\pywin32_postinstall.py -install')
        except Exception:
            pass
        try:
            input("\nPress Enter to exit...")
        except Exception:
            pass
        sys.exit(1)


ensure_dependencies()

import pytesseract
import psutil
import webbrowser
import tkinter as tk
from tkinter import messagebox, filedialog
import pystray

from PIL import Image, ImageDraw, ImageEnhance, ImageFilter

import win32gui
import win32process
import win32ui


def is_admin():
    try:
        return ctypes.windll.shell32.IsUserAnAdmin()
    except Exception:
        return False


if not is_admin():
    ctypes.windll.shell32.ShellExecuteW(
        None, "runas", sys.executable,
        " ".join(f'"{arg}"' for arg in sys.argv), None, 1,
    )
    sys.exit()

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)  # per-monitor aware
except Exception:
    try:
        ctypes.windll.user32.SetProcessDPIAware()
    except Exception:
        pass

# ============================================================
# WUTHERING WAVES OCR WATCHDOG
# Launches WuWa, OCRs the window, detects login/patch screens,
# and auto-restarts the game on patch-complete. No fixed coords.
# ============================================================

# ---------------- WINDOWS DPI AWARENESS ----------------
# Keep all Win32 window geometry in physical pixels. Without this, Windows
# can report logical dimensions (for example 1280x720 for a 1920x1080
# display at 150% scaling), which makes PrintWindow allocate a cropped
# bitmap and leaves OCR looking at only part of the game.
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)  # per-monitor aware
except Exception:
    try:
        ctypes.windll.user32.SetProcessDPIAware()
    except Exception:
        pass


# ---------------- CONFIG ----------------

GAME_EXE = r""
TESSERACT_PATH = r"C:\Program Files\Tesseract-OCR\tesseract.exe"
TESSERACT_DOWNLOAD_URL = "https://github.com/UB-Mannheim/tesseract/wiki"

RESTART_WAIT_SECONDS = 18          # wait after "please restart" before relaunch
CHECK_INTERVAL_SECONDS = 0.75       # seconds between OCR scans

LOGIN_CONFIRMATIONS_REQUIRED = 2   # consecutive hits needed to confirm
PATCH_CONFIRMATIONS_REQUIRED = 2

LOGIN_MATCH_THRESHOLD = 0.72       # fuzzy-match thresholds (1.0 = exact)
PATCH_MATCH_THRESHOLD = 0.69

# Restart popup must appear within this fraction of screen width/height
# (0.0 = left/top edge, 1.0 = right/bottom edge) to count as real, so
# corner text (like the version watermark) can't trigger a false restart.
PATCH_CENTER_X_RANGE = (0.15, 0.85)
PATCH_CENTER_Y_RANGE = (0.20, 0.80)

MAX_OCR_DIMENSION = 2200           # downscale cap so OCR stays fast at 4K

DEBUG = True                       # verbose logging + debug screenshot
DEBUG_SCREENSHOT_EVERY_N = 3       # save debug PNG every Nth scan (perf)

GAME_PROCESS_NAMES = {
    "Wuthering Waves.exe",
    "Client-Win64-Shipping.exe",
    "WutheringWaves.exe",
}

LOGIN_TARGET = "tap to land in solaris 3"
PATCH_TARGETS = ["patching complete", "please restart the game"]

SCRIPT_DIR = Path(__file__).resolve().parent


def get_app_dir():

# Where to keep files that must persist across runs (log, debug screenshot, saved config)

    appdata = os.environ.get("APPDATA") or os.path.expanduser("~")
    app_dir = os.path.join(appdata, "WuWaWatchdog")
    os.makedirs(app_dir, exist_ok=True)
    return app_dir


APP_DIR = get_app_dir()
LOG_FILE = Path(APP_DIR) / "ocr_log.txt"
DEBUG_SCREENSHOT = Path(APP_DIR) / "ocr_debug.png"
CONFIG_FILE = Path(APP_DIR) / "wuwa_watchdog_config.json"

# Optional: drop a PNG next to the script (or bundle it into the exe with
# PyInstaller's --add-data) to use a custom tray icon. Falls back to a
# generated placeholder if it isn't there.

TRAY_ICON_FILENAME = "icon.png"


def resource_path(filename):

# Resolve a bundled READ-ONLY resource (like the tray icon), whether run from source or frozen (where bundled data lands in sys._MEIPASS).

    base = getattr(sys, "_MEIPASS", str(SCRIPT_DIR))
    return os.path.join(base, filename)


def load_config():
    try:
        with open(CONFIG_FILE, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def save_config(data):
    try:
        with open(CONFIG_FILE, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
    except Exception as e:
        log(f"[CONFIG] Could not save config: {e}")


pytesseract.pytesseract.tesseract_cmd = TESSERACT_PATH


# ---------------- SHARED STATE (tray + watchdog thread) ----------------

stop_event = threading.Event()   # set by the tray "Quit" action
_tray_icon = None                # the pystray.Icon, once created
_status_text = "Starting..."     # shown in the tray's status menu item


def set_status(text, also_log=True):
    global _status_text
    _status_text = text
    if also_log:
        log(text)
    if _tray_icon is not None:
        try:
            _tray_icon.title = f"WuWa Watchdog: {text}"
        except Exception:
            pass


# ---------------- LOGGING ----------------
# Keeps one file handle open for the whole run instead of reopening (ocr_log.txt on every single log() call).

_log_handle = None

def _get_log_handle():
    global _log_handle
    if _log_handle is None:
        try:
            _log_handle = open(LOG_FILE, "a", encoding="utf-8", buffering=1)
        except Exception:
            _log_handle = False  # sentinel: open failed, don't retry every call
    return _log_handle

def _close_log_handle():
    global _log_handle
    if _log_handle:
        try:
            _log_handle.close()
        except Exception:
            pass
        _log_handle = None

atexit.register(_close_log_handle)

_log_queue = queue.Queue()  # feeds the live log viewer window

def log(message):
    try:
        print(message)
    except Exception:
        pass  # no console when frozen as a windowed/tray exe
    handle = _get_log_handle()
    if handle:
        try:
            handle.write(message + "\n")
        except Exception:
            pass
    _log_queue.put(message)


def safe_print(*args, **kwargs):
    """print() that can't crash a console-less frozen exe."""
    try:
        print(*args, **kwargs)
    except Exception:
        pass


# ---------------- TEXT NORMALIZATION ----------------

def normalize_text(text):
    """Lowercase, strip punctuation, collapse whitespace."""
    text = text.lower().replace("-", " ").replace("_", " ").replace("–", " ").replace("—", " ")
    text = re.sub(r"[^a-z0-9\s]", " ", text)
    text = re.sub(r"\s+", " ", text)
    return text.strip()


# ---------------- GAME PROCESS DETECTION ----------------

def get_game_processes():
    processes = []
    for process in psutil.process_iter(["pid", "name"]):
        try:
            if process.info["name"] in GAME_PROCESS_NAMES:
                processes.append(process)
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass
    return processes


def is_game_running():
    return len(get_game_processes()) > 0


# ---------------- FIND GAME WINDOWS ----------------

def find_game_windows():
    """Find visible top-level windows belonging to a WuWa process."""
    game_pids = {p.pid for p in get_game_processes()}
    windows = []

    def enum_callback(hwnd, extra):
        if not win32gui.IsWindowVisible(hwnd):
            return True
        try:
            left, top, right, bottom = win32gui.GetWindowRect(hwnd)
            width, height = right - left, bottom - top
            if width < 500 or height < 300:
                return True
        except Exception:
            return True
        try:
            _, pid = win32process.GetWindowThreadProcessId(hwnd)
            if pid not in game_pids:
                return True
        except Exception:
            return True

        windows.append({
            "hwnd": hwnd, "pid": pid, "title": win32gui.GetWindowText(hwnd),
            "rect": (left, top, right, bottom), "width": width, "height": height,
        })
        return True

    try:
        win32gui.EnumWindows(enum_callback, None)
    except Exception as e:
        log(f"[WINDOW] EnumWindows error: {e}")

    return windows


def find_best_game_window():
    windows = find_game_windows()
    if not windows:
        return None
    windows.sort(key=lambda w: w["width"] * w["height"], reverse=True)
    return windows[0]


def wait_for_game_window(timeout=60):
    log("[WINDOW] Waiting for Wuthering Waves window...")
    start = time.time()
    while True:
        if stop_event.is_set():
            return None
        window = find_best_game_window()
        if window:
            log("[WINDOW] Found WuWa window:")
            log(f"          Title: {window['title']}")
            log(f"          Size: {window['width']}x{window['height']}")
            return window
        if timeout is not None and time.time() - start > timeout:
            return None
        time.sleep(1)


# ---------------- WINDOW CAPTURE ----------------

def capture_game_window(window):
    """Capture the complete WuWa client area in physical pixels."""
    hwnd = window["hwnd"]
    if not win32gui.IsWindow(hwnd):
        return None

    try:
        # The process is DPI-aware, so this is the physical client size.
        left, top, right, bottom = win32gui.GetClientRect(hwnd)
        width, height = right - left, bottom - top
    except Exception as e:
        log(f"[CAPTURE] Could not get client size: {e}")
        return None

    if width <= 0 or height <= 0:
        log(f"[CAPTURE] Invalid client size: {width}x{height}")
        return None

    hwnd_dc = src_dc = mem_dc = bitmap = None
    try:
        hwnd_dc = win32gui.GetWindowDC(hwnd)
        if not hwnd_dc:
            return None

        src_dc = win32ui.CreateDCFromHandle(hwnd_dc)
        mem_dc = src_dc.CreateCompatibleDC()

        bitmap = win32ui.CreateBitmap()
        bitmap.CreateCompatibleBitmap(src_dc, width, height)
        mem_dc.SelectObject(bitmap)

        user32 = ctypes.WinDLL("user32", use_last_error=True)
        print_window = user32.PrintWindow
        print_window.restype = wintypes.BOOL
        print_window.argtypes = [wintypes.HWND, wintypes.HDC, wintypes.UINT]

        result = print_window(
            wintypes.HWND(int(hwnd)),
            wintypes.HDC(int(mem_dc.GetSafeHdc())),
            2,  # PW_RENDERFULLCONTENT
        )

        if not result:
            log(f"[CAPTURE] PrintWindow failed (error={ctypes.get_last_error()}).")
            return None

        info = bitmap.GetInfo()
        bits = bitmap.GetBitmapBits(True)
        image = Image.frombuffer(
            "RGB", (info["bmWidth"], info["bmHeight"]), bits,
            "raw", "BGRX", 0, 1
        )
        result_image = image.copy()

        if DEBUG and result_image.size != (width, height):
            log(
                f"[CAPTURE] Size mismatch: requested={width}x{height}, "
                f"bitmap={result_image.width}x{result_image.height}"
            )

        return result_image

    except Exception as e:
        log(f"[CAPTURE] Window capture error: {e}")
        return None

    finally:
        try:
            if mem_dc is not None:
                mem_dc.DeleteDC()
        except Exception:
            pass
        try:
            if src_dc is not None:
                src_dc.DeleteDC()
        except Exception:
            pass
        try:
            if hwnd_dc is not None:
                win32gui.ReleaseDC(hwnd, hwnd_dc)
        except Exception:
            pass
        try:
            if bitmap is not None:
                win32gui.DeleteObject(bitmap.GetSafeHandle())
        except Exception:
            pass


def prepare_for_ocr(image):
    """Grayscale + downscale + contrast/sharpen for Tesseract."""
    image = image.convert("L")
    width, height = image.size
    largest = max(width, height)

    if largest > MAX_OCR_DIMENSION:
        scale = MAX_OCR_DIMENSION / largest
        new_size = (max(1, int(width * scale)), max(1, int(height * scale)))
        # BILINEAR: faster than LANCZOS, plenty sharp enough for OCR
        image = image.resize(new_size, Image.Resampling.BILINEAR)

    image = ImageEnhance.Contrast(image).enhance(2.2)
    image = image.filter(ImageFilter.SHARPEN)
    return image


# ---------------- OCR ----------------

def perform_ocr(image):
    processed = prepare_for_ocr(image)

    try:
        data = pytesseract.image_to_data(
            processed, lang="eng", config="--oem 1 --psm 11",
            output_type=pytesseract.Output.DICT,
        )
    except Exception as e:
        log(f"[OCR ERROR] {e}")
        return {"text": "", "words": [], "image": processed}

    words = []
    for i in range(len(data["text"])):
        raw_text = data["text"][i].strip()
        if not raw_text:
            continue
        try:
            confidence = float(data["conf"][i])
        except Exception:
            confidence = 0
        if confidence < 25:  # drop low-confidence garbage
            continue
        word = normalize_text(raw_text)
        if not word:
            continue
        words.append({
            "text": word, "confidence": confidence,
            "left": data["left"][i], "top": data["top"][i],
            "width": data["width"][i], "height": data["height"][i],
        })

    combined_text = " ".join(w["text"] for w in words)
    return {"text": combined_text, "words": words, "image": processed}


# ---------------- FUZZY TEXT MATCHING ----------------

def similarity(a, b):
    return difflib.SequenceMatcher(None, a, b).ratio()


def find_fuzzy_phrase(words, target, threshold):
    """Slide a window over OCR words looking for a fuzzy match to target."""
    target = normalize_text(target)
    target_words = target.split()
    if not target_words:
        return None

    target_count = len(target_words)
    for window_size in range(max(1, target_count - 1), target_count + 3):
        for start in range(0, len(words) - window_size + 1):
            window = words[start:start + window_size]
            candidate = " ".join(w["text"] for w in window)
            score = similarity(target, candidate)
            if score >= threshold:
                return {"score": score, "candidate": candidate, "words": window}

    return None

def censor_uid(ocr):
    # Redacts "User ID: <ID>" from logs.
    words = ocr["words"]
    masked = []
    for i in range(len(words) - 1):
        if words[i]["text"] in ("id", "1d", "ld"):
            nxt = words[i + 1]
            if nxt["text"].isdigit() and 6 <= len(nxt["text"]) <= 12:
                nxt["text"] = "*" * len(nxt["text"])
                masked.append(nxt)
    ocr["text"] = " ".join(w["text"] for w in words)
    return masked
    
def blackout_uid(image, ocr, uid_words):
    # Censors the UID in the screenshot.
    if not uid_words:
        return image
    scale = image.width / max(1, ocr["image"].width)
    pad = 4
    draw = ImageDraw.Draw(image)
    for w in uid_words:
        draw.rectangle(
            (int((w["left"] - pad) * scale), int((w["top"] - pad) * scale),
             int((w["left"] + w["width"] + pad) * scale),
             int((w["top"] + w["height"] + pad) * scale)),
            fill=(0, 0, 0),
        )
    return image
    
# ---------------- DETECTION ----------------

def detect_login(ocr):
    result = find_fuzzy_phrase(ocr["words"], LOGIN_TARGET, LOGIN_MATCH_THRESHOLD)
    if not result:
        return None

    # Position isn't required, just a confidence boost (login text sits low).
    matched_words = result["words"]
    avg_top = sum(w["top"] for w in matched_words) / len(matched_words)
    relative_y = avg_top / max(1, ocr["image"].height)
    bottom_position = relative_y >= 0.65

    score = min(result["score"] + (0.05 if bottom_position else 0), 1.0)
    return {"score": score, "candidate": result["candidate"], "bottom_position": bottom_position}


def detect_patch(ocr):
# Require BOTH "patching complete" and "please restart the game" to be present roughly int the center

    words = ocr["words"]

    matches = []
    for target in PATCH_TARGETS:
        result = find_fuzzy_phrase(words, target, PATCH_MATCH_THRESHOLD)
        if not result:
            return None  # all targets must be found, not just one
        matches.append(result)

    matched_words = [w for r in matches for w in r["words"]]
    if not matched_words:
        return None

    width, height = ocr["image"].width, ocr["image"].height
    avg_left = sum(w["left"] for w in matched_words) / len(matched_words)
    avg_top = sum(w["top"] for w in matched_words) / len(matched_words)
    rel_x = avg_left / max(1, width)
    rel_y = avg_top / max(1, height)

    x_lo, x_hi = PATCH_CENTER_X_RANGE
    y_lo, y_hi = PATCH_CENTER_Y_RANGE
    if not (x_lo <= rel_x <= x_hi and y_lo <= rel_y <= y_hi):
        return None  # text found, but not where the popup actually appears

    score = min(r["score"] for r in matches)
    candidate = " | ".join(r["candidate"] for r in matches)
    return {"score": score, "candidate": candidate}

def save_debug_screenshot(image):
    if not DEBUG:
        return
    try:
        image.save(DEBUG_SCREENSHOT)
    except Exception as e:
        log(f"[DEBUG] Could not save screenshot: {e}")

# ---------------- CLOSE / LAUNCH GAME ----------------

def close_game():
    processes = get_game_processes()
    if not processes:
        log("[WuWa] No game process found.")
        return

    log("[WuWa] Closing Wuthering Waves...")
    for process in processes:
        try:
            log(f"[WuWa] Terminating {process.info['name']} (PID {process.pid})")
            process.terminate()
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass

    deadline = time.time() + 6
    while time.time() < deadline:
        if not is_game_running():
            log("[WuWa] Game closed.")
            return
        time.sleep(0.25)

    log("[WuWa] Game did not close normally. Force closing...")
    for process in get_game_processes():
        try:
            process.kill()
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass

    deadline = time.time() + 5
    while time.time() < deadline:
        if not is_game_running():
            log("[WuWa] Game force-closed.")
            return
        time.sleep(0.25)

    log("[WuWa] WARNING: WuWa process may still be running.")


# ---------------- AUTO-FIND GAME EXE (STEAM) ----------------

# Fallback for when the hardcoded GAME_EXE path is wrong, missing, or the game got moved/reinstalled to a different Steam library.

def find_steam_install_path():
    keys = [
        (winreg.HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
    ]
    for hive, subkey, value_name in keys:
        try:
            with winreg.OpenKey(hive, subkey) as key:
                value, _ = winreg.QueryValueEx(key, value_name)
                if value and os.path.isdir(value):
                    return value
        except OSError:
            continue
    return None


def find_steam_library_folders():
    steam_path = find_steam_install_path()
    if not steam_path:
        return []

    libraries = [steam_path]
    vdf_path = os.path.join(steam_path, "steamapps", "libraryfolders.vdf")

    try:
        with open(vdf_path, "r", encoding="utf-8", errors="ignore") as f:
            content = f.read()
        for match in re.finditer(r'"path"\s*"([^"]+)"', content):
            path = match.group(1).replace("\\\\", "\\")
            if path not in libraries:
                libraries.append(path)
    except Exception:
        pass

    return libraries


def _find_exe_in(base_dir, exe_names, max_depth=3):
    """Bounded search under base_dir for any of exe_names (avoids
    walking an entire drive if a folder turns out to be huge)."""
    if not base_dir or not os.path.isdir(base_dir):
        return None

    base_depth = base_dir.rstrip(os.sep).count(os.sep)
    for root, dirs, files in os.walk(base_dir):
        depth = root.rstrip(os.sep).count(os.sep) - base_depth
        if depth >= max_depth:
            dirs[:] = []  # don't descend further from here
        for name in exe_names:
            if name in files:
                return os.path.join(root, name)

    return None


def find_game_exe_via_registry(exe_names):
    
# Non-Steam installs

    uninstall_keys = [
        (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        (winreg.HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
    ]

    for hive, subkey in uninstall_keys:
        try:
            with winreg.OpenKey(hive, subkey) as key:
                for i in range(winreg.QueryInfoKey(key)[0]):
                    try:
                        entry_name = winreg.EnumKey(key, i)
                        with winreg.OpenKey(key, entry_name) as entry:
                            display_name = winreg.QueryValueEx(entry, "DisplayName")[0]
                            if "wuthering waves" not in display_name.lower():
                                continue
                            install_location = winreg.QueryValueEx(entry, "InstallLocation")[0]
                            found = _find_exe_in(install_location, exe_names)
                            if found:
                                return found
                    except OSError:
                        continue
        except OSError:
            continue

    return None


def find_game_exe_common_locations(exe_names):
    """Last-ditch guess: check a few typical default install folder
    names on every fixed drive letter, for non-Steam installs that
    didn't register properly or got moved by hand."""
    folder_names = [
        "Wuthering Waves",
        os.path.join("Wuthering Waves", "Wuthering Waves Game"),
        os.path.join("Games", "Wuthering Waves"),
        os.path.join("Program Files", "Wuthering Waves"),
    ]

    drive_bits = ctypes.windll.kernel32.GetLogicalDrives()
    drives = [f"{chr(65 + i)}:\\" for i in range(26) if drive_bits & (1 << i)]

    for drive in drives:
        for folder in folder_names:
            found = _find_exe_in(os.path.join(drive, folder), exe_names, max_depth=2)
            if found:
                return found

    return None


def find_game_exe_auto():

# Best-effort search. Order: Steam libraries > Windows installed programs registry (standalone/non-Steam) > common default install locations.

    exe_names = ["Wuthering Waves.exe", "Client-Win64-Shipping.exe"]

    for library in find_steam_library_folders():
        found = _find_exe_in(
            os.path.join(library, "steamapps", "common", "Wuthering Waves"),
            exe_names,
        )
        if found:
            return found

    found = find_game_exe_via_registry(exe_names)
    if found:
        return found

    return find_game_exe_common_locations(exe_names)

EXPECTED_EXE_NAMES = ["Wuthering Waves.exe", "Client-Win64-Shipping.exe", "WutheringWaves.exe"]

def prompt_browse_for_game_exe():

# If nothing was auto-detected: ask the user to browse for the exe.

    messagebox.showinfo(
        "WuWa Watchdog",
        "Could not find Wuthering Waves automatically.\n\n"
        "In the next window, browse into the game's install folder and "
        "select its MAIN executable -- usually named:\n\n"
        "    Wuthering Waves.exe\n\n"
        "(or, if you go into a \"Binaries\" subfolder, "
        "Client-Win64-Shipping.exe)\n\n"
        "Do NOT pick a launcher, updater, or crash-reporter exe -- those "
        "usually sit one folder up or have \"Launcher\"/\"CrashReport\" "
        "in the name.\n\n"
        "A typical path looks like:\n"
        "...\\Wuthering Waves\\Wuthering Waves Game\\Wuthering Waves.exe",
    )

    root = tk.Tk()
    root.withdraw()

    while True:
        path = filedialog.askopenfilename(
            title="Select Wuthering Waves' MAIN game executable",
            filetypes=[("Executable", "*.exe"), ("All files", "*.*")],
        )

        if not path or not os.path.isfile(path):
            root.destroy()
            return None

        if os.path.basename(path) in EXPECTED_EXE_NAMES:
            root.destroy()
            return path

        # Picked something unsure? warn instead of silently rejecting it.
        
        use_anyway = messagebox.askyesno(
            "WuWa Watchdog",
            f'"{os.path.basename(path)}" doesn\'t match the usual game '
            f"exe names ({', '.join(EXPECTED_EXE_NAMES)}).\n\n"
            "It might be a launcher or another tool instead of the "
            "actual game client.\n\n"
            "Use it anyway?",
        )
        if use_anyway:
            root.destroy()
            return path
        
        # else: loop back and let them browse again


def resolve_game_exe():
    """
    Figures out which game exe to use, in priority order:
      1. A path remembered in config.json from any previous run
      2. The hardcoded GAME_EXE default in this script
      3. Auto-detection (Steam, registry, common folders)
      4. Asking the user to browse for it
    Whichever one succeeds gets saved to config.json, so future runs
    skip straight to step 1 (this also skips re-scanning every drive
    letter on every single launch once it's found the game once).
    Returns the resolved path, or None if the user gave up.

    Testing hooks (env vars, both no-op unless set to "1"):
      WUWA_TEST_NO_CONFIG      - ignore any remembered config.json path
      WUWA_TEST_NO_AUTODETECT  - skip hardcoded/Steam/registry/common-folder
                                  detection entirely, forcing the browse dialog
    Lets you exercise the fallback dialogs on a machine where everything
    is already correctly set up, without touching real files.
    """
    config = load_config()

    if os.environ.get("WUWA_TEST_NO_CONFIG") != "1":
        remembered = config.get("game_exe")
        if remembered and os.path.isfile(remembered):
            return remembered
    else:
        config = {}

    if os.environ.get("WUWA_TEST_NO_AUTODETECT") != "1":
        if os.path.isfile(GAME_EXE):
            config["game_exe"] = GAME_EXE
            save_config(config)
            return GAME_EXE

        auto_path = find_game_exe_auto()
        if auto_path:
            log(f"[SYSTEM] Auto-detected game exe: {auto_path}")
            config["game_exe"] = auto_path
            save_config(config)
            return auto_path
    else:
        log("[TEST] WUWA_TEST_NO_AUTODETECT=1 -- skipping straight to browse dialog.")

    browsed = prompt_browse_for_game_exe()
    if browsed:
        log(f"[SYSTEM] User-selected game exe: {browsed}")
        config["game_exe"] = browsed
        save_config(config)
        return browsed

    return None


def launch_game():
    if not os.path.isfile(GAME_EXE):
        log(f"\n[ERROR] Game executable was not found:\n        {GAME_EXE}")
        log("The game may have been moved or uninstalled.")
        log(f"Delete {CONFIG_FILE} and restart the app to pick the game exe again.")
        return False

    log("[WuWa] Launching game...")
    try:
        subprocess.Popen(
            [GAME_EXE], cwd=os.path.dirname(GAME_EXE),
            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
        )
        return True
    except Exception as e:
        log(f"[WuWa] Launch error: {e}")
        return False


def wait_for_game_process():
    log("[WuWa] Waiting for game process...")
    while not is_game_running():
        if stop_event.is_set():
            return
        time.sleep(1)
    log("[WuWa] Game process detected.")


def wait_for_window_after_launch():
    if stop_event.is_set():
        return
    wait_for_game_process()
    return wait_for_game_window(timeout=120)


def restart_countdown():
    log(f"\n[WuWa] Waiting {RESTART_WAIT_SECONDS} seconds before restarting...")
    for remaining in range(RESTART_WAIT_SECONDS, 0, -1):
        if stop_event.is_set():
            return
        safe_print(f"\r[WuWa] Restarting in {remaining:3d} seconds...", end="", flush=True)
        time.sleep(1)
    safe_print()


# ---------------- MAIN OCR LOOP ----------------

def monitor_game():
    login_count = 0
    patch_count = 0
    scan_number = 0

    while True:
        if stop_event.is_set():
            log("[WuWa] Watchdog stopped by user (tray quit).")
            return

        # Re-find the window each loop since restarts change the HWND.
        window = find_best_game_window()
        if not window:
            if not is_game_running():
                log("[WuWa] Game process disappeared. Waiting for it to return...")
                wait_for_game_process()
            window = wait_for_game_window(timeout=30)
            if not window:
                time.sleep(1)
                continue

        image = capture_game_window(window)
        if image is None:
            time.sleep(1)
            continue

        scan_number += 1

        ocr = perform_ocr(image)
        
        uid_words = censor_uid(ocr)
        
        if DEBUG and (scan_number % DEBUG_SCREENSHOT_EVERY_N == 0 or uid_words):
            save_debug_screenshot(blackout_uid(image, ocr, uid_words))

        if DEBUG:
            log("\n[DEBUG] OCR WORDS:")
            for word in ocr["words"]:
                text = word["text"]
                if text.isdigit() and len(text) >= 7:
                    text = "*" * len(text)
                log(f"    '{text}' conf={word['confidence']:.1f} "
                    f"x={word['left']} y={word['top']} w={word['width']} h={word['height']}")
            log("")
        
        if uid_words:
            log("\n" + "=" * 60)
            log("[OCR] User ID detected (game already running)")
            log("=" * 60)
            log("\nWuthering Waves will remain running.")
            log("The watchdog is exiting.\n")
            set_status("User ID detected - watchdog finished.", also_log=False)
            return

        login = detect_login(ocr)
        patch = detect_patch(ocr)

        if login:
            login_count += 1
            log(f"[LOGIN] Possible detection {login_count}/{LOGIN_CONFIRMATIONS_REQUIRED} "
                f"| score={login['score']:.2f} | text='{login['candidate']}'")
        else:
            login_count = 0

        if patch:
            patch_count += 1
            log(f"[PATCH] Possible detection {patch_count}/{PATCH_CONFIRMATIONS_REQUIRED} "
                f"| score={patch['score']:.2f} | text='{patch['candidate']}'")
        else:
            patch_count = 0

        if login_count >= LOGIN_CONFIRMATIONS_REQUIRED:
            log("\n" + "=" * 60)
            log("[SUCCESS] LOGIN SCREEN CONFIRMED")
            log("=" * 60)
            log(f"\nDetected:\n    {login['candidate']}\n")
            log("Wuthering Waves will remain running.")
            log("The watchdog is exiting.\n")
            set_status("Login detected - watchdog finished.", also_log=False)
            return

        if patch_count >= PATCH_CONFIRMATIONS_REQUIRED:
            log("\n" + "=" * 60)
            log("[PATCH] RESTART MESSAGE CONFIRMED")
            log("=" * 60)
            log(f"\nDetected: {patch['candidate']}\n")
            set_status("Patch detected - restarting game...", also_log=False)

            login_count = 0
            patch_count = 0

            close_game()
            restart_countdown()

            if stop_event.is_set():
                log("[WuWa] Watchdog stopped by user (tray quit).")
                return

            if launch_game():
                log("[WuWa] Restart launched.")
                new_window = wait_for_window_after_launch()
                log("[WuWa] New game window detected." if new_window
                    else "[WuWa] WARNING: Game window was not found yet.")
                set_status("Monitoring for login/patch screen...", also_log=False)
            else:
                log("[WuWa] Restart failed.")
                set_status("Restart failed - see log.", also_log=False)
                return

            continue

        time.sleep(CHECK_INTERVAL_SECONDS)


# ---------------- TESSERACT CHECK ----------------

def find_tesseract():
    candidates = [
        TESSERACT_PATH,
        os.path.join(os.environ.get("LOCALAPPDATA", ""), "Programs", "Tesseract-OCR", "tesseract.exe"),
        shutil.which("tesseract"),
    ]
    for c in candidates:
        if c and os.path.isfile(c):
            return c
    return None


def _tesseract_present():
    if os.environ.get("WUWA_TEST_NO_TESSERACT") == "1":
        return False
    found = find_tesseract()
    if found:
        pytesseract.pytesseract.tesseract_cmd = found
    return bool(found)

def ensure_tesseract_installed():

# If Tesseract isn't found, pop up a small dialog offering to open the download page

    if _tesseract_present():
        return True

    root = tk.Tk()
    root.withdraw()

    while not _tesseract_present():
        wants_download = messagebox.askyesno(
            "Tesseract-OCR Not Found",
            "This script needs Tesseract-OCR, which isn't installed "
            f"at:\n\n{TESSERACT_PATH}\n\n"
            "Open the download page now?",
        )

        if not wants_download:
            root.destroy()
            return False

        webbrowser.open(TESSERACT_DOWNLOAD_URL)

        keep_waiting = messagebox.askokcancel(
            "Waiting for Install",
            "Install Tesseract-OCR (default install path recommended), "
            "then click OK to continue.\n\n"
            "Click Cancel to give up and exit.",
        )

        if not keep_waiting:
            root.destroy()
            return False

    root.destroy()
    return True


# ---------------- SYSTEM TRAY ----------------

# The live log viewer runs on its own dedicated thread with its own Tk
# root, kept alive for the whole app lifetime. It's separate from both
# the watchdog thread and pystray's main-thread event loop -- Tkinter
# widgets can only safely be touched from the thread that owns them, so
# this thread does nothing except drain _log_queue into a Text widget.

_log_viewer_show_event = threading.Event()
_log_viewer_root = None
_log_viewer_text = None


def _log_viewer_poll():
    updated = False
    while True:
        try:
            line = _log_queue.get_nowait()
        except queue.Empty:
            break
        _log_viewer_text.insert(tk.END, line + "\n")
        updated = True

    if updated:
        if int(_log_viewer_text.index("end-1c").split(".")[0]) > 3000:
            _log_viewer_text.delete("1.0", "1000.0")  # drop oldest lines
        _log_viewer_text.see(tk.END)

    if _log_viewer_show_event.is_set():
        _log_viewer_show_event.clear()
        _log_viewer_root.deiconify()
        _log_viewer_root.lift()

    _log_viewer_root.after(200, _log_viewer_poll)


def run_log_viewer():
    global _log_viewer_root, _log_viewer_text

    root = tk.Tk()
    root.title("WuWa Watchdog - Live Log")
    root.geometry("820x480")
    # Closing the window minimizes it. Quit in the systray ends the app.
    root.protocol("WM_DELETE_WINDOW", root.withdraw)

    text = tk.Text(root, wrap="word", bg="#111318", fg="#ddd", insertbackground="#ddd")
    scrollbar = tk.Scrollbar(root, command=text.yview)
    text.configure(yscrollcommand=scrollbar.set)
    scrollbar.pack(side="right", fill="y")
    text.pack(side="left", fill="both", expand=True)

    _log_viewer_root = root
    _log_viewer_text = text
    root.withdraw()  # starts hidden

    root.after(200, _log_viewer_poll)
    root.mainloop()


def build_tray_image():
    """Load a bundled tray icon if present, else generate a placeholder."""
    icon_path = resource_path(TRAY_ICON_FILENAME)
    if os.path.isfile(icon_path):
        try:
            return Image.open(icon_path)
        except Exception:
            pass

    image = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.ellipse((2, 2, 62, 62), fill=(60, 110, 220, 255))
    draw.text((22, 20), "W", fill=(255, 255, 255, 255))
    return image


def on_show_log(icon, item):
    _log_viewer_show_event.set()


def on_show_debug_screenshot(icon, item):
    if not DEBUG_SCREENSHOT.exists():
        ctypes.windll.user32.MessageBoxW(0, "No debug screenshot saved yet.", "WuWa Watchdog", 0x40)
        return
    try:
        os.startfile(str(DEBUG_SCREENSHOT))
    except Exception as e:
        log(f"[TRAY] Could not open debug screenshot: {e}")


def on_open_data_folder(icon, item):
    try:
        os.startfile(APP_DIR)
    except Exception as e:
        log(f"[TRAY] Could not open data folder: {e}")


def on_quit(icon, item):
    set_status("Stopping (quit requested)...")
    stop_event.set()
    icon.stop()


def build_tray_menu():
    items = [
        pystray.MenuItem(lambda item: _status_text, None, enabled=False),
        pystray.Menu.SEPARATOR,
        pystray.MenuItem("Show Live Log", on_show_log),
    ]
    if DEBUG:
        items.append(pystray.MenuItem("Show Debug Screenshot", on_show_debug_screenshot))
    items += [
        pystray.MenuItem("Open Data Folder", on_open_data_folder),
        pystray.Menu.SEPARATOR,
        pystray.MenuItem("Quit", on_quit),
    ]
    return pystray.Menu(*items)


def run_watchdog(icon):

# Runs on a background thread (spawned by pystray) once the tray icon is visible.

    global _tray_icon
    _tray_icon = icon
    icon.visible = True

    try:
        if is_game_running():
            set_status("Game already running.")
        elif not launch_game():
            set_status("Failed to launch game - see log.")
            return

        set_status("Waiting for game window...")
        window = wait_for_window_after_launch()
        if not window:
            set_status("Game window not found - see log.")
            log("The game may still be starting.")
            return

        log("\n" + "=" * 60)
        log("[OCR] MONITORING STARTED")
        log("=" * 60)
        log("\n[OCR] Looking for:")
        log(f"       LOGIN  = {LOGIN_TARGET}")
        log("       PATCH  = Patching complete / Please restart")
        log("\n[OCR] Entire WuWa window is scanned.\n")

        set_status("Monitoring for login/patch screen...")
        monitor_game()

    except Exception as e:
        log(f"\n[FATAL ERROR] {e}")
        log("The game has NOT been intentionally closed.")
        set_status("Fatal error - see log.")

    finally:
        icon.stop()

_instance_mutex = None

def ensure_single_instance():
    """Returns False if another copy of the watchdog is already running."""
    global _instance_mutex
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _instance_mutex = kernel32.CreateMutexW(None, False, "Local\\WuWaWatchdog_SingleInstance")
    if ctypes.get_last_error() == 183:  # ERROR_ALREADY_EXISTS
        root = tk.Tk()
        root.withdraw()
        messagebox.showinfo(
            "WuWa Watchdog",
            "The watchdog is already running (check your system tray).",
        )
        root.destroy()
        return False
    return True

def main():
    if not ensure_single_instance():
        return
        
    safe_print("\n" + "=" * 60)
    safe_print("       WUTHERING WAVES OCR WATCHDOG")
    safe_print("=" * 60 + "\n")

    # Must run before any log() call: log() now keeps the file open for
    # the whole session, and Windows can't delete a file that's open.
    for stale_file in (LOG_FILE, DEBUG_SCREENSHOT):
        try:
            stale_file.unlink()
        except FileNotFoundError:
            pass
        except OSError:
            pass

    if not ensure_tesseract_installed():
        log("[ERROR] Tesseract-OCR is required. Exiting.")
        messagebox.showerror("WuWa Watchdog", "Tesseract-OCR is required. Exiting.")
        return
    log("[SYSTEM] Tesseract found.")

    global GAME_EXE
    resolved = resolve_game_exe()
    if not resolved:
        log("[ERROR] No Wuthering Waves executable was selected. Exiting.")
        messagebox.showerror("WuWa Watchdog", "No Wuthering Waves executable was selected. Exiting.")
        return
    GAME_EXE = resolved
    log(f"[SYSTEM] Using game executable: {GAME_EXE}")

    threading.Thread(target=run_log_viewer, daemon=True).start()

    icon = pystray.Icon(
        "wuwa_watchdog",
        build_tray_image(),
        "WuWa Watchdog: Starting...",
        menu=build_tray_menu(),
    )
    
    icon.run(setup=run_watchdog)


if __name__ == "__main__":
    main()