//! Minimal battery reader via the kernel sysfs `power_supply` interface
//! (`/sys/class/power_supply`). This is dependency-free, so it builds and runs
//! on a normal glibc Linux as well as on Android/Termux (Bionic), where crates
//! that assume a glibc userspace fail to compile. Returns `None` when no
//! battery is present (e.g. a desktop), so callers can skip the indicator.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryState {
    Unknown,
    Charging,
    Discharging,
    Full,
}

pub type BatteryReading = (u8, BatteryState);

/// Return `(percent, state)` for the first battery found, or `None` when no
/// battery is present.
pub fn read_battery() -> Option<BatteryReading> {
    let base = std::path::Path::new("/sys/class/power_supply");
    let read_dir = match std::fs::read_dir(base) {
        Ok(d) => d,
        Err(_) => return None,
    };
    for entry in read_dir.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let is_battery = std::fs::read_to_string(dir.join("type"))
            .map(|s| s.trim() == "Battery")
            .unwrap_or(false)
            || dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.eq_ignore_ascii_case("battery") || n.starts_with("BAT"))
                .unwrap_or(false);
        if !is_battery {
            continue;
        }
        let pct = read_percent(&dir)?;
        let state = read_state(&dir, pct);
        return Some((pct, state));
    }
    None
}

fn read_percent(dir: &std::path::Path) -> Option<u8> {
    // Prefer the ready-made 0..100 `capacity` file.
    if let Ok(s) = std::fs::read_to_string(dir.join("capacity")) {
        if let Ok(v) = s.trim().parse::<i32>() {
            return Some(v.clamp(0, 100) as u8);
        }
    }
    // Otherwise compute from charge_now / charge_full (or energy_*).
    let now = read_i64(dir, "charge_now").or_else(|| read_i64(dir, "energy_now"));
    let full = read_i64(dir, "charge_full").or_else(|| read_i64(dir, "energy_full"));
    match (now, full) {
        (Some(n), Some(f)) if f > 0 => Some(((n * 100 / f).clamp(0, 100) as u8)),
        _ => None,
    }
}

fn read_i64(dir: &std::path::Path, name: &str) -> Option<i64> {
    std::fs::read_to_string(dir.join(name))
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()
}

fn read_state(dir: &std::path::Path, pct: u8) -> BatteryState {
    let s = std::fs::read_to_string(dir.join("status")).unwrap_or_default();
    match s.trim() {
        "Charging" => BatteryState::Charging,
        "Discharging" => BatteryState::Discharging,
        "Full" => BatteryState::Full,
        "Not charging" => {
            if pct >= 100 {
                BatteryState::Full
            } else {
                BatteryState::Discharging
            }
        }
        _ => BatteryState::Unknown,
    }
}
