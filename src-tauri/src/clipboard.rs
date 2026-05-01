use crate::{
    models::{NewClipboardItem, Settings, SourceMetadata},
    source,
    store::unix_now,
    AppState,
};
use arboard::{Clipboard, ImageData};
use image::{imageops::FilterType, DynamicImage, ImageFormat, RgbaImage};
use sha2::{Digest, Sha256};
use std::{borrow::Cow, io::Cursor, sync::Arc, thread, time::Duration};
use tauri::{AppHandle, Emitter};

#[derive(Default)]
pub struct ClipboardCache {
    last_hash: Option<String>,
}

impl ClipboardCache {
    pub fn remember(&mut self, hash: String) {
        self.last_hash = Some(hash);
    }

    fn is_last(&self, hash: &str) -> bool {
        self.last_hash.as_deref() == Some(hash)
    }
}

pub fn start_monitor(app: AppHandle, state: Arc<AppState>) {
    thread::spawn(move || {
        let mut clipboard = match Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(err) => {
                eprintln!("Clipboard monitor could not start: {err}");
                return;
            }
        };

        loop {
            let settings = match state.settings.lock() {
                Ok(settings) => settings.clone(),
                Err(_) => Settings::default(),
            };

            if !settings.paused {
                if let Err(err) = capture_once(&app, &state, &settings, &mut clipboard) {
                    eprintln!("Clipboard capture skipped: {err}");
                }
            }

            thread::sleep(Duration::from_millis(700));
        }
    });
}

pub fn capture_now(app: &AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    let settings = state
        .settings
        .lock()
        .map_err(|err| err.to_string())?
        .clone();
    if settings.paused {
        return Ok(());
    }

    let mut clipboard = Clipboard::new().map_err(|err| err.to_string())?;
    capture_once(app, state, &settings, &mut clipboard)
}

pub fn copy_item_to_system_clipboard(state: &Arc<AppState>, id: i64) -> Result<(), String> {
    let (kind, content, blob, hash) = state.store.get_item_payload(id)?;
    let mut clipboard = Clipboard::new().map_err(|err| err.to_string())?;

    match kind.as_str() {
        "image" => {
            let blob = blob.ok_or_else(|| "Image payload is missing".to_string())?;
            let decoded = image::load_from_memory(&blob)
                .map_err(|err| err.to_string())?
                .to_rgba8();
            let (width, height) = decoded.dimensions();
            clipboard
                .set_image(ImageData {
                    width: width as usize,
                    height: height as usize,
                    bytes: Cow::Owned(decoded.into_raw()),
                })
                .map_err(|err| err.to_string())?;
        }
        _ => clipboard.set_text(content).map_err(|err| err.to_string())?,
    }

    if let Ok(mut cache) = state.clipboard_cache.lock() {
        cache.remember(hash);
    }

    Ok(())
}

pub fn copy_text_to_system_clipboard(state: &Arc<AppState>, text: String) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|err| err.to_string())?;
    clipboard
        .set_text(text.clone())
        .map_err(|err| err.to_string())?;

    if let Ok(mut cache) = state.clipboard_cache.lock() {
        cache.remember(hash_bytes("text", text.trim().as_bytes()));
    }

    Ok(())
}

fn capture_once(
    app: &AppHandle,
    state: &Arc<AppState>,
    settings: &Settings,
    clipboard: &mut Clipboard,
) -> Result<(), String> {
    if let Ok(text) = clipboard.get_text() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            let hash = hash_bytes("text", trimmed.as_bytes());
            if should_process(state, &hash)? {
                let source = source::detect_source();
                if !source::should_skip_source(&source) {
                    let item = text_item(trimmed.to_string(), source, hash.clone());
                    if state.store.save_item(item, settings)? {
                        let _ = app.emit("clipboard-updated", unix_now());
                    }
                }
                if let Ok(mut cache) = state.clipboard_cache.lock() {
                    cache.remember(hash);
                }
            }
            return Ok(());
        }
    }

    if let Ok(image) = clipboard.get_image() {
        let hash = hash_image(&image);
        if should_process(state, &hash)? {
            let source = source::detect_source();
            if !source::should_skip_source(&source) {
                let (png, thumb) = image_to_pngs(&image)?;
                let item = NewClipboardItem {
                    kind: "image".to_string(),
                    content: format!("Image {} x {}", image.width, image.height),
                    url: None,
                    domain: source.domain.clone(),
                    title: source.page_title.clone(),
                    source,
                    hash: hash.clone(),
                    blob: Some(png),
                    thumb: Some(thumb),
                };

                if state.store.save_item(item, settings)? {
                    let _ = app.emit("clipboard-updated", unix_now());
                }
            }
            if let Ok(mut cache) = state.clipboard_cache.lock() {
                cache.remember(hash);
            }
        }
    }

    Ok(())
}

fn should_process(state: &Arc<AppState>, hash: &str) -> Result<bool, String> {
    let cache = state
        .clipboard_cache
        .lock()
        .map_err(|err| err.to_string())?;
    Ok(!cache.is_last(hash))
}

fn text_item(content: String, source_meta: SourceMetadata, hash: String) -> NewClipboardItem {
    let parsed_domain = source::domain_from_url(&content);
    let is_link = parsed_domain.is_some()
        && (content.starts_with("http://") || content.starts_with("https://"));
    let clipboard_link = (!is_link)
        .then(|| source::clipboard_link_for_text(&content))
        .flatten();
    let (exact_source_url, exact_source_title, exact_source_domain) = clipboard_link
        .map(|(url, title, domain)| (Some(url), title, domain))
        .unwrap_or((None, None, None));
    let mut source = source_meta;
    if exact_source_url.is_some() {
        source.page_url = exact_source_url;
        source.page_title = exact_source_title.or(source.page_title);
        source.domain = exact_source_domain.or(source.domain);
    }

    NewClipboardItem {
        kind: if is_link { "link" } else { "text" }.to_string(),
        url: is_link.then(|| content.clone()),
        domain: parsed_domain.or_else(|| source.domain.clone()),
        title: source.page_title.clone(),
        content,
        source,
        hash,
        blob: None,
        thumb: None,
    }
}

fn image_to_pngs(image: &ImageData<'_>) -> Result<(Vec<u8>, Vec<u8>), String> {
    let rgba = RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.to_vec(),
    )
    .ok_or_else(|| "Clipboard image has invalid dimensions".to_string())?;

    let dynamic = DynamicImage::ImageRgba8(rgba);
    let mut original = Cursor::new(Vec::new());
    dynamic
        .write_to(&mut original, ImageFormat::Png)
        .map_err(|err| err.to_string())?;

    let thumb_image = dynamic.resize(180, 128, FilterType::Triangle);
    let mut thumb = Cursor::new(Vec::new());
    thumb_image
        .write_to(&mut thumb, ImageFormat::Png)
        .map_err(|err| err.to_string())?;

    Ok((original.into_inner(), thumb.into_inner()))
}

fn hash_bytes(kind: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn hash_image(image: &ImageData<'_>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"image");
    hasher.update([0]);
    hasher.update(image.width.to_le_bytes());
    hasher.update(image.height.to_le_bytes());
    hasher.update(image.bytes.as_ref());
    format!("{:x}", hasher.finalize())
}
