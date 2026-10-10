//! One stored tile or item for `map_handler`: an `If-None-Match` is answered from metadata alone; full replies
//! read the bytes with their version (`MapStorage::read_grid_versioned`). Blocking.

use bm_storage::{Compression, GridKey, ItemKey, MapStorage, Tile, Version};
use http::HeaderValue;

use crate::client_unpack;
use crate::content_type::OCTET_STREAM;
use crate::encoding::{Accepted, Encoded, body_coding, encode, packed_coding};
use crate::transcode;
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
    /// Client-side unpacking ([`client_unpack`]): `None` when the server does not offer it, else whether this
    /// request asked for it.
    pub unpacks: Option<bool>,
}

pub(crate) enum Reply {
    NotModified(HeaderValue),
    Found {
        encoded: Encoded,
        etag: Option<HeaderValue>,
        /// Set when the reply's form depended on `Accept`: its `Content-Type`.
        negotiated: Option<&'static str>,
    },
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
    let packed = match &req.target {
        Target::Tile(GridKey::Hires, tile) if storage.packs_hires() => {
            Some((*tile, packed_coding(req.gz_url, &req.accepted)))
        }
        _ => None,
    };
    let unpacks = req.unpacks == Some(true) && !req.gz_url;
    let coding_id = |coding: Compression| (coding != Compression::None).then(|| coding.id());
    let tag = |v: Version, c| {
        etag(v, match packed {
            Some((_, coding)) => coding_id(coding),
            None => body_coding(c, req.is_png, req.gz_url, &req.accepted),
        })
    };
    if let Some(inm) = &req.if_none_match {
        let (version, compression) = match &req.target {
            Target::Tile(grid, tile) => (storage.grid_version(*grid, *tile)?, storage.grid_compression(*grid)),
            Target::Item(item) => (storage.item_version(item)?, storage.item_compression(item)),
        };
        // a client that unpacks holds whichever form the tile had: the packed body, or PRBM if it is stored raw
        let body_tag = packed.filter(|_| unpacks).map(|(_, coding)| client_unpack::etag_coding(coding));
        let tags = version.into_iter().flat_map(|v| [body_tag.map(|c| etag(v, Some(c))), Some(tag(v, compression))]);
        if let Some(t) = tags.flatten().find(|t| inm.matches(t)) {
            return Ok(Reply::NotModified(t));
        }
    }
    if let Some((tile, coding)) = packed {
        let Some((blob, version)) = storage.read_hires_packed(tile)? else { return Ok(Reply::Missing) };
        let version = version.filter(|_| req.etags);
        let content_encoding = coding_id(coding).filter(|_| !req.gz_url);
        let negotiated = |media_type| req.unpacks.map(|_| media_type);
        if unpacks && bm_storage::packed_model_frame(&blob).is_some() {
            return Ok(Reply::Found {
                encoded: Encoded { body: transcode::packed_body(blob, coding)?, content_encoding },
                etag: version.map(|v| etag(v, Some(client_unpack::etag_coding(coding)))),
                negotiated: negotiated(client_unpack::MEDIA_TYPE),
            });
        }
        let encoded = Encoded { body: transcode::packed(&blob, coding)?, content_encoding };
        return Ok(Reply::Found { encoded, etag: version.map(|v| tag(v, coding)), negotiated: negotiated(OCTET_STREAM) });
    }
    let read = match (&req.target, req.etags) {
        (Target::Tile(grid, tile), true) => storage.read_grid_versioned(*grid, *tile)?,
        (Target::Item(item), true) => storage.read_item_versioned(item)?,
        (Target::Tile(grid, tile), false) => storage.read_grid(*grid, *tile)?.map(|s| (s, None)),
        (Target::Item(item), false) => storage.read_item(item)?.map(|s| (s, None)),
    };
    let Some((stored, version)) = read else { return Ok(Reply::Missing) };
    let etag = version.map(|v| tag(v, stored.compression));
    Ok(Reply::Found { encoded: encode(stored, req.is_png, req.gz_url, &req.accepted)?, etag, negotiated: None })
}
