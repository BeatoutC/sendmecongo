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

pub const MP15: Preset = Preset {
    name: "mp15",
    version: 15,
    ec: EcLevel::L,
    fps: 20.0,
    lanes: 1,
    hold_refreshes: 3,
    repair_pct: 30,
};

pub const MP20: Preset = Preset {
    name: "mp20",
    version: 20,
    ec: EcLevel::L,
    fps: 20.0,
    lanes: 1,
    hold_refreshes: 3,
    repair_pct: 30,
};

/// 手机大负载档：v0.1 PoC 实测手机 JS/WASM 解码只有 ~3-5 帧/s（jsQR 489ms/帧），
/// 「快解码多扫小码」路线数学上不可达 → 换「慢解码扫大码」：显示 10fps（60Hz 下
/// hold 6 = 每码稳定 100ms，30fps 相机的 33ms 曝光必然完整落在一个码内，零叠影），
/// 让解码器以自己的节奏采样， fountain 码（repair 30%）吸收丢帧。V30/V40 每码
/// 装得多，3-5 次解码/s 也有 ~5-10 KB/s。
pub const MP30: Preset = Preset {
    name: "mp30",
    version: 30,
    ec: EcLevel::L,
    fps: 10.0,
    lanes: 1,
    hold_refreshes: 6,
    repair_pct: 30,
};

pub const MP40: Preset = Preset {
    name: "mp40",
    version: 40,
    ec: EcLevel::L,
    fps: 10.0,
    lanes: 1,
    hold_refreshes: 6,
    repair_pct: 30,
};

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

/// 手机档（mp15/mp20）：给手机 ~30fps 相机对屏实时扫描用，与桌面录像档的取舍不同。
/// 关键不是快，而是：模块更大（V15/V20 才能在相机帧里保住每模块 >=5px）+
/// 每码停留 3 个刷新（20fps），让 ~33ms 的相机曝光大概率完整落在一个码的显示期内，
/// 避免拍到换帧叠影。高 repair 对冲手机端的对焦/抖动丢帧。
pub const ALL: [Preset; 10] = [
    MP15, MP20, MP30, MP40, ROBUST, BALANCED, TURBO15, TURBO30, TURBO60, MEGABIT,
];

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
