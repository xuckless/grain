//! Round-trip latency per input/output device pair, kept in app data as JSON (AGENTS.md §9).

use anyhow::Result;
use engine_core::cue::{Latency, LatencySource};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Latency assumed before a device pair is calibrated. cpal's stream timestamps already include
/// the latency the drivers report, so only the unreported remainder is left to measure.
pub const DEFAULT_MS: f32 = 0.0;

pub struct LatencyStore {
    path: PathBuf,
}

impl LatencyStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn all(&self) -> BTreeMap<String, Latency> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// The stored latency for `devices`, or the default.
    pub fn get(&self, devices: &str) -> Latency {
        self.all().remove(devices).unwrap_or(Latency {
            ms: DEFAULT_MS,
            source: LatencySource::Default,
            devices: devices.to_string(),
        })
    }

    pub fn set(&self, latency: &Latency) -> Result<()> {
        let mut all = self.all();
        all.insert(latency.devices.clone(), latency.clone());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(&all)?)?;
        Ok(())
    }
}

/// The key of a device pair.
pub fn devices(input: &str, output: &str) -> String {
    format!("{input} | {output}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_per_device_pair() {
        let dir = std::env::temp_dir().join(format!("grain-latency-{}", std::process::id()));
        let store = LatencyStore::new(dir.join("latency.json"));
        let pair = devices("Mic", "Speakers");
        assert_eq!(store.get(&pair).source, LatencySource::Default);
        store
            .set(&Latency {
                ms: 87.5,
                source: LatencySource::Measured { confidence: 0.8 },
                devices: pair.clone(),
            })
            .unwrap();
        assert_eq!(store.get(&pair).ms, 87.5);
        assert_eq!(store.get(&devices("Mic", "Headphones")).ms, DEFAULT_MS);
        let _ = std::fs::remove_dir_all(dir);
    }
}
