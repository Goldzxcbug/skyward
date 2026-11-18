//! Early recovery diagnostics, kept in RAM and the kernel log.
//! They are available over adb even when the display cannot start.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

/// A RAM-backed log; never use reserved boot-control partition space.
const FILE: &str = "/skyward/run/boot.log";

const LEN: usize = 48 * 1024;

/// The lines kept, from the first still worth reading to the last written.
static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Keep a bounded diagnostic log in RAM as well as in the kernel log.
pub fn line(text: impl AsRef<str>) {
    let text = text.as_ref();
    let _ = skyward_sys::kmsg(text);
    let Ok(mut lines) = LINES.lock() else { return };
    lines.push(text.to_string());
    // Keep the newest lines that fit, the oldest going first.
    let mut total: usize = lines.iter().map(|l| l.len() + 1).sum();
    let mut first = 0;
    while total > LEN && first + 1 < lines.len() {
        total -= lines[first].len() + 1;
        first += 1;
    }
    if first > 0 {
        lines.drain(..first);
    }
    let mut text = String::with_capacity(total);
    for l in lines.iter() {
        text.push_str(l);
        text.push('\n');
    }
    if std::fs::create_dir_all(skyward_sys::RUN).is_err() {
        return;
    }
    let Ok(mut file) = OpenOptions::new().write(true).create(true).truncate(true).open(FILE) else { return };
    let _ = file.write_all(text.as_bytes());
}

/// What the recovery found where it started: the kernel's own command line,
/// whether the display driver is in, and where the display's files are. A
/// screen that stays dark says nothing; this does.
pub fn snapshot(what: &str) {
    line(format!("== {what}"));
    let modules = std::fs::read_to_string("/proc/modules").unwrap_or_default();
    line(format!("msm_drm loaded: {}", modules.lines().any(|l| l.starts_with("msm_drm "))));
    line(format!("dri: {}", listing("/dev/dri")));
    line(format!("backlight: {}", listing("/sys/class/backlight")));
    match std::fs::read_to_string("/sys/module/msm_drm/parameters/dsi_display0") {
        Ok(v) => line(format!("dsi_display0: {}", v.trim())),
        Err(e) => line(format!("dsi_display0: {e}")),
    }
    line(format!("inputs: {}", inputs()));
    line(format!("hbp: {}", ["/dev/hbp0", "/dev/hbp_core"].iter().map(|p| format!("{p} {}", if std::path::Path::new(p).exists() { "there" } else { "missing" })).collect::<Vec<_>>().join(", ")));
    line(format!("touch programs: {}", programs("touch")));
}

/// Each device under /dev/input by its name, so a panel that reports nothing
/// can be told from one that is not there.
fn inputs() -> String {
    let mut found = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/dev/input") {
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("event") {
                continue;
            }
            let label = std::fs::read_to_string(format!("/sys/class/input/{name}/device/name"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "?".into());
            found.push(format!("{name}={label}"));
        }
    }
    found.sort();
    found.join(" ")
}

/// The running programs whose command line holds `word`, as their paths.
fn programs(word: &str) -> String {
    let mut found = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/proc") {
        for entry in dir.flatten() {
            let path = entry.path().join("cmdline");
            let Ok(raw) = std::fs::read(&path) else { continue };
            let text = raw.split(|b| *b == 0).next().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
            if !text.is_empty() && text.contains(word) {
                found.push(text);
            }
        }
    }
    found.sort();
    found.join(", ")
}

/// The names in a folder, in one line, or why they cannot be read.
fn listing(dir: &str) -> String {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect::<Vec<_>>().join(" "),
        Err(e) => format!("{e}"),
    }
}
