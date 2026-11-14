//! Native model store for the model manager (AGENTS.md §10): downloads manifest models into the
//! app's data directory, verifying every file's SHA-256 as it streams, and deletes them again.
//! The registry, state machine and checksum are shared with the web ([`engine_ml::models`]).

use anyhow::{bail, Context, Result};
use engine_ml::models::{self, Checksum, ModelEntry, ModelEvent, ModelInfo, ModelState, Obtain};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Progress is reported at most this often (bytes).
const PROGRESS_STEP: u64 = 1 << 20;

pub struct ModelStore {
    /// Where downloads go (app data).
    dir: PathBuf,
    /// Every directory models are found in (`dir` first; development builds add the repo's).
    search: Vec<PathBuf>,
    /// Models being downloaded or verified, or whose last download failed.
    active: HashMap<String, ModelState>,
}

impl ModelStore {
    pub fn new(search: Vec<PathBuf>) -> Self {
        Self {
            dir: search[0].clone(),
            search,
            active: HashMap::new(),
        }
    }

    pub fn dirs(&self) -> &[PathBuf] {
        &self.search
    }

    /// Where a model file is installed, if anywhere.
    pub fn find(&self, name: &str) -> Option<PathBuf> {
        self.search
            .iter()
            .map(|d| d.join(name))
            .find(|p| p.exists())
    }

    fn installed(&self, info: &ModelInfo) -> bool {
        info.files.iter().all(|f| self.find(&f.name).is_some())
    }

    pub fn state(&self, info: &ModelInfo) -> ModelState {
        match self.active.get(&info.id) {
            Some(s) => s.clone(),
            None if self.installed(info) => ModelState::Ready,
            None => ModelState::NotDownloaded,
        }
    }

    pub fn entries(&self) -> Vec<ModelEntry> {
        models::manifest()
            .into_iter()
            .map(|info| ModelEntry {
                state: self.state(&info),
                info,
            })
            .collect()
    }

    /// Applies `event` to model `id`'s state; returns the new state (unchanged if it doesn't
    /// apply). A finished or deleted model leaves the active set (its files speak for it).
    pub fn advance(&mut self, info: &ModelInfo, event: ModelEvent) -> ModelState {
        let current = self.state(info);
        let Some(next) = models::next(&current, event) else {
            return current;
        };
        match next {
            ModelState::Ready | ModelState::NotDownloaded => self.active.remove(&info.id),
            ref s => self.active.insert(info.id.clone(), s.clone()),
        };
        next
    }

    /// Removes a downloaded model's files (only from the app's own directory).
    pub fn delete(&mut self, info: &ModelInfo) -> Result<ModelState> {
        for f in &info.files {
            let path = self.dir.join(&f.name);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
        }
        Ok(self.advance(info, ModelEvent::Deleted))
    }

    pub fn download_dir(&self) -> &Path {
        &self.dir
    }
}

/// Downloads every file of `info` into `dir`, streaming each through its checksum; `on_event`
/// sees the download's events (start, progress, downloaded, then verified or a failure). Files are
/// written as `.part` and only renamed into place once every checksum matched. Blocking.
pub fn download(info: &ModelInfo, dir: &Path, mut on_event: impl FnMut(ModelEvent)) -> Result<()> {
    if info.obtain != Obtain::Download {
        bail!("{} can't be downloaded in the app", info.id);
    }
    let total = info.size().unwrap_or(0);
    on_event(ModelEvent::Start { total });
    match fetch_all(info, dir, &mut on_event) {
        Ok(()) => {
            on_event(ModelEvent::Verified);
            Ok(())
        }
        Err(e) => {
            on_event(ModelEvent::Fail {
                message: e.to_string(),
            });
            Err(e)
        }
    }
}

