use std::fs;

pub fn pss_mb(pid: u32) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Pss:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

fn parent_of(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

pub fn web_processes(root: u32) -> usize {
    let Ok(entries) = fs::read_dir("/proc") else { return 0 };
    entries
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim() == "WebKitWebProces"))
        .filter(|&pid| {
            let mut current = pid;
            while let Some(parent) = parent_of(current) {
                if parent == root {
                    return true;
                }
                if parent <= 1 {
                    break;
                }
                current = parent;
            }
            false
        })
        .count()
}

pub fn process_age_ms() -> u128 {
    let uptime: f64 =
        fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next()?.parse().ok()).unwrap_or(0.0);
    let started_ticks: f64 = fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|s| s.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok())
        .unwrap_or(0.0);
    ((uptime - started_ticks / 100.0) * 1000.0).max(0.0) as u128
}
