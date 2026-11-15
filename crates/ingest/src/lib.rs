//! Downloads a YouTube video's best audio stream and converts it to 44.1 kHz stereo WAV,
//! using `yt-dlp` and `ffmpeg` on the user's machine. Desktop only; the web build never
//! links this crate (AGENTS.md §0, §4.1).
//!
//! The app bundles yt-dlp, ffmpeg and ffprobe as sidecars (apps/desktop/src-tauri/sidecars.toml);
//! [`resolve`] prefers those and falls back to tools on `PATH`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the yt-dlp in use came from, in lookup order (AGENTS.md §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A copy in the app's data directory, made by [`update`] (the bundle itself is read-only).
    Updated,
    /// The sidecar shipped next to the app executable.
    Bundled,
    /// Found on [`search_path`] (e.g. Homebrew).
    System,
}

/// The tools a download runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub yt_dlp: PathBuf,
    pub origin: Origin,
    /// Directory holding bundled `ffmpeg` and `ffprobe`, passed as `--ffmpeg-location` (which
    /// takes a real path, never a bare name). Absent when they are not bundled: yt-dlp then
    /// finds them on [`search_path`] itself.
    pub ffmpeg_dir: Option<PathBuf>,
}

/// Result of looking for yt-dlp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Found(Tools),
    Missing,
}

/// Looks for yt-dlp in order: the updated copy in `data_bin`, the sidecar in `exe_dir`, then
/// [`search_path`]. Bundled ffmpeg/ffprobe in `exe_dir` are used whichever yt-dlp is found.
pub fn resolve(exe_dir: &Path, data_bin: &Path) -> Resolution {
    let ffmpeg_dir = (exe_dir.join(exe("ffmpeg")).is_file()
        && exe_dir.join(exe("ffprobe")).is_file())
    .then(|| exe_dir.to_path_buf());
    let local = [
        (data_bin.join(exe("yt-dlp")), Origin::Updated),
        (exe_dir.join(exe("yt-dlp")), Origin::Bundled),
    ]
    .into_iter()
    .find(|(path, _)| path.is_file());
    let found = local.or_else(|| {
        std::env::split_paths(&search_path())
            .map(|dir| dir.join(exe("yt-dlp")))
            .find(|path| path.is_file())
            .map(|path| (path, Origin::System))
    });
    match found {
        Some((yt_dlp, origin)) => Resolution::Found(Tools {
            yt_dlp,
            origin,
            ffmpeg_dir,
        }),
        None => Resolution::Missing,
    }
}

/// A tool (`ffmpeg`, `ffprobe`) bundled next to the app's executable, else on [`search_path`].
pub fn find_tool(exe_dir: &Path, name: &str) -> Option<PathBuf> {
    let bundled = exe_dir.join(exe(name));
    if bundled.is_file() {
        return Some(bundled);
    }
    std::env::split_paths(&search_path())
        .map(|dir| dir.join(exe(name)))
        .find(|p| p.is_file())
}

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Common package-manager bin directories. Apps launched from Finder get a minimal `PATH`
/// (`/usr/bin:/bin:...`), so Homebrew-installed tools would otherwise be invisible.
const EXTRA_BIN_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// The inherited `PATH` with [`EXTRA_BIN_DIRS`] appended. Set on every yt-dlp child, so it
/// finds a system ffmpeg and its JS runtime (deno) when those are not bundled.
fn search_path() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for extra in EXTRA_BIN_DIRS {
        let extra = PathBuf::from(extra);
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    std::env::join_paths(dirs).expect("PATH entries contain no separator")
}

