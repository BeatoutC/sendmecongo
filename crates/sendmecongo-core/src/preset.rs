//! Channel presets. `fps` is symbols per second across all lanes combined;
//! `hold` is how many display refreshes a single code stays stable for (60 Hz screen).

use crate::frame::OVERHEAD;
use crate::qr;
use qrcode::EcLevel;

#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub name: &'static str,
    pub version: i16,
    pub ec: EcLevel,
    pub fps: f64,
    pub lanes: u8,
    pub hold_refreshes: u32,
    pub repair_pct: u32,
}

pub const ROBUST: Preset = Preset {
    name: "robust",
    version: 15,
    ec: EcLevel::L,
    fps: 10.0,
    lanes: 1,
    hold_refreshes: 6,
    repair_pct: 35,
};

pub const BALANCED: Preset = Preset {
    name: "balanced",
    version: 20,
    ec: EcLevel::L,
    fps: 15.0,
    lanes: 1,
    hold_refreshes: 4,
    repair_pct: 25,
};

pub const TURBO15: Preset = Preset {
    name: "turbo15",
    version: 30,
    ec: EcLevel::L,
    fps: 15.0,
    lanes: 1,
    hold_refreshes: 4,
    repair_pct: 20,
};

pub const TURBO30: Preset = Preset {
    name: "turbo30",
    version: 30,
    ec: EcLevel::L,
    fps: 30.0,
    lanes: 1,
    hold_refreshes: 2,
    repair_pct: 20,
};

pub const TURBO60: Preset = Preset {
    name: "turbo60",
    version: 30,
    ec: EcLevel::L,
    fps: 60.0,
    lanes: 2,
    hold_refreshes: 2,
    repair_pct: 20,
};

pub const MEGABIT: Preset = Preset {
    name: "megabit",
    version: 40,
    ec: EcLevel::L,
    fps: 60.0,
    lanes: 2,
    hold_refreshes: 2,
    repair_pct: 20,
};

pub const ALL: [Preset; 6] = [ROBUST, BALANCED, TURBO15, TURBO30, TURBO60, MEGABIT];

pub fn by_name(name: &str) -> Option<Preset> {
    ALL.iter().copied().find(|p| p.name == name)
}

impl Preset {
    /// Usable symbol payload: QR byte capacity minus per-frame overhead.
    pub fn symbol_size(&self) -> u16 {
        (qr::capacity(self.version, self.ec).saturating_sub(OVERHEAD)) as u16
    }

    /// Nominal optical payload rate in bytes/s (before camera loss).
    pub fn nominal_bps(&self) -> f64 {
        self.fps * self.symbol_size() as f64
    }

    pub fn nominal_mbps(&self) -> f64 {
        self.nominal_bps() * 8.0 / 1_000_000.0
    }
}
