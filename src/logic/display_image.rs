//! Owner: A5. Swift source: `../immagine/Immagine/Thumbnails/DisplayImage.swift`.
//! Spec: `../immagine/ImmagineTests/` -> DisplayImageTests.
//! Port the tests first (same names in snake_case), then implement until `cargo test --lib logic::display_image` is green.
//!
//! The Swift version decodes with ImageIO and copies into Core Animation's native layout. Neither
//! exists in the logic layer, so the codec is a trait (`ImageCodec`) that the browser implements
//! (`createImageBitmap` or a WASM decoder). What stays here: the decode-size rules, the downsample
//! limit, and the check that a decoded bitmap is well formed.

/// Pixel dimensions of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

impl PixelSize {
    /// The longer of the two sides.
    pub fn long_side(self) -> u32 {
        self.width.max(self.height)
    }
}

/// Decoded pixels: 8-bit RGBA, 4 bytes per pixel, row-major, top row first. Alpha is straight
/// (not premultiplied) as the codec produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub size: PixelSize,
    pub rgba: Vec<u8>,
}

/// The seam to a platform image decoder (ImageIO on iOS, the browser on web).
pub trait ImageCodec {
    /// Pixel size of the first frame, read from the header alone. `None` for data that is no image.
    fn probe(&self, data: &[u8]) -> Option<PixelSize>;

    /// First frame decoded to RGBA8, with orientation applied. With `max_long_side`, the frame is
    /// scaled down so that its long side is that value; it is never scaled up.
    fn decode(&self, data: &[u8], max_long_side: Option<u32>) -> Option<Bitmap>;
}

/// The size an image gets when its long side is limited to `max_pixel`, aspect ratio kept.
/// Never enlarges; a limit of 0 counts as 1.
pub fn target_size(size: PixelSize, max_pixel: u32) -> PixelSize {
    let long = size.long_side();
    let limit = max_pixel.max(1);
    if long == 0 || limit >= long {
        return size;
    }
    let scale = limit as f64 / long as f64;
    let scaled = |side: u32| 1.max((side as f64 * scale).round() as u32);
    if size.width >= size.height {
        PixelSize { width: limit, height: scaled(size.height) }
    } else {
        PixelSize { width: scaled(size.width), height: limit }
    }
}

/// A whole image file (thumbnail or preview as the server renders it), first frame, full size.
pub fn decode<C: ImageCodec>(codec: &C, data: &[u8]) -> Option<Bitmap> {
    codec.decode(data, None).and_then(well_formed)
}

/// An image file decoded straight to `max_pixel` on its long side. A limit above the image's own
/// size is clamped to it, so the codec never gets asked to enlarge (the Swift code logged an
/// ImageIO error for each such request).
pub fn downsample<C: ImageCodec>(codec: &C, data: &[u8], max_pixel: u32) -> Option<Bitmap> {
    let limit = match codec.probe(data) {
        Some(size) => max_pixel.min(size.long_side()),
        None => max_pixel,
    };
    codec.decode(data, Some(limit)).and_then(well_formed)
}

