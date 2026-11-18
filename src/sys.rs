//! Small OS helpers: clipboard, file dialogs, Explorer.

use std::path::{Path, PathBuf};
use std::process::Command;

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn buffer_to_path(buf: &[u16]) -> PathBuf {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    PathBuf::from(String::from_utf16_lossy(&buf[..end]))
}

/// "2026-11-02 12:34:56" -> "2026-11-02_12-34-56" (safe in file names)
pub fn file_stamp() -> String {
    crate::log::timestamp().replace(' ', "_").replace(':', "-")
}

/// Puts `text` on the clipboard as Unicode text.
pub fn clipboard_set_text(owner: HWND, text: &str) -> bool {
    unsafe {
        let data = wide(text);
        let bytes = data.len() * 2;
        if OpenClipboard(owner).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let stored = (|| -> Option<()> {
            let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes).ok()?;
            let ptr = GlobalLock(hmem) as *mut u16;
            if ptr.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            let _ = GlobalUnlock(hmem);
            SetClipboardData(13, HANDLE(hmem.0)).ok()?; // CF_UNICODETEXT; the clipboard owns the memory now
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        stored
    }
}

/// "Open file" dialog for .json files.
pub fn pick_open_file(owner: HWND, title: &str) -> Option<PathBuf> {
    unsafe {
        let mut file = vec![0u16; 1024];
        let filter = wide("JSON files (*.json)\0*.json\0All files (*.*)\0*.*\0");
        let title_w = wide(title);
        let mut ofn = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(file.as_mut_ptr()),
            nMaxFile: file.len() as u32,
            lpstrTitle: PCWSTR(title_w.as_ptr()),
            Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        if GetOpenFileNameW(&mut ofn).as_bool() {
            Some(buffer_to_path(&file))
        } else {
            None
        }
    }
}

/// "Open file" dialog for screenshots (png / jpg / bmp).
pub fn pick_image_file(owner: HWND, title: &str) -> Option<PathBuf> {
    unsafe {
        let mut file = vec![0u16; 1024];
        let filter = wide("Images (*.png;*.jpg;*.jpeg;*.bmp)\0*.png;*.jpg;*.jpeg;*.bmp\0All files (*.*)\0*.*\0");
        let title_w = wide(title);
        let mut ofn = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(file.as_mut_ptr()),
            nMaxFile: file.len() as u32,
            lpstrTitle: PCWSTR(title_w.as_ptr()),
            Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        if GetOpenFileNameW(&mut ofn).as_bool() {
            Some(buffer_to_path(&file))
        } else {
            None
        }
    }
}

/// "Save file" dialog for .json files, starting in `dir` with `default_name` filled in.
pub fn pick_save_file(owner: HWND, title: &str, dir: &Path, default_name: &str) -> Option<PathBuf> {
    unsafe {
        let mut file = vec![0u16; 1024];
        let name = wide(default_name);
        let n = name.len().min(file.len() - 1);
        file[..n].copy_from_slice(&name[..n]);
        let filter = wide("JSON files (*.json)\0*.json\0All files (*.*)\0*.*\0");
        let title_w = wide(title);
        let dir_w = wide(&dir.to_string_lossy());
        let mut ofn = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(file.as_mut_ptr()),
            nMaxFile: file.len() as u32,
            lpstrTitle: PCWSTR(title_w.as_ptr()),
            lpstrInitialDir: PCWSTR(dir_w.as_ptr()),
            lpstrDefExt: PCWSTR(w!("json").0),
            Flags: OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        if GetSaveFileNameW(&mut ofn).as_bool() {
            Some(buffer_to_path(&file))
        } else {
            None
        }
    }
}

/// Opens a file or folder with its default app / in Explorer.
pub fn open_path(path: &Path) {
    let _ = Command::new("explorer.exe").arg(path).spawn();
}

/// Opens Explorer with the file selected.
pub fn reveal(path: &Path) {
    let _ = Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn();
}
