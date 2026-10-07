//! One stored tile or item for `map_handler`: an `If-None-Match` is answered from metadata alone; full replies
//! read the bytes with their version (`MapStorage::read_grid_versioned`). Blocking.

use bm_storage::{GridKey, ItemKey, MapStorage, Tile, Version};
use http::HeaderValue;

use crate::encoding::{Accepted, Encoded, body_coding, encode};
use crate::validators::{IfNoneMatch, etag};

pub(crate) enum Target {
    Tile(GridKey, Tile),
    Item(ItemKey),
}

pub(crate) struct Request {
    pub target: Target,
    pub is_png: bool,
    pub gz_url: bool,
    pub accepted: Accepted,
    pub if_none_match: Option<IfNoneMatch>,
    /// Send `ETag` on full replies.
    pub etags: bool,
}

pub(crate) enum Reply {
    NotModified(HeaderValue),
    Found { encoded: Encoded, etag: Option<HeaderValue> },
    Missing,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Storage(#[from] bm_storage::Error),
    #[error(transparent)]
    Encode(#[from] bm_compress::Error),
}

pub(crate) fn serve(storage: &dyn MapStorage, req: Request) -> Result<Reply, Error> {
    let tag = |v: Version, c| etag(v, body_coding(c, req.is_png, req.gz_url, &req.accepted));
    if let Some(inm) = &req.if_none_match {
        let (version, compression) = match &req.target {
            Target::Tile(grid, tile) => (storage.grid_version(*grid, *tile)?, storage.grid_compression(*grid)),
            Target::Item(item) => (storage.item_version(item)?, storage.item_compression(item)),
        };
        if let Some(t) = version.map(|v| tag(v, compression)).filter(|t| inm.matches(t)) {
            return Ok(Reply::NotModified(t));
        }
    }
    let read = match (&req.target, req.etags) {
        (Target::Tile(grid, tile), true) => storage.read_grid_versioned(*grid, *tile)?,
        (Target::Item(item), true) => storage.read_item_versioned(item)?,
        (Target::Tile(grid, tile), false) => storage.read_grid(*grid, *tile)?.map(|s| (s, None)),
        (Target::Item(item), false) => storage.read_item(item)?.map(|s| (s, None)),
    };
    let Some((stored, version)) = read else { return Ok(Reply::Missing) };
    let etag = version.map(|v| tag(v, stored.compression));
    Ok(Reply::Found { encoded: encode(stored, req.is_png, req.gz_url, &req.accepted)?, etag })
}
