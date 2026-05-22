use std::path::PathBuf;

fn main() {
    sync_diff_script();
    tauri_build::build()
}

/// Dev convenience: if the upstream copy of `diff_testcases.py` exists on this
/// machine, mirror it into `src-tauri/scripts/` before cargo compiles.
///
/// - On the dev box this means: edit the script in the diff/ project, run the
///   app, build automatically picks up the new version (no manual Copy-Item).
/// - On any other machine the upstream path won't exist, so we leave the
///   already-vendored `src-tauri/scripts/diff_testcases.py` untouched — the app
///   stays portable.
///
/// To override the upstream location, set `DIFF_SCRIPT_SRC=<path>` in env.
fn sync_diff_script() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendored = manifest_dir.join("scripts").join("diff_testcases.py");

    let upstream = std::env::var("DIFF_SCRIPT_SRC")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // Default: sibling repo on the dev box
            PathBuf::from(r"C:\Users\chenj\Documents\trae_projects\diff\scripts\diff_testcases.py")
        });

    // Always re-run if the vendored file changes (so include_str! invalidates).
    println!("cargo:rerun-if-changed={}", vendored.display());

    if !upstream.is_file() {
        // No upstream → just rely on whatever is already vendored.
        return;
    }

    // Re-run when upstream changes too.
    println!("cargo:rerun-if-changed={}", upstream.display());

    let need_copy = match (
        std::fs::read(&upstream).ok(),
        std::fs::read(&vendored).ok(),
    ) {
        (Some(a), Some(b)) => a != b,
        (Some(_), None) => true,
        _ => false,
    };

    if need_copy {
        if let Some(parent) = vendored.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::copy(&upstream, &vendored) {
            // Don't fail the whole build — fall back to vendored copy.
            println!(
                "cargo:warning=Failed to sync diff_testcases.py from {}: {}",
                upstream.display(),
                e
            );
        } else {
            println!(
                "cargo:warning=Synced diff_testcases.py from {} -> {}",
                upstream.display(),
                vendored.display()
            );
        }
    }
}
