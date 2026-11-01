//! Lossless WebP through libwebp's C API, one thread.

use std::ffi::c_int;
use std::mem::MaybeUninit;

use libwebp_sys as w;

pub fn encode(rgba: &[u8], width: usize, height: usize, z: c_int, exact: bool) -> Vec<u8> {
    unsafe {
        let mut cfg = MaybeUninit::<w::WebPConfig>::zeroed().assume_init();
        assert!(
            w::WebPConfigInitInternal(
                &mut cfg,
                w::WebPPreset::WEBP_PRESET_DEFAULT,
                75.0,
                w::WEBP_ENCODER_ABI_VERSION as c_int
            ) != 0
        );
        assert!(w::WebPConfigLosslessPreset(&mut cfg, z) != 0);
        cfg.exact = exact as c_int;
        cfg.thread_level = 0;
        let mut pic = MaybeUninit::<w::WebPPicture>::zeroed().assume_init();
        assert!(w::WebPPictureInit(&mut pic));
        pic.use_argb = 1;
        pic.width = width as c_int;
        pic.height = height as c_int;
        assert!(w::WebPPictureImportRGBA(&mut pic, rgba.as_ptr(), (width * 4) as c_int) != 0);
        let mut mem = MaybeUninit::<w::WebPMemoryWriter>::zeroed().assume_init();
        w::WebPMemoryWriterInit(&mut mem);
        pic.writer = Some(w::WebPMemoryWrite);
        pic.custom_ptr = (&raw mut mem).cast();
        assert!(w::WebPEncode(&cfg, &mut pic) != 0, "WebPEncode: {:?}", pic.error_code);
        let out = std::slice::from_raw_parts(mem.mem, mem.size).to_vec();
        w::WebPPictureFree(&mut pic);
        w::WebPMemoryWriterClear(&mut mem);
        out
    }
}

pub fn decode(blob: &[u8]) -> Vec<u8> {
    unsafe {
        let (mut width, mut height) = (0, 0);
        let p = w::WebPDecodeRGBA(blob.as_ptr(), blob.len(), &mut width, &mut height);
        assert!(!p.is_null());
        let out = std::slice::from_raw_parts(p, (width * height * 4) as usize).to_vec();
        w::WebPFree(p.cast());
        out
    }
}
