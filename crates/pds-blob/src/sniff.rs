//! The media type of a blob from its first bytes (`docs/SPEC.md` §6.4).
//!
//! The Reference asks the `file-type` package and stores what it answers in
//! place of the declared type. This module knows the signatures below, which
//! cover the image, video and audio types the vendored lexicons accept, and
//! a few containers. For any other content it answers `None` and the
//! declared type stands, where `file-type` knows about 150 more formats.

/// The longest prefix [`sniff`] reads.
pub const SNIFF_LEN: usize = 64;

fn at(bytes: &[u8], offset: usize, magic: &[u8]) -> bool {
    bytes.get(offset..offset + magic.len()) == Some(magic)
}

/// The type of an ISO base media file from its major brand.
fn iso_brand(brand: &[u8]) -> &'static str {
    match brand {
        b"avif" | b"avis" => "image/avif",
        b"heic" | b"heix" => "image/heic",
        b"mif1" => "image/heif",
        b"qt  " => "video/quicktime",
        b"M4A " => "audio/x-m4a",
        b"M4V " => "video/x-m4v",
        b"3gp4" | b"3gp5" | b"3gp6" => "video/3gpp",
        _ => "video/mp4",
    }
}

/// The media type that the first bytes of a file show, or `None`.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if at(bytes, 0, &[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if at(bytes, 0, &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some("image/png");
    }
    if at(bytes, 0, b"GIF87a") || at(bytes, 0, b"GIF89a") {
        return Some("image/gif");
    }
    if at(bytes, 0, b"RIFF") {
        if at(bytes, 8, b"WEBP") {
            return Some("image/webp");
        }
        if at(bytes, 8, b"WAVE") {
            return Some("audio/wav");
        }
        if at(bytes, 8, b"AVI ") {
            return Some("video/vnd.avi");
        }
    }
    if at(bytes, 0, b"BM") {
        return Some("image/bmp");
    }
    if at(bytes, 4, b"ftyp") {
        return bytes.get(8..12).map(iso_brand);
    }
    if at(bytes, 0, &[0x1a, 0x45, 0xdf, 0xa3]) {
        // EBML: Matroska or WebM. `file-type` reads the DocType; a WebM
        // file names it in the header.
        let webm = bytes.windows(4).any(|window| window == b"webm");
        return Some(if webm { "video/webm" } else { "video/matroska" });
    }
    if at(bytes, 0, b"OggS") {
        return Some("audio/ogg");
    }
    if at(bytes, 0, b"fLaC") {
        return Some("audio/flac");
    }
    if at(bytes, 0, b"ID3") || at(bytes, 0, &[0xff, 0xfb]) || at(bytes, 0, &[0xff, 0xf3]) {
        return Some("audio/mpeg");
    }
    if at(bytes, 0, b"%PDF") {
        return Some("application/pdf");
    }
    if at(bytes, 0, &[b'P', b'K', 0x03, 0x04]) {
        return Some("application/zip");
    }
    if at(bytes, 0, &[0x1f, 0x8b, 0x08]) {
        return Some("application/gzip");
    }
    if at(bytes, 0, &[0x00, b'a', b's', b'm']) {
        return Some("application/wasm");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_content_names_its_type() {
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0, 0, 0]), Some("image/jpeg"));
        assert_eq!(
            sniff(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0]),
            Some("image/png")
        );
        assert_eq!(sniff(b"GIF89a\x01\x00"), Some("image/gif"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"\0\0\0\x20ftypisom\0\0"), Some("video/mp4"));
        assert_eq!(sniff(b"\0\0\0\x20ftypavif\0\0"), Some("image/avif"));
        assert_eq!(sniff(b"plain text"), None);
        assert_eq!(sniff(b""), None);
    }
}
