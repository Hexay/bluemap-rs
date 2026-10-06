//! BlueMap's two content-type tables, kept apart because they disagree (`.ttf`, `.conf`, unknown suffixes).

/// `FileRequestHandler.toContentType` for webroot files: unknown → `text/plain` (also `.conf`, see #716).
pub fn static_file(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map_or(name, |(_, e)| e);
    match ext {
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "svg" => "image/svg+xml",
        "css" => "text/css",
        "js" => "text/javascript",
        "html" | "htm" | "shtml" => "text/html",
        "xml" => "text/xml",
        _ => "text/plain",
    }
}

/// BlueMapAPI `ContentTypeRegistry.fromFileName` for map data (assets, json items): unknown →
/// `application/octet-stream`.
pub fn map_item(path: &str) -> &'static str {
    let Some(dot) = path.rfind('.') else { return OCTET_STREAM };
    if path.rfind('/').is_some_and(|slash| dot < slash) {
        return OCTET_STREAM;
    }
    match &path[dot + 1..] {
        "txt" => "text/plain",
        "css" => "text/css",
        "csv" => "text/csv",
        "htm" | "html" => "text/html",
        "js" => "text/javascript",
        "xml" => "text/xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "mp3" => "audio/mpeg",
        "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "weba" => "audio/webm",
        "mp4" => "video/mp4",
        "mpeg" => "video/mpeg",
        "webm" => "video/webm",
        "ttf" => "font/ttf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => OCTET_STREAM,
    }
}

pub const OCTET_STREAM: &str = "application/octet-stream";
