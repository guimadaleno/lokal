//! What an image is to a model that sees: its kind and size, read from its
//! first bytes, the data URL it's sent as, and what it takes of the context.

/// The kinds of image a model server takes. Ollama 0.40.2 refuses a GIF.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Png,
    Jpeg,
    Webp,
}

#[derive(Debug, PartialEq)]
pub struct Image {
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
}

/// How far into a file its size is looked for. A JPEG's size comes after
/// its metadata, which is rarely more than a few kilobytes.
const SIZE_WITHIN: usize = 1024 * 1024;

/// What each image adds besides its pixels: the markers around it.
const TOKENS_AROUND: u64 = 64;
/// The pixels a token covers: a 28-pixel grid (Qwen 2.5, Mistral Small),
/// finer than Qwen 3.8's 32, so the count stays on the high side.
const PIXELS_PER_TOKEN: u64 = 28 * 28;

/// The image a file holds, known by its first bytes, with its size. None
/// for anything else, or an image whose size can't be read.
pub fn read(bytes: &[u8]) -> Option<Image> {
    let bytes = &bytes[..bytes.len().min(SIZE_WITHIN)];
    let (kind, (width, height)) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        (Kind::Png, png_size(bytes)?)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        (Kind::Jpeg, jpeg_size(bytes)?)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        (Kind::Webp, webp_size(bytes)?)
    } else {
        return None;
    };
    (width > 0 && height > 0).then_some(Image {
        kind,
        width,
        height,
    })
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Png => "PNG",
            Kind::Jpeg => "JPEG",
            Kind::Webp => "WebP",
        }
    }

    fn media_type(self) -> &'static str {
        match self {
            Kind::Png => "image/png",
            Kind::Jpeg => "image/jpeg",
            Kind::Webp => "image/webp",
        }
    }
}

impl Image {
    /// About what the image takes of the context, on the high side.
    pub fn tokens(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) / PIXELS_PER_TOKEN + TOKENS_AROUND
    }
}

/// The image as the chat API takes it: a `data:` URL.
pub fn data_url(kind: Kind, bytes: &[u8]) -> String {
    format!("data:{};base64,{}", kind.media_type(), base64(bytes))
}

/// What an image sent as a data URL takes of the context, read from the
/// start of its data. None for a URL that isn't one.
pub fn tokens_in_url(url: &str) -> Option<u64> {
    let data = url.strip_prefix("data:image/")?.split_once(";base64,")?.1;
    // Whole groups of four characters, enough for SIZE_WITHIN bytes.
    let length = data.len().min(SIZE_WITHIN.div_ceil(3) * 4) / 4 * 4;
    Some(read(&unbase64(&data[..length])?)?.tokens())
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((big_endian(bytes, 16, 4)?, big_endian(bytes, 20, 4)?))
}

/// From the first frame marker: what comes before it are segments that
/// say their own length.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        match marker {
            // Fill bytes before a marker.
            0xFF => at += 1,
            // Markers that stand alone, with no length.
            0x01 | 0xD0..=0xD8 => at += 2,
            // A frame, but for the three that aren't (Huffman and
            // arithmetic tables, a JPEG extension).
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                return Some((big_endian(bytes, at + 7, 2)?, big_endian(bytes, at + 5, 2)?));
            }
            // The image data starts with no frame before it.
            0xDA => return None,
            _ => at += 2 + big_endian(bytes, at + 2, 2)? as usize,
        }
    }
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        // Lossy: 14 bits each, after the frame's start code.
        b"VP8 " if bytes.get(23..26)? == [0x9D, 0x01, 0x2A] => Some((
            little_endian(bytes, 26, 2)? & 0x3FFF,
            little_endian(bytes, 28, 2)? & 0x3FFF,
        )),
        // Lossless: 14 bits each, less one, after its signature.
        b"VP8L" if *bytes.get(20)? == 0x2F => {
            let bits = little_endian(bytes, 21, 4)?;
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        // Extended: 24 bits each, less one.
        b"VP8X" => Some((
            little_endian(bytes, 24, 3)? + 1,
            little_endian(bytes, 27, 3)? + 1,
        )),
        _ => None,
    }
}

fn big_endian(bytes: &[u8], at: usize, length: usize) -> Option<u32> {
    let part = bytes.get(at..at + length)?;
    Some(
        part.iter()
            .fold(0, |number, &byte| number << 8 | u32::from(byte)),
    )
}

