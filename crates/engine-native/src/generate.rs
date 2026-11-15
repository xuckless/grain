//! The optional AI-generation sidecar on desktop (AGENTS.md §8): `services/ml-sidecar/` (ACE-Step
//! 1.5 behind FastAPI) installed on request with `uv` into app data, started for one generation
//! and stopped afterwards, so no Python stays resident. It listens on 127.0.0.1 only, on a free
//! port, and every request carries a random per-run token.
//!
//! The lifecycle states are [`engine_core::generate::GeneratorState`]; this module does the
//! work behind each transition.

use anyhow::{anyhow, bail, Context, Result};
use engine_core::generate::GenerationRequest;
use engine_core::resample::resample;
use engine_core::wav;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The service's files, copied from the app (or repository) into the install.
const SERVICE_FILES: [&str; 7] = [
    "pyproject.toml",
    "uv.lock",
    "grain_sidecar/__init__.py",
    "grain_sidecar/__main__.py",
    "grain_sidecar/server.py",
    "grain_sidecar/stub_backend.py",
    "grain_sidecar/acestep_backend.py",
];
/// How long a started sidecar may take to answer `/health` (uv checks the environment, Python
/// imports FastAPI; the model itself loads on the first generation).
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// How long one generation may take (CPU-only runs of a long song are slow).
const GENERATE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// `uv`, from PATH or where its installers put it (apps started from the Dock get no shell PATH).
pub fn find_uv() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let exe = if cfg!(windows) { "uv.exe" } else { "uv" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .chain(
            home.into_iter()
                .flat_map(|h| [h.join(".local/bin"), h.join(".cargo/bin")]),
        )
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}

/// Which backend the sidecar serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    AceStep,
    /// No model: a click track (tests and development).
    Stub,
}

/// Where the sidecar is installed: `root/project` (the service and its `.venv`) and
/// `root/checkpoints` (ACE-Step's weights).
pub struct Install {
    pub root: PathBuf,
    pub uv: PathBuf,
}

impl Install {
    fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    fn checkpoints(&self) -> PathBuf {
        self.root.join("checkpoints")
    }

    fn marker(&self) -> PathBuf {
        self.root.join("installed")
    }

    pub fn installed(&self) -> bool {
        self.marker().is_file()
    }

    /// `uv <args>` on the installed project, with the ACE-Step extra.
    fn uv(&self, args: &[&str]) -> Command {
        let mut c = Command::new(&self.uv);
        c.arg(args[0])
            .args(["--frozen", "--extra", "acestep", "--project"])
            .arg(self.project())
            .args(&args[1..])
            .env("ACESTEP_CHECKPOINTS_DIR", self.checkpoints())
            .stdin(Stdio::null());
        c
    }

    /// Copies the service from `service` (the bundled or repository copy), creates its Python
    /// environment and downloads the weights. `on_step` names each step. Blocking, long.
    pub fn install(&self, service: &Path, mut on_step: impl FnMut(&str)) -> Result<()> {
        on_step("Copying the sidecar");
        for f in SERVICE_FILES {
            let to = self.project().join(f);
            std::fs::create_dir_all(to.parent().expect("has a parent"))?;
            std::fs::copy(service.join(f), &to).with_context(|| f.to_string())?;
        }
        on_step("Installing Python and PyTorch (about 2 GB)");
        run(self.uv(&["sync"]))?;
        on_step("Downloading ACE-Step weights (about 6.3 GB)");
        let checkpoints = self.checkpoints();
        let checkpoints = checkpoints.to_string_lossy();
        run(self.uv(&[
            "run",
            "python",
            "-I",
            "-m",
            "grain_sidecar",
            "download",
            "--checkpoints",
            &checkpoints,
        ]))?;
        std::fs::write(self.marker(), b"")?;
        Ok(())
    }

    /// Starts the installed sidecar for a run.
    pub fn start(&self) -> Result<Sidecar> {
        let checkpoints = self.checkpoints();
        let checkpoints = checkpoints.to_string_lossy();
        let mut module = module(&self.project());
        module.env("ACESTEP_CHECKPOINTS_DIR", self.checkpoints());
        Sidecar::start(module, &["--checkpoints", &checkpoints], Backend::AceStep)
    }
}

/// `python -I -m grain_sidecar` in `project`'s environment. Python runs directly (not under
/// `uv run`), so stopping the child stops the server.
pub fn module(project: &Path) -> Command {
    let python = if cfg!(windows) {
        project.join(".venv/Scripts/python.exe")
    } else {
        project.join(".venv/bin/python")
    };
    let mut c = Command::new(python);
    c.args(["-I", "-m", "grain_sidecar"]).stdin(Stdio::null());
    c
}

/// Removes an install (its environment and weights) at `root`.
pub fn uninstall(root: &Path) -> Result<()> {
    if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

/// Runs `c` to completion; fails with the end of its output.
fn run(mut c: Command) -> Result<()> {
    let out = c.output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err.lines().rev().take(6).collect::<Vec<_>>().join("\n");
        bail!("{} failed: {tail}", c.get_program().to_string_lossy());
    }
    Ok(())
}

