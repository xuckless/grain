//! The YouTube extractor: which yt-dlp is in use, downloading with it, and updating it
//! (AGENTS.md §4.1). Lookup order: updated copy in app data → bundled sidecar → `PATH`.

use crate::UrlImport;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Extractor {
    exe_dir: PathBuf,
    data_bin: PathBuf,
    downloads: PathBuf,
    resolution: Mutex<ingest::Resolution>,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Updated,
    Bundled,
    System,
}

impl From<ingest::Origin> for Origin {
    fn from(o: ingest::Origin) -> Self {
        match o {
            ingest::Origin::Updated => Origin::Updated,
            ingest::Origin::Bundled => Origin::Bundled,
            ingest::Origin::System => Origin::System,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExtractorUpdate {
    Updated { version: String, origin: Origin },
    UpToDate { version: String, origin: Origin },
}

impl Extractor {
    pub fn new(data: &Path) -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        let data_bin = data.join("bin");
        let resolution = Mutex::new(ingest::resolve(&exe_dir, &data_bin));
        Self {
            exe_dir,
            data_bin,
            downloads: data.join("downloads"),
            resolution,
        }
    }

    fn tools(&self) -> Result<ingest::Tools, String> {
        match &*self.resolution.lock().expect("extractor lock") {
            ingest::Resolution::Found(tools) => Ok(tools.clone()),
            ingest::Resolution::Missing => Err(MISSING.into()),
        }
    }

    pub fn status(&self) -> UrlImport {
        match &*self.resolution.lock().expect("extractor lock") {
            ingest::Resolution::Found(tools) => UrlImport::Available {
                origin: tools.origin.into(),
            },
            ingest::Resolution::Missing => UrlImport::Missing {
                reason: MISSING.into(),
            },
        }
    }

    pub fn version(&self) -> Result<String, String> {
        ingest::version(&self.tools()?).map_err(|e| e.to_string())
    }

    pub fn download(&self, url: &str) -> anyhow::Result<ingest::Downloaded> {
        std::fs::create_dir_all(&self.downloads)?;
        let tools = self.tools().map_err(anyhow::Error::msg)?;
        Ok(ingest::download(&tools, url, &self.downloads)?)
    }

    /// Updates yt-dlp, then looks it up again so later downloads use the new copy.
    pub fn update(&self) -> Result<ExtractorUpdate, String> {
        let outcome = ingest::update(&self.tools()?, &self.data_bin).map_err(|e| e.to_string())?;
        let resolution = ingest::resolve(&self.exe_dir, &self.data_bin);
        let origin = match &resolution {
            ingest::Resolution::Found(tools) => tools.origin.into(),
            ingest::Resolution::Missing => return Err(MISSING.into()),
        };
        *self.resolution.lock().expect("extractor lock") = resolution;
        Ok(match outcome {
            ingest::UpdateOutcome::Updated { version } => {
                ExtractorUpdate::Updated { version, origin }
            }
            ingest::UpdateOutcome::UpToDate { version } => {
                ExtractorUpdate::UpToDate { version, origin }
            }
        })
    }
}

const MISSING: &str =
    "yt-dlp isn't bundled with this build or installed. Install it (e.g. brew install yt-dlp) or use a release build.";
