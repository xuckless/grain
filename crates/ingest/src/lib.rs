//! Downloads a YouTube video's best audio stream and converts it to 44.1 kHz stereo WAV,
//! using `yt-dlp` and `ffmpeg` on the user's machine. Desktop only; the web build never
//! links this crate (AGENTS.md §0, §4.1).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Location of the yt-dlp binary. yt-dlp finds ffmpeg/ffprobe itself on [`search_path`]
/// (`--ffmpeg-location` takes a filesystem path, so a bare `ffmpeg` there breaks extraction).
pub struct Tools {
    pub yt_dlp: PathBuf,
}

impl Tools {
    /// yt-dlp looked up on [`search_path`].
    pub fn from_path() -> Self {
        Self {
            yt_dlp: "yt-dlp".into(),
        }
    }
}

/// Common package-manager bin directories. Apps launched from Finder get a minimal `PATH`
/// (`/usr/bin:/bin:...`), so Homebrew-installed tools would otherwise be invisible.
const EXTRA_BIN_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// The inherited `PATH` with [`EXTRA_BIN_DIRS`] appended. Set on the yt-dlp child, so it is
/// used both to find yt-dlp itself and by yt-dlp to find ffmpeg and its JS runtime.
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
fn arguments(url: &str, out_dir: &Path) -> Vec<String> {
    vec![
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
    ]
}

pub fn download(tools: &Tools, url: &str, out_dir: &Path) -> Result<Downloaded, IngestError> {
    let url = normalize_url(url)?;
    let output = Command::new(&tools.yt_dlp)
        .env("PATH", search_path())
        .args(arguments(&url, out_dir))
        .output()
        .map_err(|source| IngestError::Spawn {
            tool: tools.yt_dlp.display().to_string(),
            source,
        })?;
    if !output.status.success() {
        return Err(IngestError::Failed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
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
        let args = arguments("u", Path::new("/tmp/x"));
        let joined = args.join(" ");
        assert!(!joined.contains("--ffmpeg-location"));
        assert!(joined.contains("-f bestaudio -x --audio-format wav"));
        assert!(joined.contains("ffmpeg:-ar 44100 -ac 2"));
        assert!(joined.ends_with(" u"));
    }

    #[test]
    fn missing_binary_is_reported() {
        let tools = Tools {
            yt_dlp: "/nonexistent/yt-dlp".into(),
        };
        let err = download(&tools, "https://youtu.be/dQw4w9WgXcQ", Path::new("/tmp")).unwrap_err();
        assert!(matches!(err, IngestError::Spawn { .. }));
    }
}