/// A free loopback port.
fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// 128 random bits as hex, for the per-run token.
fn random_token() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow!("no randomness: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// A running sidecar; stopped (killed) when dropped.
pub struct Sidecar {
    child: Child,
    pub base: String,
    token: String,
}

impl Sidecar {
    /// Starts `module` (see [`module`]) serving `backend`
    /// on a free loopback port, with `extra` arguments, and waits until it answers.
    pub fn start(mut module: Command, extra: &[&str], backend: Backend) -> Result<Self> {
        let port = free_port()?;
        let token = random_token()?;
        module
            .args(["serve", "--port", &port.to_string(), "--backend"])
            .arg(match backend {
                Backend::AceStep => "acestep",
                Backend::Stub => "stub",
            })
            .args(extra)
            .env("GRAIN_SIDECAR_TOKEN", &token)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let child = module.spawn().context("starting the sidecar")?;
        let mut sidecar = Self {
            child,
            base: format!("http://127.0.0.1:{port}"),
            token,
        };
        sidecar.wait_ready()?;
        Ok(sidecar)
    }

    fn agent(timeout: Duration) -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .new_agent()
    }

    fn wait_ready(&mut self) -> Result<()> {
        let start = Instant::now();
        let agent = Self::agent(Duration::from_secs(2));
        loop {
            if let Some(status) = self.child.try_wait()? {
                let mut err = String::new();
                if let Some(e) = self.child.stderr.as_mut() {
                    let _ = e.read_to_string(&mut err);
                }
                let tail: String = err.lines().rev().take(4).collect::<Vec<_>>().join("\n");
                bail!("the sidecar exited ({status}): {tail}");
            }
            let health = agent
                .get(format!("{}/health", self.base))
                .header("Authorization", format!("Bearer {}", self.token))
                .call();
            if let Ok(r) = health {
                if r.status() == 200 {
                    return Ok(());
                }
                bail!("the sidecar refused its token ({})", r.status());
            }
            if start.elapsed() > START_TIMEOUT {
                bail!(
                    "the sidecar didn't start within {}s",
                    START_TIMEOUT.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// `GET /health` with `token`, returning the status code (for tests of the contract).
    pub fn health_status(&self, token: &str) -> Result<u16> {
        let r = Self::agent(Duration::from_secs(5))
            .get(format!("{}/health", self.base))
            .header("Authorization", format!("Bearer {token}"))
            .call()?;
        Ok(r.status().as_u16())
    }

    /// Runs `request` conditioned on `audio` (mono, `sample_rate`, `seconds` long); returns the
    /// generated audio as mono at `sample_rate`, `audio.len()` samples.
    pub fn generate(
        &self,
        request: &GenerationRequest,
        audio: &[f32],
        sample_rate: u32,
    ) -> Result<Vec<f32>> {
        let seconds = audio.len() as f64 / f64::from(sample_rate);
        let mut call = Self::agent(GENERATE_TIMEOUT)
            .post(format!("{}/generate", self.base))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "audio/wav");
        for (k, v) in request.query(seconds) {
            call = call.query(k, v);
        }
        let mut response = call.send(&wav::encode_mono16(audio, sample_rate)[..])?;
        let status = response.status();
        let body = response.body_mut().read_to_vec()?;
        if status != 200 {
            let detail = String::from_utf8_lossy(&body);
            bail!("generation failed ({status}): {detail}");
        }
        let (mono, rate) = wav::decode_mono(&body)?;
        let mut out = resample(&[&mono], rate, sample_rate)?.remove(0);
        out.resize(audio.len(), 0.0);
        Ok(out)
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::edit::EditSource;
    use engine_core::generate::{GenerationTask, Instrument, GENERATION_MODEL};

    /// The repository's service, run with its locked base dependencies (no ACE-Step).
    fn stub() -> Option<Sidecar> {
        let Some(uv) = find_uv() else {
            eprintln!("skipped: uv not found");
            return None;
        };
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../services/ml-sidecar");
        let mut sync = Command::new(uv);
        sync.args(["sync", "--frozen", "--project"]).arg(&project);
        run(sync).unwrap();
        Some(Sidecar::start(module(&project), &[], Backend::Stub).unwrap())
    }

    fn request(task: GenerationTask) -> GenerationRequest {
        GenerationRequest {
            task,
            prompt: "warm keys & brushed drums".into(),
            seed: 42,
            model: GENERATION_MODEL.into(),
            bpm: 100.0,
            key: "C Major".into(),
        }
    }

    #[test]
    fn stub_sidecar_lifecycle_and_contract() {
        let Some(sidecar) = stub() else { return };
        // Only its own token gets in.
        assert_eq!(sidecar.health_status("wrong").unwrap(), 401);
        let sr = 44_100;
        let vocal = vec![0.1f32; sr as usize * 12];
        for task in [
            GenerationTask::Backing {
                from: EditSource::VocalStem,
            },
            GenerationTask::Lego {
                from: EditSource::VocalStem,
                instrument: Instrument::Bass,
            },
            GenerationTask::Repaint {
                start: 2.0,
                end: 4.0,
            },
        ] {
            let out = sidecar.generate(&request(task), &vocal, sr).unwrap();
            assert_eq!(out.len(), vocal.len());
            assert!(
                out.iter().any(|v| v.abs() > 0.05),
                "{task:?}: the click track"
            );
        }
        let bad = GenerationTask::Repaint {
            start: 5.0,
            end: 1.0,
        };
        let err = sidecar.generate(&request(bad), &vocal, sr).unwrap_err();
        assert!(err.to_string().contains("start < end"), "{err}");
        let base = sidecar.base.clone();
        drop(sidecar);
        // Stopped: nothing listens any more.
        let gone = ureq::get(format!("{base}/health")).call();
        assert!(gone.is_err());
    }

    #[test]
    fn install_layout_is_under_its_root() {
        let i = Install {
            root: PathBuf::from("/data/ml-sidecar"),
            uv: PathBuf::from("uv"),
        };
        assert!(!i.installed());
        let c = i.uv(&["sync"]);
        let args: Vec<_> = c
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "sync",
                "--frozen",
                "--extra",
                "acestep",
                "--project",
                // Platform separators: `\` on Windows.
                &PathBuf::from("/data/ml-sidecar")
                    .join("project")
                    .to_string_lossy(),
            ]
        );
    }
}
