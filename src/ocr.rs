//! OCR through the built-in Windows OCR engine (Windows.Media.Ocr) - nothing to install or bundle.

use std::path::Path;

use windows::core::{Interface, Result, HSTRING};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::{Buffer, DataReader, DataWriter, IBuffer};
use windows::Storage::{FileAccessMode, StorageFile};

use crate::detect::{Ocr, RawWord};
use crate::image_ops::Frame;

/// Is the Windows OCR language for this tag (e.g. "ja-JP") installed?
pub fn language_supported(tag: &str) -> bool {
    match Language::CreateLanguage(&HSTRING::from(tag)) {
        Ok(lang) => OcrEngine::IsLanguageSupported(&lang).unwrap_or(false),
        Err(_) => false,
    }
}

/// The OCR engine for a language tag. For English any English OCR the user has is accepted
/// (en-US, en-GB, ... or the profile languages); every other language needs its own OCR pack.
pub fn create_engine_for(tag: &str) -> Result<OcrEngine> {
    if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(tag)) {
        if OcrEngine::IsLanguageSupported(&lang).unwrap_or(false) {
            if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                return Ok(engine);
            }
        }
    }
    if tag.to_lowercase().starts_with("en") {
        return match OcrEngine::TryCreateFromUserProfileLanguages() {
            Ok(engine) => Ok(engine),
            Err(_) => {
                let lang = Language::CreateLanguage(&HSTRING::from("en-US"))?;
                OcrEngine::TryCreateFromLanguage(&lang)
            }
        };
    }
    Err(windows::core::Error::new(
        windows::Win32::Foundation::E_FAIL,
        format!("the Windows OCR language pack for {tag} is not installed"),
    ))
}

/// The English engine (startup check, tests).
pub fn create_engine() -> Result<OcrEngine> {
    create_engine_for("en-US")
}

/// Startup check. Runs on a throw-away thread so the UI thread never touches WinRT.
pub fn available() -> bool {
    std::thread::spawn(|| create_engine().is_ok()).join().unwrap_or(false)
}

/// `target_height`: frames taller than this are box-downscaled to it first (see settings: OCR image height).
pub fn recognize(engine: &OcrEngine, frame: &Frame, target_height: i32) -> Result<Ocr> {
    let scaled;
    let image: &Frame = if frame.height > target_height {
        scaled = frame.resized_to_height(target_height);
        &scaled
    } else {
        frame
    };

    let writer = DataWriter::new()?;
    writer.WriteBytes(&image.bgra)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, image.width, image.height)?;

    let result = engine.RecognizeAsync(&bitmap)?.get()?;

    let mut raw = Vec::new();
    for line in result.Lines()? {
        for word in line.Words()? {
            let rect = word.BoundingRect()?;
            raw.push(RawWord {
                text: word.Text()?.to_string(),
                left: rect.X,
                top: rect.Y,
                width: rect.Width,
                height: rect.Height,
            });
        }
    }
    Ok(Ocr::from_raw(&raw, image.width, image.height))
}

/// Decodes an image file (png / jpg / bmp ...) into a BGRA frame, like a window capture.
pub fn load_image(path: &Path) -> Result<Frame> {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(abs.to_string_lossy().as_ref()))?.get()?;
    let stream = file.OpenAsync(FileAccessMode::Read)?.get()?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
    let bitmap = decoder.GetSoftwareBitmapAsync()?.get()?;
    let bgra = SoftwareBitmap::Convert(&bitmap, BitmapPixelFormat::Bgra8)?;
    let (width, height) = (bgra.PixelWidth()?, bgra.PixelHeight()?);
    let len = width as usize * height as usize * 4;

    let buffer = Buffer::Create(len as u32)?;
    buffer.SetLength(len as u32)?;
    let ibuffer: IBuffer = buffer.cast()?;
    bgra.CopyToBuffer(&ibuffer)?;
    let reader = DataReader::FromBuffer(&ibuffer)?;
    let mut bytes = vec![0u8; len];
    reader.ReadBytes(&mut bytes)?;
    for px in bytes.chunks_exact_mut(4) {
        px[3] = 255; // opaque, like a window capture
    }
    Ok(Frame { width, height, bgra: bytes })
}
