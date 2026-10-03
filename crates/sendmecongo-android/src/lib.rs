//! JNI bridge for the Android receiver: camera luma in, file bytes out.
//!
//! All state lives in Rust ([`Session`]); Kotlin only feeds frames and reads a
//! JSON stats string, so there is exactly one implementation of the protocol.
//! The decode path is the desktop receiver's `Scanner` (a verbatim copy of
//! `sendmecongo-recv/src/scan.rs`) over the shared `Receiver` from
//! `sendmecongo-core` — nothing here re-implements protocol logic.
//!
//! JNI surface (bound to `local.airgate.recv.NativeBridge`):
//!   create() -> handle / destroy(handle)
//!   feed(handle, directByteBuffer, width, height, rowStride) -> stats JSON
//!   finish(handle) -> ByteArray? (file bytes once complete; name via stats)

pub mod scanner;

use jni::objects::{JByteBuffer, JClass, JString};
use jni::sys::{jbyteArray, jint, jlong};
use jni::JNIEnv;
use sendmecongo_core::{frame, Received, Receiver};
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Retired session handles awaiting a safe drop. An in-flight `feed` on the
/// analyzer thread may still hold a handle while `destroy` runs (stop button,
/// rebinding on rotation); freeing immediately would be a use-after-free, so
/// sessions are parked here and only truly dropped once enough newer ones
/// have retired that no live call can still reference them.
static RETIRED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
const RETIRE_KEEP: usize = 8;

fn retire(handle: jlong) {
    let mut list = RETIRED.lock().unwrap_or_else(|e| e.into_inner());
    list.push(handle as usize);
    while list.len() > RETIRE_KEEP {
        let stale = list.remove(0) as *mut Session;
        drop(unsafe { Box::from_raw(stale) });
    }
}

/// Extract a human-readable message from a panic payload.
fn panic_msg(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Stats window: unique-symbol rate and frame rate are computed over this span.
const WINDOW: Duration = Duration::from_secs(3);
/// Below this unique-symbol rate an ETA is noise, not information.
const MIN_ETA_RATE: f64 = 0.5;

struct Session {
    receiver: Receiver,
    scanner: scanner::Scanner,
    /// Stats-only mirror of the receiver's dedup set, so `feed` can bill each
    /// new symbol's payload bytes without poking the receiver's privates.
    seen: HashSet<(u8, u32)>,
    unique_bytes: u64,
    frames: u64,
    codes: u64,
    rejected: u64,
    decode_micros: u64,
    /// K: symbols the object is made of (from the first valid header). The
    /// progress bar denominator, exactly as on desktop (`Counters::target`).
    target: usize,
    /// (t, unique, frames) samples for the rolling rate window.
    window: VecDeque<(Instant, usize, u64)>,
    /// First bytes of the most recent new symbol — the SMQ-magic canary that
    /// proves byte fidelity end to end.
    last_hex: String,
    /// Reusable luma buffer so a 30 fps stream does not allocate 2 MB/frame.
    luma: Vec<u8>,
    done: Option<Received>,
}

impl Session {
    fn new() -> Self {
        Self {
            receiver: Receiver::new(),
            scanner: scanner::Scanner::new(),
            seen: HashSet::new(),
            unique_bytes: 0,
            frames: 0,
            codes: 0,
            rejected: 0,
            decode_micros: 0,
            target: 0,
            window: VecDeque::new(),
            last_hex: String::new(),
            luma: Vec::new(),
            done: None,
        }
    }

    fn stats_json(&self) -> String {
        let unique = self.receiver.frames_used();
        // The window is pruned on every feed; the oldest sample still inside
        // it is the rate baseline.
        let (unique_rate, fps) = match (self.window.front(), self.window.back()) {
            (Some((t0, u0, f0)), Some((t1, u1, f1))) => {
                let dt = t1.duration_since(*t0).as_secs_f64();
                if dt > 0.05 {
                    (
                        (u1.saturating_sub(*u0)) as f64 / dt,
                        (f1.saturating_sub(*f0)) as f64 / dt,
                    )
                } else {
                    (0.0, 0.0)
                }
            }
            _ => (0.0, 0.0),
        };
        let avg_payload = if unique > 0 { self.unique_bytes / unique as u64 } else { 0 };
        let goodput_kbps = unique_rate * avg_payload as f64 / 1024.0;
        let remaining = self.target.saturating_sub(unique);
        let eta_sec = if self.done.is_none()
            && self.target > 0
            && unique_rate >= MIN_ETA_RATE
        {
            Some(remaining as f64 / unique_rate)
        } else {
            None
        };

        let ms_per_frame = if self.frames > 0 {
            self.decode_micros as f64 / self.frames as f64 / 1000.0
        } else {
            0.0
        };

        serde_json::json!({
            "fps": (fps * 10.0).round() / 10.0,
            "ms": (ms_per_frame * 10.0).round() / 10.0,
            "codes": self.codes,
            "unique": unique,
            "target": self.target,
            "goodput": (goodput_kbps * 100.0).round() / 100.0,
            "eta": eta_sec.map(|s| (s * 10.0).round() / 10.0).unwrap_or(-1.0),
            "rejected": self.rejected,
            "last_hex": self.last_hex,
            "done": self.done.is_some(),
            "name": self.done.as_ref().map(|r| r.name.clone()).unwrap_or_default(),
        })
        .to_string()
    }

    fn push_payload(&mut self, payload: &[u8]) {
        self.codes += 1;
        if let Ok((header, _)) = frame::parse(payload) {
            if self.target == 0 && header.symbol_size > 0 {
                self.target = header.object_len.div_ceil(header.symbol_size as u32) as usize;
            }
            if self.seen.insert((header.sbn, header.esi)) {
                self.unique_bytes += payload.len() as u64;
                self.last_hex = payload
                    .iter()
                    .take(16)
                    .map(|b| format!("{b:02x}"))
                    .collect();
            }
        }
        match self.receiver.push(payload) {
            Ok(Some(received)) => self.done = Some(received),
            Ok(None) => {}
            Err(_) => self.rejected += 1,
        }
    }
}

fn session<'a>(handle: jlong) -> Result<&'a mut Session, String> {
    if handle == 0 {
        return Err("bad handle".into());
    }
    Ok(unsafe { &mut *(handle as *mut Session) })
}

#[no_mangle]
pub extern "system" fn Java_local_airgate_recv_NativeBridge_create(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    Box::into_raw(Box::new(Session::new())) as jlong
}

#[no_mangle]
pub extern "system" fn Java_local_airgate_recv_NativeBridge_destroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle != 0 {
        retire(handle);
    }
}

