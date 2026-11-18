//! Types shared by the main window and its tabs.

use windows::Win32::Graphics::Gdi::HFONT;

#[derive(Clone, Copy)]
pub struct Fonts {
    pub ui: HFONT,
    pub bold: HFONT,
    pub small: HFONT,
    pub mono: HFONT,
    pub title: HFONT,
    /// big page title ("Live Logs", "Settings", ...)
    pub page: HFONT,
}

/// Everything a tab needs to lay out and draw itself.
#[derive(Clone, Copy)]
pub struct Ctx {
    pub scale: f32,
    pub fonts: Fonts,
    /// width of one character of the monospaced log font
    pub char_w: i32,
    pub line_h: i32,
}

/// Things a tab asks the main window to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    CopyLogs,
    ExportLogs,
    ImportSettings,
    ExportSettings,
    DebugShot,
    OpenData,
	/// a notification setting changed: show the box for 2 s (each change restarts the timer)
    LiveOverlayPreview,
    /// run the real OCR + detection on a simulated login/patch screen
    SelfTest,
    /// run the real OCR + detection on an image file the user picks
    TestImage,
    /// show the notification box for a few seconds
    PreviewOverlay,
    /// re-read languages.json
    ReloadLanguages,
    OpenLanguagesFile,
    /// forget the saved game install (the next start asks again)
    ForgetGame,
}

/// What happened in a tab after an input event.
#[derive(Default)]
pub struct React {
    /// the tab needs repainting
    pub redraw: bool,
    /// start the smooth-scroll timer
    pub anim: bool,
    /// capture the mouse (a slider drag started)
    pub capture: bool,
    /// release the mouse capture (the drag ended)
    pub release: bool,
    /// the hovered item has a tooltip: start the tooltip delay
    pub tip: bool,
    pub action: Option<Action>,
}