fn fetch_all(info: &ModelInfo, dir: &Path, on_event: &mut impl FnMut(ModelEvent)) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .new_agent();
    let mut received = 0u64;
    let mut digests = Vec::new();
    for f in &info.files {
        let url = f.url.as_deref().context("file has no url")?;
        if !models::allowed(url) {
            bail!("{url} isn't on a model host");
        }
        let part = dir.join(format!("{}.part", f.name));
        let mut body = agent.get(url).call()?.into_body().into_reader();
        let mut out = std::fs::File::create(&part)?;
        let mut checksum = Checksum::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut reported = received;
        loop {
            let n = body.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            checksum.update(&buf[..n]);
            received += n as u64;
            if received - reported >= PROGRESS_STEP {
                reported = received;
                on_event(ModelEvent::Progress { received });
            }
        }
        out.flush()?;
        digests.push((f, part, checksum.finish()));
    }
    on_event(ModelEvent::Progress { received });
    on_event(ModelEvent::Downloaded);
    let result = verify(&digests, dir);
    if result.is_err() {
        for (_, part, _) in &digests {
            let _ = std::fs::remove_file(part);
        }
    }
    result
}

/// Checks every downloaded file's digest, then moves them into place.
fn verify(digests: &[(&engine_ml::models::ModelFile, PathBuf, String)], dir: &Path) -> Result<()> {
    if let Some((f, _, _)) = digests.iter().find(|(f, _, d)| !models::matches(f, d)) {
        bail!(models::mismatch(f));
    }
    for (f, part, _) in digests {
        std::fs::rename(part, dir.join(&f.name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_ml::models::{Distribution, ModelFile};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "engine-native-models-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn info(sha: &str) -> ModelInfo {
        ModelInfo {
            id: "x".into(),
            task: String::new(),
            distribution: Distribution::Default,
            license: "MIT".into(),
            license_note: String::new(),
            source: String::new(),
            files: vec![ModelFile {
                name: "x.onnx".into(),
                url: Some("https://huggingface.co/x".into()),
                sha256: Some(sha.into()),
                size: Some(3),
            }],
            obtain: Obtain::Download,
        }
    }

    #[test]
    fn a_bad_checksum_leaves_nothing_installed() {
        let dir = temp("verify");
        let part = dir.join("x.onnx.part");
        std::fs::write(&part, b"abd").unwrap();
        let m = info("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let mut c = Checksum::new();
        c.update(b"abd");
        let err = verify(&[(&m.files[0], part.clone(), c.finish())], &dir).unwrap_err();
        assert!(err.to_string().contains("checksum"));
        assert!(!dir.join("x.onnx").exists());

        // The right bytes go into place.
        std::fs::write(&part, b"abc").unwrap();
        let mut c = Checksum::new();
        c.update(b"abc");
        verify(&[(&m.files[0], part, c.finish())], &dir).unwrap();
        assert!(dir.join("x.onnx").exists());
    }

    /// Downloads the small basic-pitch model from GitHub (network; run with `--ignored`).
    #[test]
    #[ignore]
    fn downloads_and_verifies_a_real_model() {
        let dir = temp("network");
        let m = models::model("basic_pitch").unwrap();
        let mut events = Vec::new();
        download(&m, &dir, |e| events.push(e)).unwrap();
        assert!(dir.join("basic_pitch.onnx").exists());
        assert_eq!(events.last(), Some(&ModelEvent::Verified));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn store_tracks_files_and_deletes_only_its_own() {
        let dir = temp("store");
        let mut store = ModelStore::new(vec![dir.clone()]);
        let m = info("00");
        assert_eq!(store.state(&m), ModelState::NotDownloaded);
        store.advance(&m, ModelEvent::Start { total: 3 });
        assert!(matches!(store.state(&m), ModelState::Downloading { .. }));
        store.advance(
            &m,
            ModelEvent::Fail {
                message: "offline".into(),
            },
        );
        assert!(matches!(store.state(&m), ModelState::Failed { .. }));
        std::fs::write(dir.join("x.onnx"), b"abc").unwrap();
        store.advance(&m, ModelEvent::Deleted);
        assert_eq!(store.state(&m), ModelState::Ready);
        assert_eq!(store.delete(&m).unwrap(), ModelState::NotDownloaded);
        assert!(!dir.join("x.onnx").exists());
    }
}
