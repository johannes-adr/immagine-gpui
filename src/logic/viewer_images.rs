//! Owner: A5. Swift source: `../immagine/Immagine/UI/Viewer/ViewerImages.swift`.
//! Spec: `../immagine/ImmagineTests/` -> ViewerImagesTests.
//! Port the tests first (same names in snake_case), then implement until `cargo test --lib logic::viewer_images` is green.
//!
//! The decisions behind the viewer's sharp image, without any I/O: how many pixels a photo needs
//! on screen, whether a prewarm has to decode or download anything, where the full-resolution
//! image comes from and when the server's fullsize rendition is the fallback. The decoding and
//! downloading itself is done by the caller through `display_image` and the client.

/// Headroom for the first zoom: the screen-sized decode is 15 % larger than the fitted photo.
pub const ZOOM_HEADROOM: f64 = 1.15;
/// Largest bitmap the GPU is asked for, per side.
pub const GPU_TEXTURE_LIMIT: u32 = 16384;
/// Pixel budget of one full-resolution bitmap (64 megapixels, 256 MB as RGBA).
pub const FULL_PIXEL_BUDGET: f64 = 64_000_000.0;

/// Long side, in pixels, at which a photo fills the screen (aspect fit), with headroom for the
/// first zoom. `ratio` is width / height of the photo (`None` when unknown); `screen` is in
/// points and `scale` is the device's pixels per point.
pub fn max_pixel(ratio: Option<f64>, screen: (f64, f64), scale: f64) -> u32 {
    let (width, height) = (screen.0 * scale, screen.1 * scale);
    let ratio = match ratio {
        Some(r) if r > 0.0 && width > 0.0 && height > 0.0 => r,
        _ => return width.max(height).max(1.0) as u32,
    };
    let fitted_width = width.min(height * ratio);
    let fitted_height = height.min(width / ratio);
    (fitted_width.max(fitted_height) * ZOOM_HEADROOM).ceil() as u32
}

/// Long side at which zooming shows a photo: its own, all of its pixels, unless one bitmap would
/// pass the GPU's texture limit or the 64 MP budget. Only giant panoramas are scaled down.
pub fn full_pixel(width: u32, height: u32) -> u32 {
    let long = width.max(height);
    let short = width.min(height);
    if short == 0 {
        return 0;
    }
    let within_budget = (FULL_PIXEL_BUDGET * long as f64 / short as f64).sqrt() as u32;
    long.min(GPU_TEXTURE_LIMIT).min(within_budget)
}

/// What a prewarm (a grid cell was touched) has to do for one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharpLoad {
    /// Already in memory, or nothing can be fetched without a token.
    Nothing,
    /// Decode the offline original at this long side.
    DecodeLocal { max_pixel: u32 },
    /// Download the server's preview.
    FetchPreview,
}

/// Decides the prewarm. `has_local_file` is the offline original's presence; it only counts for
/// images (videos never use their local file here). `local_cached` and `preview_cached` come
/// from the rendition cache.
pub fn sharp_load(is_image: bool, has_local_file: bool, local_cached: bool, preview_cached: bool, has_token: bool, max_pixel: u32) -> SharpLoad {
    if is_image && has_local_file {
        if local_cached {
            SharpLoad::Nothing
        } else {
            SharpLoad::DecodeLocal { max_pixel }
        }
    } else if preview_cached || !has_token {
        SharpLoad::Nothing
    } else {
        SharpLoad::FetchPreview
    }
}

/// Where a full-resolution image comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FullSource {
    /// The offline original, decoded from its local file.
    OfflineOriginal,
    /// The original downloaded from the server.
    ServerOriginal,
    /// The server's "fullsize" rendition, the fallback for originals the codec cannot read.
    ServerFullsize,
}

impl FullSource {
    /// Name used in the log line.
    pub fn label(self) -> &'static str {
        match self {
            FullSource::OfflineOriginal => "offline original",
            FullSource::ServerOriginal => "server original",
            FullSource::ServerFullsize => "server fullsize",
        }
    }
}

/// The first source to try: the offline original if there is one, else the server's original when
/// a session exists. `None` when neither is available.
pub fn full_source(has_local_file: bool, has_server: bool) -> Option<FullSource> {
    if has_local_file {
        Some(FullSource::OfflineOriginal)
    } else if has_server {
        Some(FullSource::ServerOriginal)
    } else {
        None
    }
}

