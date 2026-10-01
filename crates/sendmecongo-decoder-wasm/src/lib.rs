//! rxing compiled to wasm32 for the miniprogram decoder worker.
//!
//! Plain C ABI on purpose: `wasm-bindgen`'s generated JS glue assumes a
//! browser/Node module system that the WeChat worker environment does not
//! provide. The ABI is four functions and one memory, so the hand-written
//! JS shim (in sendmecongo-mp/workers/decoder/) stays under a page.
//!
//! Result layout, little-endian, at the pointer `decode_rgba` returns:
//!
//! ```text
//! [0..4)  u32 payload length (0 = nothing decoded)
//! [4..)   payload bytes (the QR byte segment, same bytes jsQR's binaryData holds)
//! ```
//!
//! The output buffer is reused across calls; the JS side copies the slice out
//! before the next call.

use std::cell::RefCell;
use std::sync::Once;

thread_local! {
    // RGBA frame written by the JS side before every decode.
    static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    // [len u32 LE][bytes] scratch, reused across calls.
    static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    // Decoder hints survive across frames, like sendmecongo-recv's Scanner.
    static HINTS: RefCell<rxing::DecodeHints> = RefCell::new(new_hints());
    // Last panic message (wasm traps are opaque; this makes them readable).
    static PANIC: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let msg = format!("{info}");
            PANIC.with(|p| {
                let mut p = p.borrow_mut();
                p.clear();
                p.extend_from_slice(msg.as_bytes());
            });
        }));
    });
}

/// Pointer to `[u32 LE len][message bytes]` of the last panic, or len=0.
#[no_mangle]
pub extern "C" fn last_panic() -> *mut u8 {
    PANIC.with(|p| {
        let mut p = p.borrow_mut();
        let body = p.split_off(0);
        p.clear();
        p.extend_from_slice(&(body.len() as u32).to_le_bytes());
        p.extend_from_slice(&body);
        p.as_mut_ptr()
    })
}

fn new_hints() -> rxing::DecodeHints {
    // TryHarder is off: on this project's footage it costs ~3x the time for the
    // same symbols (sendmecongo-recv/scan.rs, measured over 47 real frames).
    let mut hints = rxing::DecodeHints::default();
    hints.TryHarder = Some(false);
    hints.PureBarcode = Some(false);
    hints
}

#[no_mangle]
pub extern "C" fn alloc_input(len: usize) -> *mut u8 {
    install_panic_hook();
    INPUT.with(|input| {
        let mut input = input.borrow_mut();
        input.clear();
        input.resize(len, 0);
        input.as_mut_ptr()
    })
}

/// Decode one RGBA8 frame. Returns the output pointer; read `[u32 LE]` there
/// for the payload length, then the payload bytes that follow. Never returns
/// null; a length of zero means "no code".
#[no_mangle]
pub extern "C" fn decode_rgba(width: u32, height: u32) -> *mut u8 {
    install_panic_hook();
    OUTPUT.with(|output| {
        let mut output = output.borrow_mut();
        output.clear();
        output.extend_from_slice(&0u32.to_le_bytes());

        let (payload_len, payload) = INPUT.with(|input| {
            let input = input.borrow();
            decode_inner(&input, width, height)
        });
        if let Some(payload) = payload {
            let len = payload.len() as u32;
            output.clear();
            output.extend_from_slice(&len.to_le_bytes());
            output.extend_from_slice(&payload);
        }
        let _ = payload_len;
        output.as_mut_ptr()
    })
}

/// Native entry point (examples/tests): same path `decode_rgba` takes.
pub fn decode_dump(rgba: &[u8], width: u32, height: u32) -> (usize, Option<Vec<u8>>) {
    decode_inner(rgba, width, height)
}

fn decode_inner(rgba: &[u8], width: u32, height: u32) -> (usize, Option<Vec<u8>>) {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 {
        return (0, None);
    }
    let Some(pixels) = w.checked_mul(h) else { return (0, None) };
    if rgba.len() < pixels * 4 {
        return (0, None);
    }

    // BT.601 luma, integer: 0.299R + 0.587G + 0.114B. rxing's luma readers take
    // one byte per pixel; converting here keeps 4x weight off the JS side.
    let mut luma = vec![0u8; pixels];
    for (px, out) in rgba.chunks_exact(4).zip(luma.iter_mut()) {
        *out = ((px[0] as u32 * 77 + px[1] as u32 * 151 + px[2] as u32 * 28 + 128) >> 8) as u8;
    }

    HINTS.with(|hints| {
        let mut hints = hints.borrow_mut();
        match rxing::helpers::detect_multiple_in_luma_with_hints(luma, width, height, &mut hints) {
            Ok(results) => {
                let first = results.iter().find_map(|r| {
                    let bytes = r.getRawBytes();
                    if bytes.is_empty() { None } else { Some(bytes.to_vec()) }
                });
                (results.len(), first)
            }
            Err(_) => (0, None),
        }
    })
}
