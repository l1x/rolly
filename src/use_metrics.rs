/// USE metrics collector — reads `/proc/self/stat` and `/proc/self/statm` on Linux.
///
/// Emits `process.cpu.utilization` and `process.memory.usage` as tracing events
/// at a configurable interval. No-op on non-Linux platforms.
use std::time::Duration;

/// Start the USE metrics polling loop.
///
/// Spawns a background tokio task that reads `/proc/self/stat` and `/proc/self/statm`
/// every `interval` and emits metric events via `tracing::info!`.
///
/// On non-Linux platforms this is a no-op.
pub(crate) fn start(interval: Duration) {
    #[cfg(target_os = "linux")]
    {
        tokio::spawn(poll_loop(interval));
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = interval;
        tracing::debug!("pz-o11y: USE metrics disabled (not Linux)");
    }
}

#[cfg(target_os = "linux")]
async fn poll_loop(interval: Duration) {
    use tokio::time;

    let page_size = page_size_bytes();
    let clock_ticks = clock_ticks_per_sec();
    let mut prev_cpu_ticks: Option<u64> = None;
    let mut prev_instant = time::Instant::now();

    let mut ticker = time::interval(interval);
    // First tick fires immediately — skip it to get a baseline.
    ticker.tick().await;

    loop {
        ticker.tick().await;

        let now = time::Instant::now();
        let elapsed_secs = now.duration_since(prev_instant).as_secs_f64();
        prev_instant = now;

        if let Some((utime, stime)) = read_cpu_ticks() {
            let total_ticks = utime + stime;
            if let Some(prev) = prev_cpu_ticks {
                let delta_ticks = total_ticks.saturating_sub(prev);
                let cpu_seconds = delta_ticks as f64 / clock_ticks as f64;
                let utilization = if elapsed_secs > 0.0 {
                    cpu_seconds / elapsed_secs
                } else {
                    0.0
                };

                tracing::info!(
                    metric = "process.cpu.utilization",
                    r#type = "gauge",
                    value = utilization,
                );
            }
            prev_cpu_ticks = Some(total_ticks);
        }

        if let Some(rss_pages) = read_rss_pages() {
            let rss_bytes = rss_pages * page_size;
            tracing::info!(
                metric = "process.memory.usage",
                r#type = "gauge",
                value = rss_bytes,
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn page_size_bytes() -> u64 {
    // SAFETY: sysconf(_SC_PAGESIZE) is always safe to call.
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 }
}

#[cfg(target_os = "linux")]
fn clock_ticks_per_sec() -> u64 {
    // SAFETY: sysconf(_SC_CLK_TCK) is always safe to call.
    unsafe { libc::sysconf(libc::_SC_CLK_TCK) as u64 }
}

/// Read utime (field 14) and stime (field 15) from `/proc/self/stat`.
#[cfg(target_os = "linux")]
fn read_cpu_ticks() -> Option<(u64, u64)> {
    let data = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Fields after the comm field (which is in parens and may contain spaces).
    let after_comm = data.rsplit_once(')')?.1;
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    // After closing paren: field[0]=state, field[1]=ppid, ... field[11]=utime, field[12]=stime
    // (0-indexed from after the comm close-paren)
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some((utime, stime))
}

/// Read RSS (field 2) from `/proc/self/statm`.
#[cfg(target_os = "linux")]
fn read_rss_pages() -> Option<u64> {
    let data = std::fs::read_to_string("/proc/self/statm").ok()?;
    let rss = data.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(rss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_is_noop_on_non_linux() {
        // Should not panic on any platform.
        // On non-Linux, this is a no-op. On Linux without a runtime, it would
        // panic — but tests run with #[tokio::test] when needed.
        #[cfg(not(target_os = "linux"))]
        {
            // Can't call start() without a tokio runtime, but we can verify
            // the function signature exists.
            let _f: fn(Duration) = start;
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn read_cpu_ticks_returns_values() {
        let result = read_cpu_ticks();
        assert!(
            result.is_some(),
            "/proc/self/stat should be readable on Linux"
        );
        let (utime, stime) = result.unwrap();
        // Process has done *some* work to get here.
        assert!(utime + stime > 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn read_rss_pages_returns_nonzero() {
        let result = read_rss_pages();
        assert!(
            result.is_some(),
            "/proc/self/statm should be readable on Linux"
        );
        assert!(
            result.unwrap() > 0,
            "RSS should be > 0 for a running process"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn page_size_is_reasonable() {
        let ps = page_size_bytes();
        assert!(ps >= 4096, "page size should be at least 4096");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn clock_ticks_is_reasonable() {
        let ct = clock_ticks_per_sec();
        assert!(ct >= 100, "clock ticks should be at least 100");
    }
}