fn little_endian(bytes: &[u8], at: usize, length: usize) -> Option<u32> {
    let part = bytes.get(at..at + length)?;
    Some(
        part.iter()
            .rev()
            .fold(0, |number, &byte| number << 8 | u32::from(byte)),
    )
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let number = group
            .iter()
            .enumerate()
            .fold(0u32, |number, (index, &byte)| {
                number | u32::from(byte) << (16 - 8 * index)
            });
        for index in 0..4 {
            if index <= group.len() {
                text.push(ALPHABET[(number >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// Base64 in whole groups of four, as `base64` writes it.
fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    for group in text.as_bytes().chunks(4) {
        let padding = group.iter().rev().take_while(|&&byte| byte == b'=').count();
        if group.len() != 4 || padding > 2 {
            return None;
        }
        let mut number = 0u32;
        for &byte in &group[..4 - padding] {
            let value = ALPHABET.iter().position(|&letter| letter == byte)?;
            number = number << 6 | value as u32;
        }
        number <<= 6 * padding;
        bytes.extend_from_slice(&number.to_be_bytes()[1..4 - padding]);
    }
    Some(bytes)
}

/// The start of a PNG of that size, enough for `read`: for tests.
#[cfg(test)]
pub fn png_start(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend(b"\x08\x06\0\0\0rest");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        png_start(width, height)
    }

    /// A JPEG start with an EXIF segment and the tables before the frame.
    fn jpeg(width: u16, height: u16, frame: u8) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x08];
        bytes.extend(b"Exif\0\0");
        bytes.extend([0xFF, 0xC4, 0x00, 0x04, 0x00, 0x00]);
        bytes.extend([0xFF, 0xFF, frame, 0x00, 0x11, 0x08]);
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([0x03, 0x01, 0x22, 0x00]);
        bytes
    }

    fn webp(chunk: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
        bytes.extend(chunk);
        bytes.extend([0; 4]);
        bytes.extend(body);
        bytes
    }

    fn size(bytes: &[u8]) -> Option<(Kind, u32, u32)> {
        read(bytes).map(|image| (image.kind, image.width, image.height))
    }

    #[test]
    fn reads_a_pngs_size() {
        assert_eq!(size(&png(1920, 1080)), Some((Kind::Png, 1920, 1080)));
        assert_eq!(size(&png(0, 1080)), None);
        assert_eq!(size(&png(1920, 1080)[..20]), None);
    }

    #[test]
    fn reads_a_jpegs_size_past_its_metadata() {
        assert_eq!(size(&jpeg(640, 480, 0xC0)), Some((Kind::Jpeg, 640, 480)));
        // Progressive.
        assert_eq!(
            size(&jpeg(4032, 3024, 0xC2)),
            Some((Kind::Jpeg, 4032, 3024))
        );
        // Image data with no frame before it.
        assert_eq!(size(&jpeg(640, 480, 0xDA)), None);
        assert_eq!(size(&jpeg(640, 480, 0xC0)[..12]), None);
    }

    #[test]
    fn reads_a_webps_size_in_its_three_forms() {
        let mut lossy = vec![0; 3];
        lossy.extend([0x9D, 0x01, 0x2A]);
        lossy.extend(640u16.to_le_bytes());
        lossy.extend(480u16.to_le_bytes());
        assert_eq!(size(&webp(b"VP8 ", &lossy)), Some((Kind::Webp, 640, 480)));
        let bits: u32 = (640 - 1) | (480 - 1) << 14;
        let mut lossless = vec![0x2F];
        lossless.extend(bits.to_le_bytes());
        assert_eq!(
            size(&webp(b"VP8L", &lossless)),
            Some((Kind::Webp, 640, 480))
        );
        let mut extended = vec![0; 4];
        extended.extend(&(8000u32 - 1).to_le_bytes()[..3]);
        extended.extend(&(4000u32 - 1).to_le_bytes()[..3]);
        assert_eq!(
            size(&webp(b"VP8X", &extended)),
            Some((Kind::Webp, 8000, 4000))
        );
    }

    #[test]
    fn anything_else_isnt_an_image() {
        assert_eq!(size(b"GIF89a\x40\x00\x40\x00"), None);
        assert_eq!(size(b"plain text"), None);
        assert_eq!(size(b""), None);
    }

    #[test]
    fn the_estimate_is_on_the_high_side_of_what_was_measured() {
        // Qwen 3.8 on Ollama 0.40.2: 64×64 took 16, 640×480 307, 1920×1080 2,047.
        for (width, height, measured) in [(64, 64, 16), (640, 480, 307), (1920, 1080, 2047)] {
            let image = Image {
                kind: Kind::Png,
                width,
                height,
            };
            assert!(image.tokens() >= measured, "{width}×{height}");
            assert!(
                image.tokens() < measured * 2 + TOKENS_AROUND,
                "{width}×{height}"
            );
        }
    }

    #[test]
    fn base64_is_rfc_4648s() {
        for (bytes, text) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(bytes.as_bytes()), text);
            assert_eq!(unbase64(text).unwrap(), bytes.as_bytes());
        }
        let bytes: Vec<u8> = (0..=255u8).collect();
        assert_eq!(unbase64(&base64(&bytes)).unwrap(), bytes);
        assert_eq!(unbase64("abc"), None);
        assert_eq!(unbase64("a!cd"), None);
    }

    #[test]
    fn the_url_says_what_the_image_takes() {
        let bytes = png(1920, 1080);
        let url = data_url(Kind::Png, &bytes);
        assert!(url.starts_with("data:image/png;base64,iVBORw0KGgo"));
        assert_eq!(
            tokens_in_url(&url),
            read(&bytes).map(|image| image.tokens())
        );
        assert_eq!(tokens_in_url("https://example.com/a.png"), None);
        assert_eq!(
            tokens_in_url("data:image/png;base64,bm90IGFuIGltYWdl"),
            None
        );
    }
}
