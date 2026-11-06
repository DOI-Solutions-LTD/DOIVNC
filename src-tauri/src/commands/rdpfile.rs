//! Importing Microsoft `.rdp` files (PRDRDP/08 §5.4).
//!
//! The parser (`vnc_store::parse_rdp_file`) is pure and takes bytes, so the
//! split of work is: the webview sends a PATH, the shell reads the file, and
//! the draft comes back for the host editor to show. Content never travels
//! webview to shell.
//!
//! That direction matters. A `.rdp` file may carry a `password 51:b:` line,
//! which is a DPAPI blob rather than a plaintext password but is still a
//! secret; keeping the bytes on the Rust side means one fewer place it can be
//! logged, and it means the size cap is enforced before anything is allocated
//! rather than after the webview has already read a gigabyte off disk.
//!
//! Nothing here writes. The draft goes to the editor and the user saves it
//! through the ordinary `save_host`, so an import is reviewable before it
//! becomes a profile, and an imported profile's password is still only stored
//! after a server has accepted it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};
use vnc_store::{RdpImport, MAX_RDP_FILE_BYTES};

/// Emitted to the library window when `.rdp` files are waiting in
/// [`PendingRdpFiles`]. It carries nothing: the library answers it with
/// [`take_pending_rdp_files`], which is also what it calls once on mount, so a
/// file passed at launch, before any listener existed, is not lost.
pub const OPEN_EVENT: &str = "rdp-file://open";

/// `.rdp` files the app was asked to open, by a double-click, "Open with" or
/// the command line, that the library has not picked up yet.
///
/// A queue rather than an event payload because the first launch's files
/// arrive before the webview is listening; the second launch's are forwarded
/// by the single-instance plugin into the running app and take the same road.
#[derive(Default)]
pub struct PendingRdpFiles(Mutex<Vec<String>>);

/// The `.rdp` paths among a process's arguments, made absolute.
///
/// `args` excludes the executable itself. A relative path is resolved against
/// `cwd`, which matters for the forwarded arguments of a second launch: they
/// were relative to THAT process's directory, not this one's. Anything that is
/// not an existing `*.rdp` file is skipped, so the app's own flags (the
/// autostart `--background`, say) never get here.
pub fn rdp_paths_from_args<I>(args: I, cwd: Option<&Path>) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    args.into_iter()
        .filter(|a| {
            Path::new(a)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("rdp"))
        })
        .map(|a| {
            let p = PathBuf::from(&a);
            match cwd {
                Some(dir) if p.is_relative() => dir.join(p),
                _ => p,
            }
        })
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// Queue `paths` for the library and tell it they are there.
pub fn queue_for_library(app: &AppHandle, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    let Some(pending) = app.try_state::<PendingRdpFiles>() else {
        tracing::warn!("rdp files arrived before the pending queue existed; dropping them");
        return;
    };
    if let Ok(mut queue) = pending.0.lock() {
        queue.extend(paths);
    }
    let _ = app.emit_to("main", OPEN_EVENT, ());
}

/// Hand the library every `.rdp` path waiting to be opened, and forget them.
#[tauri::command]
pub fn take_pending_rdp_files(pending: tauri::State<'_, PendingRdpFiles>) -> Vec<String> {
    pending
        .0
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

/// One file's result in a multi-file import.
///
/// A batch never fails as a whole: one refused file among ten must not lose
/// the other nine, so each row carries either a draft or the reason there is
/// none, and the UI lists both.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpFileImport {
    /// The path as given, so the UI can name the row it is reporting on.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<RdpImport>,
    /// Why there is no draft. A sentence for the user, never a debug dump.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Read one file, with the size cap applied before the read.
///
/// `metadata` first rather than reading and measuring afterwards: the cap
/// exists so a hostile or mistaken path cannot make the shell allocate an
/// arbitrary amount, and checking after the read would defeat it.
fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if meta.len() > MAX_RDP_FILE_BYTES as u64 {
        return Err(format!(
            "{} is too large to be a Remote Desktop file ({} bytes).",
            path.display(),
            meta.len()
        ));
    }
    std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))
}

/// Parse one `.rdp` file into a draft host profile.
fn import_one(path: &Path) -> Result<RdpImport, String> {
    let bytes = read_capped(path)?;
    let file_name = path.file_name().and_then(|n| n.to_str());
    vnc_store::parse_rdp_file(&bytes, file_name).map_err(|e| e.to_string())
}

/// Read a `.rdp` file and return the draft profile it describes.
///
/// Errors only for a file that must not become a profile at all: one that is
/// unreadable, one over the size cap, and one that launches a RemoteApp,
/// which does something materially different from opening a desktop. Anything
/// else that could not be carried across comes back as a warning on an
/// otherwise usable draft, because a file the user chose to import should not
/// be refused over one setting this app does not have.
#[tauri::command]
pub async fn import_rdp_file(path: String) -> Result<RdpImport, String> {
    // Filesystem IO is synchronous.
    tokio::task::spawn_blocking(move || import_one(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

/// The same for a multi-selection, one row per path, in the order given.
#[tauri::command]
pub async fn import_rdp_files(paths: Vec<String>) -> Result<Vec<RdpFileImport>, String> {
    tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| {
                let buf = PathBuf::from(&path);
                match import_one(&buf) {
                    Ok(import) => RdpFileImport {
                        path,
                        import: Some(import),
                        error: None,
                    },
                    Err(error) => RdpFileImport {
                        path,
                        import: None,
                        error: Some(error),
                    },
                }
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(dir: &Path, name: &str, body: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(body).expect("write");
        path
    }

    #[test]
    fn a_file_becomes_a_draft_named_after_itself() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(
            dir.path(),
            "Office PC.rdp",
            b"full address:s:office.corp.example\nusername:s:CORP\\alice\n",
        );
        let import = import_one(&path).expect("a plain file imports");
        assert_eq!(import.profile.address, "office.corp.example");
        assert_eq!(import.profile.protocol, "rdp");
        assert_eq!(import.profile.friendly_name, "Office PC");
    }

    /// The cap is enforced from the file's metadata, before the read, so an
    /// oversized file is never allocated.
    #[test]
    fn an_oversized_file_is_refused_before_it_is_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(dir.path(), "huge.rdp", &vec![b'x'; MAX_RDP_FILE_BYTES + 1]);
        let err = import_one(&path).expect_err("over the cap");
        assert!(err.contains("too large"), "{err}");
    }

    /// Only existing `.rdp` files come out of a command line, whatever the
    /// case of the extension, and a relative one is resolved against the
    /// directory the launching process was in.
    #[test]
    fn rdp_paths_are_picked_out_of_the_arguments() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = write(dir.path(), "a.rdp", b"full address:s:a\n");
        write(dir.path(), "B.RDP", b"full address:s:b\n");
        let args = vec![
            "--background".to_owned(),
            a.to_string_lossy().into_owned(),
            "B.RDP".to_owned(),
            "missing.rdp".to_owned(),
            "notes.txt".to_owned(),
        ];
        let got = rdp_paths_from_args(args, Some(dir.path()));
        assert_eq!(
            got,
            vec![
                a.to_string_lossy().into_owned(),
                dir.path().join("B.RDP").to_string_lossy().into_owned(),
            ]
        );
    }

    #[test]
    fn a_missing_file_is_a_sentence_not_a_panic() {
        let err = import_one(Path::new("/no/such/file.rdp")).expect_err("missing");
        assert!(err.contains("could not open"), "{err}");
    }
}