/// A bitmap is usable only with a non-empty size and exactly 4 bytes per pixel.
fn well_formed(bitmap: Bitmap) -> Option<Bitmap> {
    let (w, h) = (bitmap.size.width as usize, bitmap.size.height as usize);
    (w > 0 && h > 0 && bitmap.rgba.len() == w * h * 4).then_some(bitmap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Stand-in for ImageIO: a "file" is `width:u16 BE, height:u16 BE, then width*height RGBA
    /// bytes`. Anything else is not an image. Scales with nearest-neighbour when asked.
    #[derive(Default)]
    struct ToyCodec {
        /// Every `max_long_side` the codec was asked for.
        asked: RefCell<Vec<Option<u32>>>,
    }

    impl ToyCodec {
        fn parse(data: &[u8]) -> Option<Bitmap> {
            if data.len() < 4 {
                return None;
            }
            let size = PixelSize { width: u16::from_be_bytes([data[0], data[1]]) as u32, height: u16::from_be_bytes([data[2], data[3]]) as u32 };
            let rgba = data[4..].to_vec();
            (rgba.len() == (size.width * size.height * 4) as usize).then_some(Bitmap { size, rgba })
        }
    }

    impl ImageCodec for ToyCodec {
        fn probe(&self, data: &[u8]) -> Option<PixelSize> {
            Self::parse(data).map(|b| b.size)
        }

        fn decode(&self, data: &[u8], max_long_side: Option<u32>) -> Option<Bitmap> {
            self.asked.borrow_mut().push(max_long_side);
            let full = Self::parse(data)?;
            let Some(limit) = max_long_side else { return Some(full) };
            let target = target_size(full.size, limit);
            let mut rgba = Vec::with_capacity((target.width * target.height * 4) as usize);
            for y in 0..target.height {
                for x in 0..target.width {
                    let sx = (x as u64 * full.size.width as u64 / target.width as u64) as usize;
                    let sy = (y as u64 * full.size.height as u64 / target.height as u64) as usize;
                    let i = (sy * full.size.width as usize + sx) * 4;
                    rgba.extend_from_slice(&full.rgba[i..i + 4]);
                }
            }
            Some(Bitmap { size: target, rgba })
        }
    }

    fn encode(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&(width as u16).to_be_bytes());
        data.extend_from_slice(&(height as u16).to_be_bytes());
        for _ in 0..width * height {
            data.extend_from_slice(&color);
        }
        data
    }

    fn square(side: u32) -> PixelSize {
        PixelSize { width: side, height: side }
    }

    #[test]
    fn decode_keeps_the_pixels_of_the_codec() {
        // Port of testJPEGDecodesToNativeBitmap: the codec's red comes out red.
        let data = encode(16, 16, [255, 0, 0, 255]);
        let image = decode(&ToyCodec::default(), &data).unwrap();
        assert_eq!(image.size, square(16));
        assert_eq!(&image.rgba[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn decode_keeps_transparency() {
        // Port of testPNGKeepsTransparency: a half-transparent blue keeps alpha 128.
        let data = encode(16, 16, [0, 0, 255, 128]);
        let image = decode(&ToyCodec::default(), &data).unwrap();
        assert_eq!(image.rgba[3], 128);
    }

    #[test]
    fn decode_asks_the_codec_for_full_size() {
        let codec = ToyCodec::default();
        decode(&codec, &encode(4, 4, [0, 0, 0, 255])).unwrap();
        assert_eq!(*codec.asked.borrow(), vec![None]);
    }

    #[test]
    fn downsample_scales_long_side_to_max_pixel() {
        // Port of testDownsampleScalesAndStaysNative (the native-layout half is a codec concern).
        let data = encode(400, 400, [0, 255, 0, 255]);
        let image = downsample(&ToyCodec::default(), &data, 100).unwrap();
        assert_eq!(image.size, square(100));
        assert_eq!(image.rgba.len(), 100 * 100 * 4);
    }

    #[test]
    fn downsample_above_the_image_size_keeps_the_image() {
        // Port of testDownsampleAboveTheImageSizeKeepsTheImage: the codec is asked for 120, not 1356.
        let codec = ToyCodec::default();
        let data = encode(120, 120, [0, 255, 0, 255]);
        let image = downsample(&codec, &data, 1356).unwrap();
        assert_eq!(image.size, square(120));
        assert_eq!(*codec.asked.borrow(), vec![Some(120)]);
    }

    #[test]
    fn broken_data_gives_none() {
        // Port of testBrokenDataGivesNil: an error body is no image.
        let codec = ToyCodec::default();
        assert!(decode(&codec, br#"{"message":"Not found"}"#).is_none(), "an error body is no image");
        assert!(decode(&codec, &[]).is_none());
        assert!(downsample(&codec, br#"{"message":"Not found"}"#, 100).is_none());
    }

    #[test]
    fn bitmap_without_four_bytes_per_pixel_is_rejected() {
        struct Short;
        impl ImageCodec for Short {
            fn probe(&self, _: &[u8]) -> Option<PixelSize> {
                Some(square(2))
            }
            fn decode(&self, _: &[u8], _: Option<u32>) -> Option<Bitmap> {
                Some(Bitmap { size: square(2), rgba: vec![0; 3] })
            }
        }
        assert!(decode(&Short, &[]).is_none());
        assert!(downsample(&Short, &[], 2).is_none());
    }

    #[test]
    fn target_size_keeps_aspect_ratio() {
        assert_eq!(target_size(PixelSize { width: 400, height: 200 }, 100), PixelSize { width: 100, height: 50 });
        assert_eq!(target_size(PixelSize { width: 200, height: 400 }, 100), PixelSize { width: 50, height: 100 });
        assert_eq!(target_size(PixelSize { width: 1000, height: 1 }, 100), PixelSize { width: 100, height: 1 }, "a sliver never vanishes");
    }

    #[test]
    fn target_size_never_enlarges_and_treats_zero_as_one() {
        assert_eq!(target_size(square(120), 1356), square(120));
        assert_eq!(target_size(square(120), 120), square(120));
        assert_eq!(target_size(square(120), 0), square(1));
    }
}
