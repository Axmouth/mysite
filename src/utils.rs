use std::path::Path as FilePath;

use rand::{Rng, distr::Alphanumeric};

pub(crate) fn is_http_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

pub(crate) fn random_token() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

pub(crate) fn normalized_slug(slug: &str, title: &str) -> String {
    let source = if slug.trim().is_empty() { title } else { slug };
    let mut output = String::new();
    let mut previous_dash = false;
    for character in source.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash && !output.is_empty() {
            output.push('-');
            previous_dash = true;
        }
    }
    output.trim_end_matches('-').to_string()
}

pub(crate) fn allowed_image_extension(file_name: &str) -> Option<&'static str> {
    match FilePath::new(file_name)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        _ => None,
    }
}

pub(crate) fn image_bytes_match_extension(extension: &str, bytes: &[u8]) -> bool {
    match extension {
        "jpg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "webp" => {
            bytes.starts_with(b"RIFF") && bytes.get(8..12).is_some_and(|kind| kind == b"WEBP")
        }
        _ => false,
    }
}

pub(crate) fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
