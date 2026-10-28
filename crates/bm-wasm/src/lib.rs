//! The browser half of client-side unpacking (docs/18): the exports `crates/bm-web/client/unpack.js` calls to turn
//! a hires tile's model body, or its zstd frame, into PRBM. One buffer pair, one caller: JavaScript fills the
//! input, unpacks, and copies the output before its next call.
//!
//! The built module is checked in; after changing the unpacker rebuild it with `py -3 tools/build_wasm.py`.

use std::sync::{Mutex, PoisonError};

use bm_format::compact::BodyUnpacker;
use zstd_sys::{ZSTD_DCtx, ZSTD_createDCtx, ZSTD_decompressDCtx, ZSTD_getFrameContentSize, ZSTD_isError};

/// Largest model body a frame may declare; a real one is well under 1 MB.
const MAX_BODY: u64 = 1 << 28;

#[derive(Default)]
struct State {
    unpacker: BodyUnpacker,
    input: Vec<u8>,
    body: Vec<u8>,
    output: Vec<u8>,
    dctx: Option<Dctx>,
}

struct Dctx(*mut ZSTD_DCtx);
// SAFETY: only used under STATE's lock
unsafe impl Send for Dctx {}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(STATE.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(State::default))
}

/// Sizes the input buffer to `len` bytes and returns it for the caller to fill.
#[unsafe(no_mangle)]
pub extern "C" fn bmq3_input(len: usize) -> *mut u8 {
    with_state(|s| {
        s.input.clear();
        s.input.resize(len, 0);
        s.input.as_mut_ptr()
    })
}

/// Unpacks the input buffer (a model body) into the output buffer: the PRBM length, or -1 for a corrupt body.
#[unsafe(no_mangle)]
pub extern "C" fn bmq3_unpack() -> i32 {
    with_state(|s| output_len(s.unpacker.unpack_into(&s.input, &mut s.output).is_ok(), s))
}

/// [`bmq3_unpack`] for an input that is the model body's zstd frame.
#[unsafe(no_mangle)]
pub extern "C" fn bmq3_unpack_frame() -> i32 {
    with_state(|s| {
        let mut body = std::mem::take(&mut s.body);
        let unpacked = decode_frame(s, &mut body) && s.unpacker.unpack_into(&body, &mut s.output).is_ok();
        s.body = body;
        output_len(unpacked, s)
    })
}

fn output_len(unpacked: bool, s: &State) -> i32 {
    if unpacked { i32::try_from(s.output.len()).unwrap_or(-1) } else { -1 }
}

/// The input frame into `body`; false for a corrupt frame or one without a content size.
fn decode_frame(s: &mut State, body: &mut Vec<u8>) -> bool {
    let input = &s.input;
    // SAFETY: zstd reads `input` and writes at most `body.capacity()` bytes into `body`
    unsafe {
        // unknown and error sizes are above MAX_BODY too
        let size = ZSTD_getFrameContentSize(input.as_ptr().cast(), input.len());
        if size > MAX_BODY {
            return false;
        }
        body.clear();
        body.reserve(size as usize);
        let dctx = s.dctx.get_or_insert_with(|| Dctx(ZSTD_createDCtx())).0;
        if dctx.is_null() {
            return false;
        }
        let n =
            ZSTD_decompressDCtx(dctx, body.as_mut_ptr().cast(), body.capacity(), input.as_ptr().cast(), input.len());
        if ZSTD_isError(n) != 0 || n as u64 != size {
            return false;
        }
        body.set_len(n);
    }
    true
}

/// The output buffer of the last unpack.
#[unsafe(no_mangle)]
pub extern "C" fn bmq3_output() -> *const u8 {
    with_state(|s| s.output.as_ptr())
}

#[cfg(test)]
mod tests {
    use bm_format::compact::CompactCodec;
    use bm_format::prbm::TileModel;

    use super::*;

    fn model_prbm() -> Vec<u8> {
        let mut model = TileModel::default();
        for tri in [[[0., 64., 0.], [0., 64., 1.], [1., 64., 1.]], [[0., 64., 0.], [1., 64., 1.], [1., 64., 0.]]] {
            tri.iter().for_each(|p| model.position.extend(p));
            model.color.extend([0.5, 0.5, 1.]);
            model.sunlight.push(15);
            model.blocklight.push(2);
            model.material.push(7);
        }
        model.uv.extend([0., 0., 0., 1., 1., 1., 0., 0., 1., 1., 1., 0.]);
        model.ao.extend([1., 1., 0.5, 1., 0.5, 0.75]);
        let mut prbm = Vec::new();
        model.write_prbm(&mut prbm).unwrap();
        prbm
    }

    /// `unpack` on `input` as JavaScript calls it; the output if it succeeded.
    fn call(unpack: extern "C" fn() -> i32, input: &[u8]) -> Option<Vec<u8>> {
        // SAFETY: bmq3_input returns a buffer of the requested length
        unsafe { std::slice::from_raw_parts_mut(bmq3_input(input.len()), input.len()) }.copy_from_slice(input);
        let len = usize::try_from(unpack()).ok()?;
        // SAFETY: bmq3_output points at the `len` bytes the unpack just wrote
        Some(unsafe { std::slice::from_raw_parts(bmq3_output(), len) }.to_vec())
    }

    #[test]
    fn exports_unpack_a_model_body_and_its_frame() {
        let (prbm, mut body, mut blob) = (model_prbm(), Vec::new(), Vec::new());
        let mut codec = CompactCodec::default();
        assert!(codec.body_into(&prbm, None, &mut body));
        assert_eq!(call(bmq3_unpack, &body), Some(prbm.clone()));
        assert_eq!(call(bmq3_unpack, &body[..body.len() - 3]), None);

        codec.encode_into(&prbm, None, &mut blob).unwrap();
        let frame = bm_format::compact::model_frame(&blob).unwrap();
        for _ in 0..2 {
            assert_eq!(call(bmq3_unpack_frame, frame), Some(prbm.clone()));
        }
        assert_eq!(call(bmq3_unpack_frame, &frame[..frame.len() - 1]), None);
        assert_eq!(call(bmq3_unpack_frame, &body), None, "a body is not a frame");
        // a frame header declaring 4 GiB of content (single segment, 8-byte size), no blocks
        let huge = [0x28, 0xb5, 0x2f, 0xfd, 0xe0, 0, 0, 0, 0, 1, 0, 0, 0];
        assert_eq!(call(bmq3_unpack_frame, &huge), None);
    }
}