/// Runs yt-dlp at `path` with `args`; returns stdout, or the trimmed error output.
fn run<S: AsRef<std::ffi::OsStr>>(path: &Path, args: &[S]) -> Result<String, IngestError> {
    let output = Command::new(path)
        .env("PATH", search_path())
        .args(args)
        .output()
        .map_err(|source| IngestError::Spawn {
            tool: path.display().to_string(),
            source,
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = if stderr.trim().is_empty() {
            stdout.as_str()
        } else {
            &stderr
        };
        return Err(IngestError::Failed(message.trim().to_string()));
    }
    Ok(stdout)
}

/// The yt-dlp version string, e.g. `2026.10.26`.
pub fn version(tools: &Tools) -> Result<String, IngestError> {
    Ok(run(&tools.yt_dlp, &["--version"])?.trim().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    Updated { version: String },
    UpToDate { version: String },
}

/// Updates the extractor with yt-dlp's own `-U`, which downloads the latest GitHub release and
/// checks it against that release's SHA2-256SUMS. The bundled sidecar is never modified: it is
/// copied to `data_bin` and the copy is updated; a failed or no-op update deletes the copy, so
/// the bundled one stays in use. A system yt-dlp belongs to its package manager.
pub fn update(tools: &Tools, data_bin: &Path) -> Result<UpdateOutcome, IngestError> {
    let before = version(tools)?;
    let target = match tools.origin {
        Origin::System => {
            return Err(IngestError::Unsupported(format!(
                "yt-dlp {before} at {} is managed by your package manager; update it there (for Homebrew: brew upgrade yt-dlp)",
                tools.yt_dlp.display()
            )))
        }
        Origin::Updated => tools.yt_dlp.clone(),
        Origin::Bundled => {
            std::fs::create_dir_all(data_bin)?;
            let copy = data_bin.join(exe("yt-dlp"));
            std::fs::copy(&tools.yt_dlp, &copy)?;
            copy
        }
    };
    let candidate = Tools {
        yt_dlp: target.clone(),
        ..tools.clone()
    };
    let after = run(&target, &["-U"]).and_then(|_| version(&candidate));
    let keep_copy = matches!(&after, Ok(v) if *v != before);
    if tools.origin == Origin::Bundled && !keep_copy {
        // Best effort: a leftover copy would only shadow the bundled sidecar.
        let _ = std::fs::remove_file(&target);
    }
    let after = after?;
    Ok(if after == before {
        UpdateOutcome::UpToDate { version: after }
    } else {
        UpdateOutcome::Updated { version: after }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    pub wav: PathBuf,
    pub title: String,
    pub url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("not a YouTube video link: {0}")]
    NotYouTube(String),
    #[error("could not run {tool}: {source}")]
    Spawn {
        tool: String,
        source: std::io::Error,
    },
    #[error("yt-dlp failed: {0}")]
    Failed(String),
    #[error("{0}")]
    Unsupported(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Accepts youtube.com/watch, youtu.be and music.youtube.com links; strips playlist context.
pub fn normalize_url(url: &str) -> Result<String, IngestError> {
    let trimmed = url.trim();
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let id = if let Some(q) = rest
        .strip_prefix("youtube.com/watch?")
        .or_else(|| rest.strip_prefix("music.youtube.com/watch?"))
        .or_else(|| rest.strip_prefix("m.youtube.com/watch?"))
    {
        q.split('&').find_map(|kv| kv.strip_prefix("v="))
    } else if let Some(p) = rest.strip_prefix("youtu.be/") {
        p.split(['?', '&', '/']).next()
    } else {
        None
    };
    match id {
        Some(id)
            if id.len() == 11
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') =>
        {
            Ok(format!("https://www.youtube.com/watch?v={id}"))
        }
        _ => Err(IngestError::NotYouTube(url.to_string())),
    }
}

/// yt-dlp arguments: best audio only, no playlist, extracted to 44.1 kHz stereo WAV.
fn arguments(url: &str, out_dir: &Path, ffmpeg_dir: Option<&Path>) -> Vec<String> {
    let ffmpeg = ffmpeg_dir
        .map(|dir| vec!["--ffmpeg-location".to_string(), dir.display().to_string()])
        .unwrap_or_default();
    ffmpeg
        .into_iter()
        .chain([
            "--no-playlist".into(),
            "--no-progress".into(),
            "-f".into(),
            "bestaudio".into(),
            "-x".into(),
            "--audio-format".into(),
            "wav".into(),
            "--postprocessor-args".into(),
            "ffmpeg:-ar 44100 -ac 2".into(),
            "-o".into(),
            out_dir.join("%(id)s.%(ext)s").display().to_string(),
            "--print".into(),
            "after_move:%(title)s".into(),
            "--print".into(),
            "after_move:filepath".into(),
            url.into(),
        ])
        .collect()
}

pub fn download(tools: &Tools, url: &str, out_dir: &Path) -> Result<Downloaded, IngestError> {
    let url = normalize_url(url)?;
    let stdout = run(
        &tools.yt_dlp,
        &arguments(&url, out_dir, tools.ffmpeg_dir.as_deref()),
    )?;
    let mut lines = stdout.lines().filter(|l| !l.trim().is_empty());
    match (lines.next(), lines.next()) {
        (Some(title), Some(path)) => Ok(Downloaded {
            wav: PathBuf::from(path.trim()),
            title: title.trim().to_string(),
            url,
        }),
        _ => Err(IngestError::Failed(format!(
            "unexpected yt-dlp output: {stdout}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_links() {
        let canonical = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "youtube.com/watch?list=PL123&v=dQw4w9WgXcQ&t=10",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
        ] {
            assert_eq!(normalize_url(url).unwrap(), canonical, "{url}");
        }
        for url in [
            "https://vimeo.com/123",
            "https://youtu.be/short",
            "dQw4w9WgXcQ",
        ] {
            assert!(normalize_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn arguments_request_audio_only_wav() {
        let joined = arguments("u", Path::new("/tmp/x"), None).join(" ");
        assert!(!joined.contains("--ffmpeg-location"));
        assert!(joined.contains("-f bestaudio -x --audio-format wav"));
        assert!(joined.contains("ffmpeg:-ar 44100 -ac 2"));
        assert!(joined.ends_with(" u"));
    }

    #[test]
    fn bundled_ffmpeg_is_passed_as_a_directory() {
        let args = arguments("u", Path::new("/tmp/x"), Some(Path::new("/App/MacOS")));
        assert_eq!(args[..2], ["--ffmpeg-location", "/App/MacOS"]);
    }

    fn fake_tools(dir: &Path, names: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        for name in names {
            std::fs::write(dir.join(exe(name)), b"").unwrap();
        }
    }

    #[test]
    fn lookup_prefers_updated_then_bundled_and_finds_bundled_ffmpeg() {
        let root = std::env::temp_dir().join(format!("ingest-resolve-{}", std::process::id()));
        let (exe_dir, data_bin) = (root.join("MacOS"), root.join("bin"));
        fake_tools(&exe_dir, &["yt-dlp", "ffmpeg", "ffprobe"]);
        let Resolution::Found(bundled) = resolve(&exe_dir, &data_bin) else {
            panic!("bundled yt-dlp not found");
        };
        assert_eq!(bundled.origin, Origin::Bundled);
        assert_eq!(bundled.ffmpeg_dir.as_deref(), Some(exe_dir.as_path()));

        fake_tools(&data_bin, &["yt-dlp"]);
        let Resolution::Found(updated) = resolve(&exe_dir, &data_bin) else {
            panic!("updated yt-dlp not found");
        };
        assert_eq!(updated.origin, Origin::Updated);
        assert_eq!(updated.yt_dlp, data_bin.join(exe("yt-dlp")));

        // Without ffprobe, ffmpeg is not taken from the bundle.
        std::fs::remove_file(exe_dir.join(exe("ffprobe"))).unwrap();
        let Resolution::Found(partial) = resolve(&exe_dir, &data_bin) else {
            panic!("yt-dlp not found");
        };
        assert_eq!(partial.ffmpeg_dir, None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_binary_is_reported() {
        let tools = Tools {
            yt_dlp: "/nonexistent/yt-dlp".into(),
            origin: Origin::Bundled,
            ffmpeg_dir: None,
        };
        let err = download(&tools, "https://youtu.be/dQw4w9WgXcQ", Path::new("/tmp")).unwrap_err();
        assert!(matches!(err, IngestError::Spawn { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn system_yt_dlp_is_left_to_its_package_manager() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ingest-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("yt-dlp");
        std::fs::write(&fake, "#!/bin/sh\necho 2026.10.26\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tools = Tools {
            yt_dlp: fake,
            origin: Origin::System,
            ffmpeg_dir: None,
        };
        assert_eq!(version(&tools).unwrap(), "2026.10.26");
        assert!(matches!(
            update(&tools, &dir.join("bin")),
            Err(IngestError::Unsupported(_))
        ));
        assert!(!dir.join("bin").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
