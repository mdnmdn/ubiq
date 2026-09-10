//! Machine facts for the remote-hosts panel: hostname, OS, triplet and resources.
//!
//! Sampled **periodically, never realtime**: one sampler thread refreshes a cached snapshot
//! every [`SAMPLE_EVERY`], and `stats()` / `HostInfo` serve whatever it last wrote. The
//! coordinator's thread never blocks on a system query — a slow platform API stalls a panel
//! refresh, not every pane's keystrokes.
//!
//! `os` / `arch` / `triplet` are compile-time constants; everything else is best-effort and
//! `None` where the platform would not say, which the interface reports as unavailable.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// How often the sampler wakes. Minutes-scale staleness is fine for a panel that names
/// resources; seconds-scale polling would be.
const SAMPLE_EVERY: Duration = Duration::from_secs(20);

/// What the sampler last saw. Plain data: the coordinator clones it under a lock nobody
/// contends, since writes happen once per interval and reads on attach or poll.
#[derive(Clone, Debug, Default)]
pub struct HostMeta {
    pub hostname: Option<String>,
    pub os: String,
    pub arch: String,
    pub triplet: String,
    pub cpu_count: u64,
    pub mem_total_bytes: Option<u64>,
    pub mem_free_bytes: Option<u64>,
    pub disk_free_bytes: Option<u64>,
    pub cpu_load_pct: Option<f32>,
}

impl HostMeta {
    fn statics() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            triplet: format!(
                "{}-{}-{}",
                std::env::consts::ARCH,
                std::env::consts::OS,
                std::env::consts::FAMILY
            ),
            cpu_count: num_cpus::get() as u64,
            ..Self::default()
        }
    }
}

/// Start sampling against `config_root`'s volume and return the shared snapshot.
///
/// The first sample is taken inline so an attach in the first seconds already has machine
/// facts; the thread keeps it fresh after that. The thread ends with the process — it holds
/// only the snapshot and a path.
pub fn start(config_root: PathBuf) -> Arc<Mutex<HostMeta>> {
    let meta = Arc::new(Mutex::new(sample(&config_root)));
    let writer = meta.clone();
    thread::Builder::new()
        .name("ubiq-host-meta".to_string())
        .spawn(move || {
            loop {
                thread::sleep(SAMPLE_EVERY);
                let next = sample(&config_root);
                if let Ok(mut guard) = writer.lock() {
                    *guard = next;
                } else {
                    break;
                }
            }
        })
        .expect("the host metadata thread");
    meta
}

fn sample(config_root: &std::path::Path) -> HostMeta {
    let mut meta = HostMeta::statics();
    meta.hostname = hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .filter(|name| !name.is_empty());
    meta.mem_total_bytes = None;
    meta.mem_free_bytes = None;
    meta.cpu_load_pct = None;
    meta.disk_free_bytes = disk_free(config_root);
    sample_memory(&mut meta);
    meta
}

/// Fill RAM/CPU from `sysinfo`, swallowing every failure into `None`.
///
/// A panel that reports how healthy the host is must never be the thing that takes it down —
/// the same rule `Coordinator::stats` applies to the usage meter.
fn sample_memory(meta: &mut HostMeta) {
    let result = std::panic::catch_unwind(|| {
        use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind};
        let mut sys = sysinfo::System::new_with_specifics(
            RefreshKind::new()
                .with_memory(MemoryRefreshKind::new().with_ram())
                .with_cpu(CpuRefreshKind::new().with_cpu_usage()),
        );
        sys.refresh_memory();
        sys.refresh_cpu_usage();
        // CPU usage needs two samples `MINIMUM_CPU_UPDATE_INTERVAL` apart; one extra
        // blocking sleep here is fine — this runs on the sampler thread, never the
        // coordinator's.
        thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_cpu_usage();
        (nonzero(sys.total_memory()), nonzero(sys.free_memory()), {
            let load = sys.global_cpu_usage();
            (load.is_finite() && load >= 0.0).then_some(load)
        })
    });
    if let Ok((total, free, load)) = result {
        meta.mem_total_bytes = total;
        meta.mem_free_bytes = free;
        meta.cpu_load_pct = load;
    }
}

fn nonzero(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

/// Free bytes on the volume holding `path`: the disk whose mount point is the longest prefix
/// of it, or the first disk when nothing matches. `None` when the disk list cannot be read.
fn disk_free(path: &std::path::Path) -> Option<u64> {
    let result = std::panic::catch_unwind(|| {
        use sysinfo::Disks;
        let disks = Disks::new_with_refreshed_list();
        let path = path.to_string_lossy().to_lowercase();
        let mut best: Option<(usize, u64)> = None;
        for disk in disks.list() {
            let mount = disk.mount_point().to_string_lossy().to_lowercase();
            if path.starts_with(mount.as_str()) {
                let free = disk.available_space();
                if best.is_none_or(|(len, _)| mount.len() > len) {
                    best = Some((mount.len(), free));
                }
            }
        }
        best.map(|(_, free)| free)
            .or_else(|| disks.list().first().map(|disk| disk.available_space()))
    });
    result.ok().flatten()
}