/// After a source produced no decodable image: the server's original falls back to the fullsize
/// rendition once, unless the request was cancelled. Immich's fullsize is only the preview again
/// unless the server generates full sizes, so it stands in only for originals the codec cannot read.
pub fn fullsize_fallback(source: FullSource, decoded: bool, cancelled: bool) -> Option<FullSource> {
    (source == FullSource::ServerOriginal && !decoded && !cancelled).then_some(FullSource::ServerFullsize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::display_image::{self, Bitmap, ImageCodec, PixelSize};

    #[test]
    fn decode_size_is_what_the_screen_shows() {
        // Port of testDecodeSizeIsWhatTheScreenShows. iPhone 18 Pro: 402 × 874 pt at 3×.
        let screen = (402.0, 874.0);
        // 4:3 landscape fills the width: 1206 px wide (+15 % for the first zoom).
        assert_eq!(max_pixel(Some(4.0 / 3.0), screen, 3.0), 1387);
        // 3:4 portrait: 1206 × 1608 px.
        assert_eq!(max_pixel(Some(3.0 / 4.0), screen, 3.0), 1850);
        // A screenshot of another iPhone: a hair wider than this screen, so it fills the width.
        assert_eq!(max_pixel(Some(1179.0 / 2556.0), screen, 3.0), 3007);
        // Taller than the screen: it fills the height (2622 px).
        assert_eq!(max_pixel(Some(0.4), screen, 3.0), 3016);
        assert_eq!(max_pixel(None, screen, 3.0), 2622, "unknown shape: the screen");
    }

    #[test]
    fn decode_size_falls_back_to_the_screen_for_bad_input() {
        assert_eq!(max_pixel(Some(0.0), (402.0, 874.0), 3.0), 2622, "ratio must be positive");
        assert_eq!(max_pixel(Some(1.0), (0.0, 0.0), 3.0), 1, "no screen: at least one pixel");
    }

    #[test]
    fn full_resolution_keeps_every_pixel() {
        // Port of testFullResolutionKeepsEveryPixel.
        assert_eq!(full_pixel(4032, 3024), 4032, "12 MP");
        assert_eq!(full_pixel(6048, 8064), 8064, "48 MP (the old cap was 6000)");
        assert_eq!(full_pixel(5648, 8472), 8472, "the largest test photo");
        // 96 MP: scaled into 64 MP (9797 × 6531).
        assert_eq!(full_pixel(12000, 8000), 9797);
        assert_eq!(full_pixel(20000, 2000), 16384, "panorama: the GPU's texture limit");
        assert_eq!(full_pixel(0, 100), 0);
    }

    #[test]
    fn offline_original_zooms_to_all_pixels() {
        // Port of testOfflineOriginalZoomsToAllPixels, decode half: a 6400 × 4000 original is
        // decoded at its own size, not at the screen size.
        struct Blank;
        impl ImageCodec for Blank {
            fn probe(&self, _: &[u8]) -> Option<PixelSize> {
                Some(PixelSize { width: 6400, height: 4000 })
            }
            fn decode(&self, _: &[u8], max_long_side: Option<u32>) -> Option<Bitmap> {
                let original = PixelSize { width: 6400, height: 4000 };
                let size = display_image::target_size(original, max_long_side.unwrap_or(6400));
                Some(Bitmap { size, rgba: vec![0; (size.width * size.height * 4) as usize] })
            }
        }
        let limit = full_pixel(6400, 4000);
        let full = display_image::downsample(&Blank, &[], limit).unwrap();
        assert_eq!(full.size, PixelSize { width: 6400, height: 4000 });
    }

    #[test]
    fn local_original_is_decoded_at_screen_size_unless_cached() {
        // Port of the decode-size rule of testLocalOriginalIsDecodedSmallAndShared.
        assert_eq!(sharp_load(true, true, false, false, true, 1387), SharpLoad::DecodeLocal { max_pixel: 1387 });
        assert_eq!(sharp_load(true, true, true, false, true, 1387), SharpLoad::Nothing, "already decoded");
    }

    #[test]
    fn server_preview_is_fetched_only_when_missing_and_possible() {
        assert_eq!(sharp_load(true, false, false, false, true, 1387), SharpLoad::FetchPreview);
        assert_eq!(sharp_load(true, false, false, true, true, 1387), SharpLoad::Nothing, "preview cached");
        assert_eq!(sharp_load(true, false, false, false, false, 1387), SharpLoad::Nothing, "no token, no request");
    }

    #[test]
    fn video_never_uses_its_local_file() {
        assert_eq!(sharp_load(false, true, false, false, true, 1387), SharpLoad::FetchPreview);
    }

    #[test]
    fn full_source_prefers_the_offline_original() {
        assert_eq!(full_source(true, true), Some(FullSource::OfflineOriginal));
        assert_eq!(full_source(true, false), Some(FullSource::OfflineOriginal));
        assert_eq!(full_source(false, true), Some(FullSource::ServerOriginal));
        assert_eq!(full_source(false, false), None);
    }

    #[test]
    fn server_original_falls_back_to_fullsize_once() {
        assert_eq!(fullsize_fallback(FullSource::ServerOriginal, false, false), Some(FullSource::ServerFullsize), "RAW original ImageIO cannot read");
        assert_eq!(fullsize_fallback(FullSource::ServerOriginal, true, false), None, "decoded: no fallback");
        assert_eq!(fullsize_fallback(FullSource::ServerOriginal, false, true), None, "cancelled: no fallback");
        assert_eq!(fullsize_fallback(FullSource::OfflineOriginal, false, false), None, "a broken local file is not retried");
        assert_eq!(fullsize_fallback(FullSource::ServerFullsize, false, false), None, "never a second fallback");
    }

    #[test]
    fn source_labels_match_the_log_lines() {
        assert_eq!(FullSource::OfflineOriginal.label(), "offline original");
        assert_eq!(FullSource::ServerOriginal.label(), "server original");
        assert_eq!(FullSource::ServerFullsize.label(), "server fullsize");
    }
}