/// Decode one camera frame. `y` must be a direct ByteBuffer of the Y plane;
/// rows may be padded (`row_stride >= width`), the padding is dropped here.
/// Returns the live stats JSON; a malformed call returns `{"error": ...}`.
#[no_mangle]
pub extern "system" fn Java_local_airgate_recv_NativeBridge_feed<'a>(
    env: JNIEnv<'a>,
    _class: JClass,
    handle: jlong,
    y: JByteBuffer,
    width: jint,
    height: jint,
    row_stride: jint,
) -> JString<'a> {
    let reply = feed_inner(&env, handle, &y, width, height, row_stride)
        .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"));
    env.new_string(reply).expect("new_string").into()
}

fn feed_inner(
    env: &JNIEnv,
    handle: jlong,
    y: &JByteBuffer,
    width: jint,
    height: jint,
    row_stride: jint,
) -> Result<String, String> {
    let session = session(handle)?;
    if session.done.is_some() {
        return Ok(session.stats_json());
    }
    let (w, h) = (width as usize, height as usize);
    let stride = if row_stride >= width { row_stride as usize } else { width as usize };
    if w == 0 || h == 0 || stride == 0 {
        return Err(format!("bad dims {w}x{h} stride {stride}"));
    }
    let ptr = env
        .get_direct_buffer_address(y)
        .map_err(|e| format!("not a direct buffer: {e}"))?;
    let cap = env
        .get_direct_buffer_capacity(y)
        .map_err(|e| format!("buffer capacity: {e}"))?;
    let needed = stride * (h - 1) + w;
    if cap < needed {
        return Err(format!("buffer too small: {cap} < {needed}"));
    }
    let src = unsafe { std::slice::from_raw_parts(ptr, cap) };

    // De-pad rows into the reusable buffer: rxing wants a tightly packed plane.
    session.luma.resize(w * h, 0);
    if stride == w {
        session.luma.copy_from_slice(&src[..w * h]);
    } else {
        for row in 0..h {
            let start = row * stride;
            session.luma[row * w..(row + 1) * w]
                .copy_from_slice(&src[start..start + w]);
        }
    }

    let started = Instant::now();
    let mut payloads = Vec::new();
    // A panic inside rxing must never cross the JNI boundary: unwinding through
    // `extern "system"` aborts the whole process (observed as a tombstone in
    // NativeBridge_feed). Surface the panic as a stats error instead.
    let scanned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session
            .scanner
            .scan(std::mem::take(&mut session.luma), w as u32, h as u32, &mut payloads)
    }));
    session.decode_micros += started.elapsed().as_micros() as u64;
    session.frames += 1;
    session.luma = Vec::new(); // scanner took it; re-grow next frame

    if let Err(p) = scanned {
        return Err(format!("decoder panic: {}", panic_msg(&p)));
    }

    let done_now = session.done.is_none();
    for payload in &payloads {
        if done_now {
            session.push_payload(payload);
        } else {
            session.codes += 1;
        }
    }

    let unique = session.receiver.frames_used();
    session.window.push_back((Instant::now(), unique, session.frames));
    while session.window.len() > 2
        && Instant::now().duration_since(session.window.front().unwrap().0) > WINDOW
    {
        session.window.pop_front();
    }
    Ok(session.stats_json())
}

/// The reconstructed file, or null until the fountain code finishes. The file
/// name rides along in the stats JSON ("name") because JNI has no cheap tuple.
#[no_mangle]
pub extern "system" fn Java_local_airgate_recv_NativeBridge_finish(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jbyteArray {
    match session(handle).ok().and_then(|s| s.done.as_ref()) {
        Some(received) => env
            .byte_array_from_slice(&received.data)
            .map(|arr| arr.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}
