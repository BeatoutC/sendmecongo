//! Native debug harness for the same code path the wasm build runs.
//!
//! Usage: cargo run -p sendmecongo-decoder-wasm --example decode_dump -- <file.dump>
//! Dump layout: [u32 LE width][u32 LE height][RGBA8 width*height*4].

use std::io::Read;

fn main() {
    let path = std::env::args().nth(1).expect("usage: decode_dump <file.dump>");
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .expect("read dump");
    if bytes.len() < 8 {
        panic!("dump too short");
    }
    let w = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let h = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let rgba = &bytes[8..];
    println!("frame {w}x{h}, rgba {} bytes", rgba.len());

    let t0 = std::time::Instant::now();
    let (count, payload) = sendmecongo_decoder_wasm::decode_dump(rgba, w, h);
    println!(
        "decoded in {:.1} ms: codes={} payload={}",
        t0.elapsed().as_secs_f32() * 1000.0,
        count,
        payload.as_ref().map(|p| p.len()).unwrap_or(0),
    );
    if let Some(p) = payload {
        println!("head: {}", p.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>());
    }
}
