//! Zed extension entry point: resolves and launches the `csv-lsp` language
//! server.
//!
//! Resolution order:
//! 1. `lsp.csv-lsp.binary.path` in Zed settings (explicit user override).
//! 2. `csv-lsp` on the worktree's `PATH` (local builds, development).
//! 3. A binary downloaded from this repository's latest GitHub release and
//!    cached in the extension's work directory. If GitHub can't be reached,
//!    the previously downloaded version is used.

use std::fs;

use zed_extension_api::{
    self as zed, settings::LspSettings, Architecture, DownloadedFileType, GithubReleaseOptions,
    LanguageServerId, LanguageServerInstallationStatus, Os, Result, Worktree,
};

const GITHUB_REPO: &str = "SirToffski/zed_csv_utils";
const SERVER_NAME: &str = "csv-lsp";
/// Downloaded releases live in `csv-lsp-<tag>/` inside the work directory.
const VERSION_DIR_PREFIX: &str = "csv-lsp-";

struct CsvColumns {
    /// Downloaded binary resolved earlier in this session.
    cached_binary_path: Option<String>,
}

/// The release asset built for the current platform.
struct PlatformAsset {
    name: String,
    file_type: DownloadedFileType,
    binary_name: &'static str,
}

/// Asset names must match `.github/workflows/release.yml`.
fn platform_asset() -> Result<PlatformAsset> {
    let (os, arch) = zed::current_platform();
    let arch = match arch {
        Architecture::Aarch64 => "aarch64",
        Architecture::X8664 => "x86_64",
        Architecture::X86 => return Err(format!("{SERVER_NAME}: 32-bit x86 is not supported")),
    };
    let (target, extension, file_type, binary_name) = match os {
        Os::Mac => (
            "apple-darwin",
            "tar.gz",
            DownloadedFileType::GzipTar,
            "csv-lsp",
        ),
        Os::Linux => (
            "unknown-linux-musl",
            "tar.gz",
            DownloadedFileType::GzipTar,
            "csv-lsp",
        ),
        Os::Windows => (
            "pc-windows-msvc",
            "zip",
            DownloadedFileType::Zip,
            "csv-lsp.exe",
        ),
    };
    Ok(PlatformAsset {
        name: format!("{SERVER_NAME}-{arch}-{target}.{extension}"),
        file_type,
        binary_name,
    })
}

fn is_file(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// Names of the `csv-lsp-<tag>` directories currently in the work directory.
fn installed_version_dirs() -> Vec<String> {
    let Ok(entries) = fs::read_dir(".") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(VERSION_DIR_PREFIX))
        .collect()
}

fn set_status(id: &LanguageServerId, status: LanguageServerInstallationStatus) {
    zed::set_language_server_installation_status(id, &status);
}

impl CsvColumns {
    fn downloaded_binary_path(&mut self, id: &LanguageServerId) -> Result<String> {
        if let Some(path) = &self.cached_binary_path {
            if is_file(path) {
                return Ok(path.clone());
            }
        }

        let asset = platform_asset()?;
        set_status(id, LanguageServerInstallationStatus::CheckingForUpdate);
        let release = match zed::latest_github_release(
            GITHUB_REPO,
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        ) {
            Ok(release) => release,
            Err(err) => {
                // Offline or rate-limited: keep working with what's installed.
                let fallback = installed_version_dirs()
                    .into_iter()
                    .map(|dir| format!("{dir}/{}", asset.binary_name))
                    .find(|path| is_file(path));
                return match fallback {
                    Some(path) => {
                        set_status(id, LanguageServerInstallationStatus::None);
                        self.cached_binary_path = Some(path.clone());
                        Ok(path)
                    }
                    None => Err(format!(
                        "{SERVER_NAME}: could not fetch the latest release of {GITHUB_REPO}: {err}"
                    )),
                };
            }
        };

        let version_dir = format!("{VERSION_DIR_PREFIX}{}", release.version);
        let binary_path = format!("{version_dir}/{}", asset.binary_name);

        if !is_file(&binary_path) {
            let download = release
                .assets
                .iter()
                .find(|a| a.name == asset.name)
                .ok_or_else(|| {
                    format!(
                        "{SERVER_NAME}: release {} has no asset named {}",
                        release.version, asset.name
                    )
                })?;

            set_status(id, LanguageServerInstallationStatus::Downloading);
            // Clear leftovers from an interrupted download of this version.
            let _ = fs::remove_dir_all(&version_dir);
            zed::download_file(&download.download_url, &version_dir, asset.file_type)
                .map_err(|e| format!("{SERVER_NAME}: failed to download {}: {e}", asset.name))?;
            zed::make_file_executable(&binary_path)?;

            // Only remove our own version directories, nothing else.
            for dir in installed_version_dirs() {
                if dir != version_dir {
                    let _ = fs::remove_dir_all(&dir);
                }
            }
        }

        set_status(id, LanguageServerInstallationStatus::None);
        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl zed::Extension for CsvColumns {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<zed::Command> {
        let binary_settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.binary);
        let args = binary_settings
            .as_ref()
            .and_then(|b| b.arguments.clone())
            .unwrap_or_default();

        let mut env = worktree.shell_env();
        if let Some(extra) = binary_settings.as_ref().and_then(|b| b.env.clone()) {
            env.retain(|(key, _)| !extra.contains_key(key));
            env.extend(extra);
        }

        let command = if let Some(path) = binary_settings.and_then(|b| b.path) {
            path
        } else if let Some(path) = worktree.which(SERVER_NAME) {
            path
        } else {
            self.downloaded_binary_path(language_server_id)
                .inspect_err(|err| {
                    set_status(
                        language_server_id,
                        LanguageServerInstallationStatus::Failed(err.clone()),
                    );
                })?
        };

        Ok(zed::Command { command, args, env })
    }
}

zed::register_extension!(CsvColumns);
