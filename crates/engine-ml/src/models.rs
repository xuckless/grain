//! The model registry (AGENTS.md §10) for the app's model manager. `models/manifest.toml` is the
//! single source: it is compiled in here, so desktop, web and `scripts/fetch_models.py` all read
//! the same entries. Hosts store the files (app data on desktop, OPFS on the web) and drive each
//! model's [`ModelState`] with [`next`]; downloads are verified with [`Checksum`].

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

const MANIFEST: &str = include_str!("../../../models/manifest.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Distribution {
    /// Usable in every build.
    Default,
    /// Restricted (e.g. non-commercial): the user accepts the licence before downloading.
    OptIn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelFile {
    pub name: String,
    /// Where it downloads from (Hugging Face or GitHub); `None` when it is built or manual.
    pub url: Option<String>,
    pub sha256: Option<String>,
    /// Bytes.
    pub size: Option<u64>,
}

/// How a model's files are obtained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum Obtain {
    /// Every file downloads (and verifies) in the app.
    Download,
    /// Exported locally from a checkpoint (`command`, see the manifest).
    Build { command: String },
    /// Can't be fetched automatically; `note` says why and how.
    Manual { note: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelInfo {
    pub id: String,
    pub task: String,
    pub distribution: Distribution,
    pub license: String,
    pub license_note: String,
    pub source: String,
    pub files: Vec<ModelFile>,
    pub obtain: Obtain,
}

impl ModelInfo {
    /// Total bytes, when every file's size is known.
    pub fn size(&self) -> Option<u64> {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn file(&self, name: &str) -> Option<&ModelFile> {
        self.files.iter().find(|f| f.name == name)
    }
}

/// The manifest's on-disk shape.
#[derive(Deserialize)]
struct Manifest {
    model: Vec<RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    id: String,
    task: String,
    distribution: Distribution,
    license: String,
    license_note: String,
    source: String,
    manual: Option<String>,
    build: Option<RawBuild>,
    file: Vec<ModelFile>,
}

#[derive(Deserialize)]
struct RawBuild {
    command: String,
}

/// Every model in the manifest, in manifest order.
pub fn manifest() -> Vec<ModelInfo> {
    let m: Manifest =
        toml::from_str(MANIFEST).expect("models/manifest.toml is valid (checked by tests)");
    m.model
        .into_iter()
        .map(|r| ModelInfo {
            obtain: match (r.manual, r.build) {
                (Some(note), _) => Obtain::Manual { note },
                (None, Some(b)) => Obtain::Build { command: b.command },
                (None, None) => Obtain::Download,
            },
            id: r.id,
            task: r.task,
            distribution: r.distribution,
            license: r.license,
            license_note: r.license_note,
            source: r.source,
            files: r.file,
        })
        .collect()
}

/// The model `id`, if the manifest lists it.
pub fn model(id: &str) -> Option<ModelInfo> {
    manifest().into_iter().find(|m| m.id == id)
}

/// Where one model is on this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum ModelState {
    NotDownloaded,
    Downloading {
        received: u64,
        total: u64,
    },
    /// Downloaded; checking the files' checksums.
    Verifying,
    Ready,
    Failed {
        message: String,
    },
}

/// What happens to a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelEvent {
    /// A download starts (`total` bytes).
    Start {
        total: u64,
    },
    Progress {
        received: u64,
    },
    /// Every file is in; verification starts.
    Downloaded,
    Verified,
    Fail {
        message: String,
    },
    Deleted,
}

/// The state after `event`, or `None` when the event doesn't apply in `state`.
pub fn next(state: &ModelState, event: ModelEvent) -> Option<ModelState> {
    use ModelEvent as E;
    use ModelState as S;
    match (state, event) {
        (S::NotDownloaded | S::Failed { .. }, E::Start { total }) => {
            Some(S::Downloading { received: 0, total })
        }
        (S::Downloading { total, .. }, E::Progress { received }) => Some(S::Downloading {
            received: received.min(*total),
            total: *total,
        }),
        (S::Downloading { .. }, E::Downloaded) => Some(S::Verifying),
        (S::Verifying, E::Verified) => Some(S::Ready),
        (S::Downloading { .. } | S::Verifying, E::Fail { message }) => Some(S::Failed { message }),
        (S::Ready | S::Failed { .. }, E::Deleted) => Some(S::NotDownloaded),
        _ => None,
    }
}

/// One model with its state, as the manager lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModelEntry {
    pub info: ModelInfo,
    pub state: ModelState,
}

