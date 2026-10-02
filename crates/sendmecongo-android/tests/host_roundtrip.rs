//! Host-side closed loop, no Android needed: sendmecongo frames rendered to QR
//! PNGs, decoded back through the Android crate's Scanner + the shared
//! Receiver. This is the gate that must be green before any camera code ships.

use sendmecongo_android::scanner::Scanner;
use sendmecongo_core::{preset, qr, Received, Receiver, Sender};

/// Deterministic pseudo-random payload: not compressible to nothing, unlike
/// `vec![0u8; n]`, and stable across runs so failures are reproducible.
fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn scan_one(scanner: &mut Scanner, frame: &[u8], p: preset::Preset, out: &mut Vec<Vec<u8>>) {
    let png = qr::png(frame, p.version, p.ec, 4).expect("render QR");
    let img = image::load_from_memory(&png).expect("qr::png output decodes");
    let gray = img.to_luma8();
    let (w, h) = gray.dimensions();
    out.clear();
    scanner.scan(gray.into_raw(), w, h, out);
}

/// Feed whole cycles of the stream until the fountain code finishes, then
/// require byte-identical reconstruction. On-device the stream loops too, so
/// "second pass mops up" mirrors reality rather than padding the test.
fn roundtrip(name: &str, data: &[u8], p: preset::Preset) {
    let sender = Sender::new(name, data, p.symbol_size(), p.repair_pct).expect("sender encodes");
    let mut scanner = Scanner::new();
    let mut receiver = Receiver::new();
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut received: Option<Received> = None;

    'passes: for _ in 0..3 {
        for frame in sender.frames() {
            scan_one(&mut scanner, frame, p, &mut out);
            for payload in &out {
                match receiver.push(payload) {
                    Ok(Some(r)) => {
                        received = Some(r);
                        break 'passes;
                    }
                    // A frame CRC failure would poison a real session; the test
                    // must see it, not average it away.
                    Err(e) => panic!("receiver rejected a frame: {e}"),
                    Ok(None) => {}
                }
            }
        }
    }

    let got = received.expect("fountain code must finish within 3 cycles");
    assert_eq!(got.name, name);
    assert_eq!(got.data, data, "byte-identical reconstruction required");
}

#[test]
fn mp40_roundtrips_200kb() {
    roundtrip("report-v0.5.0.bin", &payload(200_000), preset::MP40);
}

#[test]
fn mp30_roundtrips_200kb() {
    roundtrip("report-v0.5.0.bin", &payload(200_000), preset::MP30);
}

/// mp15 is a single small code per frame; the scanner's band fallback must not
/// corrupt a one-code stream either.
#[test]
fn mp15_roundtrips_20kb() {
    roundtrip("small.txt", &payload(20_000), preset::MP15);
}

/// 提速档：20/30 符号/s 不改变单帧编码，但 repair 40 的 OTI/K' 路径必须同样闭环。
#[test]
fn mp40_20_roundtrips_200kb() {
    roundtrip("report-v0.5.0.bin", &payload(200_000), preset::MP40_20);
}

#[test]
fn mp40_30_roundtrips_200kb() {
    roundtrip("report-v0.5.0.bin", &payload(200_000), preset::MP40_30);
}
