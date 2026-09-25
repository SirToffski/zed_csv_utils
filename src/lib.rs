use zed_extension_api as zed;

struct RainbowCsvAlign;

impl zed::Extension for RainbowCsvAlign {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        // Dev flow: `cargo build -p csv_lsp` then ensure `csv-lsp` is on PATH,
        // or place the binary next to the worktree. Publishing flow should
        // download prebuilt per-OS binaries via `download_file` instead.
        let cmd = worktree
            .which("csv-lsp")
            .ok_or_else(|| {
                "csv-lsp not found on PATH. Build with `cargo build -p csv_lsp` and add target/debug to PATH.".to_string()
            })?;
        Ok(zed::Command {
            command: cmd,
            args: vec![],
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(RainbowCsvAlign);
