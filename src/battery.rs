//! Battery reader with universal fallbacks:
//!
//! 1. Linux kernel sysfs  `/sys/class/power_supply` (standard Linux).
//! 2. Termux API command `termux-battery-status` (Android/Termux).
//!
//! Returns `None` when no battery is present (desktop, no Termux:API), so
//! callers can skip the indicator.  No external Rust dependencies required.

use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryState {
    Unknown,
    Charging,
    Discharging,
    Full,
}

pub type BatteryReading = (u8, BatteryState);

/// Try every known battery reader in order and return the first success.
pub fn read_battery() -> Option<BatteryReading> {
    read_sysfs().or_else(read_termux)
}

// ---------------------------------------------------------------------------
// 1. Linux kernel sysfs  (/sys/class/power_supply)
// ---------------------------------------------------------------------------
fn read_sysfs() -> Option<BatteryReading> {
    let base = std::path::Path::new("/sys/class/power_supply");
    let read_dir = std::fs::read_dir(base).ok()?;
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
    if let Ok(s) = std::fs::read_to_string(dir.join("capacity")) {
        if let Ok(v) = s.trim().parse::<i32>() {
            return Some(v.clamp(0, 100) as u8);
        }
    }
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
            if pct >= 100 { BatteryState::Full } else { BatteryState::Discharging }
        }
        _ => BatteryState::Unknown,
    }
}

// ---------------------------------------------------------------------------
// 2. Termux API  (termux-battery-status)
// ---------------------------------------------------------------------------
/// Spawn `termux-battery-status` and parse its JSON output.
fn read_termux() -> Option<BatteryReading> {
    let out = Command::new("termux-battery-status")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // Expected JSON:  {"percentage": 87, "status": "Discharging", ...}
    let raw = std::str::from_utf8(&out.stdout).ok()?;
    parse_termux_json(raw)
}

fn parse_termux_json(raw: &str) -> Option<BatteryReading> {
    let pct = {
        let key_pos = raw.find("\"percentage\"")?;
        let after_key = &raw[key_pos..];
        let colon = after_key.find(':')?;
        let rest = after_key[colon + 1..].trim_start();
        let end = rest.find(|c: char| !c.is_ascii_digit())?;
        rest[..end].parse::<u8>().ok()?
    };
    let state = {
        let key_pos = raw.find("\"status\"")?;
        let after_key = &raw[key_pos..];
        let colon = after_key.find(':')?;
        let rest = after_key[colon + 1..].trim_start();
        let quoted = rest.strip_prefix('"')?;
        let end = quoted.find('"')?;
        termux_status_to_state(&quoted[..end])
    };
    Some((pct.clamp(0, 100), state))
}

fn termux_status_to_state(s: &str) -> BatteryState {
    match s {
        "CHARGING" | "PLUGGED_AC" | "PLUGGED_USB" | "PLUGGED_WIRELESS" => BatteryState::Charging,
        "DISCHARGING" | "UNPLUGGED" => BatteryState::Discharging,
        "FULL" => BatteryState::Full,
        _ => BatteryState::Unknown,
    }
}
