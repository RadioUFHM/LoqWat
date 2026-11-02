//! Lightweight resource measurements for diagnostics (never includes content).

/// Peak resident memory of this process in bytes, if available.
#[cfg(unix)]
pub fn peak_rss_bytes() -> Option<u64> {
    // SAFETY: getrusage only writes into the provided struct.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return None;
    }
    let max = usage.ru_maxrss as u64;
    // macOS reports bytes; Linux reports kilobytes.
    if cfg!(target_os = "macos") {
        Some(max)
    } else {
        Some(max * 1024)
    }
}

#[cfg(not(unix))]
pub fn peak_rss_bytes() -> Option<u64> {
    None
}

pub fn format_mb(bytes: Option<u64>) -> String {
    bytes.map_or_else(
        || "n/a".into(),
        |b| format!("{:.0} MB", b as f64 / 1_048_576.0),
    )
}
