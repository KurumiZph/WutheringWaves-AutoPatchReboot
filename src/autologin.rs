//! Auto login: after the "Tap to land in Solaris-3" screen is confirmed, click the middle of the game
//! window (or press Space). Nothing else is ever sent to the game.
//!
//! Both actions are real input (SendInput), so the game window is brought to the front first.
//! The mouse is put back where it was after a click.

use std::time::Duration;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursorPos, GetForegroundWindow, IsIconic, SetCursorPos, SetForegroundWindow, ShowWindow,
    SW_RESTORE,
};

use crate::capture::GameWindow;
use crate::settings::LoginMethod;

fn pause(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

unsafe fn send_mouse(flags: MOUSE_EVENT_FLAGS) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
}

/// Key press by hardware scan code (what games read) or by virtual key.
unsafe fn send_key(vk: u16, scan: u16, by_scan: bool, up: bool) {
    let mut flags = if by_scan { KEYEVENTF_SCANCODE } else { KEYBD_EVENT_FLAGS(0) };
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
}

/// Brings the game window to the front (restoring it if minimised).
unsafe fn focus(hwnd: HWND) -> Result<(), String> {
    if IsIconic(hwnd).as_bool() {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        pause(400);
    }
    // a tap on Alt lets SetForegroundWindow succeed from a background process
    send_key(0x12, 0, false, false);
    send_key(0x12, 0, false, true);
    let _ = SetForegroundWindow(hwnd);
    for _ in 0..15 {
        if GetForegroundWindow() == hwnd {
            return Ok(());
        }
        pause(50);
    }
    Err("could not bring the game window to the front".to_string())
}

/// Performs the configured action. Returns a log line describing what was done.
pub fn perform(window: &GameWindow, method: LoginMethod) -> Result<String, String> {
    unsafe {
        focus(window.hwnd)?;
        pause(150);
        match method {
            LoginMethod::Click => {
                let mut rc = RECT::default();
                GetClientRect(window.hwnd, &mut rc).map_err(|e| format!("GetClientRect failed: {e}"))?;
                let mut pt = POINT { x: (rc.right - rc.left) / 2, y: (rc.bottom - rc.top) / 2 };
                let _ = ClientToScreen(window.hwnd, &mut pt);

                let mut old = POINT::default();
                let had_old = GetCursorPos(&mut old).is_ok();
                SetCursorPos(pt.x, pt.y).map_err(|e| format!("SetCursorPos failed: {e}"))?;
                pause(120);
                send_mouse(MOUSEEVENT_DOWN);
                pause(60);
                send_mouse(MOUSEEVENT_UP);
                pause(150);
                if had_old {
                    let _ = SetCursorPos(old.x, old.y);
                }
                Ok(format!("[LOGIN] Auto login: clicked the middle of the game window ({}, {}).", pt.x, pt.y))
            }
            LoginMethod::Space => {
                send_key(0, 0x39, true, false); // scan code 0x39 = Space
                pause(60);
                send_key(0, 0x39, true, true);
                Ok("[LOGIN] Auto login: sent the Space key to the game.".to_string())
            }
        }
    }
}

const MOUSEEVENT_DOWN: MOUSE_EVENT_FLAGS = MOUSEEVENTF_LEFTDOWN;
const MOUSEEVENT_UP: MOUSE_EVENT_FLAGS = MOUSEEVENTF_LEFTUP;
