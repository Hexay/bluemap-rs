//! The browser half of client-side unpacking (docs/18): the exports `crates/bm-web/client/unpack.js` calls to turn
//! the model body of a hires tile into PRBM. One buffer pair, one caller: JavaScript fills the input, unpacks, and
//! copies the output before its next call.
//!
//! The built module is checked in; after changing the unpacker rebuild it with `py -3 tools/build_wasm.py`.

use std::sync::{Mutex, PoisonError};

use bm_format::compact::BodyUnpacker;

#[derive(Default)]
struct State {
    unpacker: BodyUnpacker,
    input: Vec<u8>,
    output: Vec<u8>,
}

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

/// Unpacks the input buffer into the output buffer: the PRBM length, or -1 for a corrupt body.
#[unsafe(no_mangle)]
pub extern "C" fn bmq3_unpack() -> i32 {
    with_state(|s| match s.unpacker.unpack_into(&s.input, &mut s.output) {
        Ok(()) => i32::try_from(s.output.len()).unwrap_or(-1),
        Err(_) => -1,
    })
}

/// The output buffer of the last [`bmq3_unpack`].
#[unsafe(no_mangle)]
pub extern "C" fn bmq3_output() -> *const u8 {
    with_state(|s| s.output.as_ptr())
}

#[cfg(test)]
mod tests {
    use bm_format::compact::CompactCodec;
    use bm_format::prbm::TileModel;

    use super::*;

    #[test]
    fn exports_unpack_a_model_body() {
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
        let (mut prbm, mut body) = (Vec::new(), Vec::new());
        model.write_prbm(&mut prbm).unwrap();
        assert!(CompactCodec::default().body_into(&prbm, None, &mut body));

        let call = |body: &[u8]| {
            // SAFETY: bmq3_input returns a buffer of the requested length
            unsafe { std::slice::from_raw_parts_mut(bmq3_input(body.len()), body.len()) }.copy_from_slice(body);
            bmq3_unpack()
        };
        let len = call(&body);
        assert_eq!(len, prbm.len() as i32);
        // SAFETY: bmq3_output points at the `len` bytes bmq3_unpack just wrote
        assert_eq!(unsafe { std::slice::from_raw_parts(bmq3_output(), len as usize) }, prbm);
        assert_eq!(call(&body[..body.len() - 3]), -1);
    }
}