/// Hosts model downloads may come from (AGENTS.md §10: model hosts only).
pub const ALLOWED_HOSTS: [&str; 3] = ["huggingface.co", "github.com", "raw.githubusercontent.com"];

/// Whether `url` is an https URL on one of [`ALLOWED_HOSTS`].
pub fn allowed(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| ALLOWED_HOSTS.contains(&host))
}

/// Streaming SHA-256 of a file being downloaded.
#[derive(Default)]
pub struct Checksum(Sha256);

impl Checksum {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// Lower-case hex digest.
    pub fn finish(self) -> String {
        self.0
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// Whether `actual` (hex) matches `file`'s checksum (files without one always match).
pub fn matches(file: &ModelFile, actual: &str) -> bool {
    file.sha256
        .as_deref()
        .is_none_or(|want| want.eq_ignore_ascii_case(actual))
}

/// The error a failed checksum reports.
pub fn mismatch(file: &ModelFile) -> String {
    format!(
        "{} failed its checksum (the download was corrupted or the file changed upstream)",
        file.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_and_every_download_is_verifiable() {
        let models = manifest();
        assert!(models.iter().any(|m| m.id == "basic_pitch"));
        for m in &models {
            if m.obtain == Obtain::Download {
                for f in &m.files {
                    assert!(
                        f.url.is_some() && f.sha256.is_some() && f.size.is_some(),
                        "{}: {}",
                        m.id,
                        f.name
                    );
                }
            }
            if m.license.contains("NC") {
                assert_eq!(m.distribution, Distribution::OptIn, "{}", m.id);
            }
        }
        let bp = model("basic_pitch").unwrap();
        assert_eq!(bp.license, "Apache-2.0");
        assert_eq!(bp.size(), Some(230_444));
        assert!(matches!(
            model("muscriptor_large").unwrap().obtain,
            Obtain::Manual { .. }
        ));
        assert!(matches!(
            model("melband_roformer_vocals").unwrap().obtain,
            Obtain::Build { .. }
        ));
    }

    #[test]
    fn downloads_only_come_from_model_hosts() {
        assert!(allowed("https://huggingface.co/a/b/resolve/main/x.onnx"));
        assert!(allowed(
            "https://raw.githubusercontent.com/spotify/basic-pitch/x/nmp.onnx"
        ));
        assert!(!allowed("http://huggingface.co/x"));
        assert!(!allowed("https://huggingface.co.evil.example/x"));
        for m in manifest() {
            for f in m.files.iter().filter_map(|f| f.url.as_deref()) {
                assert!(allowed(f), "{f}");
            }
        }
    }

    #[test]
    fn state_machine_walks_a_download() {
        let s = ModelState::NotDownloaded;
        let s = next(&s, ModelEvent::Start { total: 100 }).unwrap();
        let s = next(&s, ModelEvent::Progress { received: 40 }).unwrap();
        assert_eq!(
            s,
            ModelState::Downloading {
                received: 40,
                total: 100
            }
        );
        let s = next(&s, ModelEvent::Downloaded).unwrap();
        assert_eq!(s, ModelState::Verifying);
        let s = next(&s, ModelEvent::Verified).unwrap();
        assert_eq!(s, ModelState::Ready);
        // Ready models don't restart or report progress; they can be deleted.
        assert_eq!(next(&s, ModelEvent::Start { total: 1 }), None);
        assert_eq!(next(&s, ModelEvent::Progress { received: 1 }), None);
        assert_eq!(
            next(&s, ModelEvent::Deleted),
            Some(ModelState::NotDownloaded)
        );
    }

    #[test]
    fn checksum_failure_fails_the_model_and_allows_a_retry() {
        let file = ModelFile {
            name: "x.onnx".into(),
            url: Some("https://huggingface.co/x".into()),
            sha256: Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()),
            size: Some(3),
        };
        let mut good = Checksum::new();
        good.update(b"ab");
        good.update(b"c");
        assert!(matches(&file, &good.finish()));
        let mut bad = Checksum::new();
        bad.update(b"abd");
        let digest = bad.finish();
        assert!(!matches(&file, &digest));

        let s = next(&ModelState::NotDownloaded, ModelEvent::Start { total: 3 }).unwrap();
        let s = next(&s, ModelEvent::Downloaded).unwrap();
        let s = next(
            &s,
            ModelEvent::Fail {
                message: mismatch(&file),
            },
        )
        .unwrap();
        assert!(matches!(&s, ModelState::Failed { message } if message.contains("checksum")));
        assert!(matches!(
            next(&s, ModelEvent::Start { total: 3 }),
            Some(ModelState::Downloading { .. })
        ));
    }
}
