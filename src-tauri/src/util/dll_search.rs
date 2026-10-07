//! Restrict where Windows looks for DLLs (DASA 1.5.2, safe library loading).
//!
//! By default Windows resolves a DLL loaded by name (and every dependency of a
//! DLL loaded by path) through a search order that ends in the current
//! directory and `PATH`, so a writable directory on either can plant a DLL the
//! app then runs. [`restrict_dll_search`] narrows that, for the rest of the
//! process, to the application directory, System32 and the directories added
//! with `AddDllDirectory`.
//!
//! What the app loads still resolves:
//! - `ggml-base.dll`, `ggml.dll`, `llama.dll`, `llama-common.dll` are static
//!   imports of `emailops.exe`, staged next to it (application directory).
//! - The dynamic ggml backend modules are loaded by absolute path from
//!   `<resources>/backends` (`ggml_backend_load_all_from_path`); their own
//!   imports are the base DLLs above (already loaded) and system DLLs such as
//!   `vulkan-1.dll` (System32), whose loader opens drivers by absolute path.
//! - The CUDA build's `ggml-cuda.dll` imports the CUDA runtime and cuBLAS,
//!   which are not bundled: they come from the user's CUDA toolkit, which until
//!   now was found through `PATH`. The toolkit's DLL directories are added
//!   explicitly from `CUDA_PATH` (set by the toolkit installer) instead.

use std::path::{Path, PathBuf};

/// Directories of an installed CUDA toolkit that hold its runtime DLLs:
/// `bin\x64` (CUDA 13) and `bin` (CUDA 12 and earlier), whichever exist.
///
/// Only an absolute `CUDA_PATH` is used: a relative one would resolve against
/// the current directory, the very location this module keeps out of the search.
#[cfg_attr(not(all(windows, feature = "cuda")), allow(dead_code))]
pub fn cuda_runtime_dll_dirs(cuda_path: Option<&Path>, is_dir: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let Some(root) = cuda_path.filter(|p| p.is_absolute()) else {
        return Vec::new();
    };
    let bin = root.join("bin");
    [bin.join("x64"), bin].into_iter().filter(|dir| is_dir(dir)).collect()
}

/// Apply the restricted DLL search order to the whole process. Call it first
/// thing in `main`, before anything can load a library.
#[cfg(windows)]
pub fn restrict_dll_search() -> std::io::Result<()> {
    use windows_sys::Win32::System::LibraryLoader::{SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS};

    // LOAD_LIBRARY_SEARCH_DEFAULT_DIRS = application directory + System32 +
    // directories added with AddDllDirectory.
    // SAFETY: takes a constant flag and touches no memory of ours.
    if unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    #[cfg(feature = "cuda")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::LibraryLoader::AddDllDirectory;

        let cuda_path = std::env::var_os("CUDA_PATH");
        for dir in cuda_runtime_dll_dirs(cuda_path.as_deref().map(Path::new), |p| p.is_dir()) {
            let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
            if unsafe { AddDllDirectory(wide.as_ptr()) }.is_null() {
                return Err(std::io::Error::last_os_error());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    type Case = (&'static str, Option<PathBuf>, Vec<PathBuf>, Vec<PathBuf>);

    /// An absolute toolkit root on the host running the test.
    fn root() -> PathBuf {
        PathBuf::from(if cfg!(windows) { r"C:\cuda" } else { "/cuda" })
    }

    #[test]
    fn cuda_runtime_dll_dirs_cases() {
        let bin = root().join("bin");
        let x64 = bin.join("x64");
        // (name, CUDA_PATH, directories that exist, expected result)
        let cases: Vec<Case> = vec![
            ("no CUDA_PATH", None, vec![bin.clone()], vec![]),
            (
                "empty CUDA_PATH",
                Some(PathBuf::new()),
                vec![PathBuf::from("bin")],
                vec![],
            ),
            (
                "relative CUDA_PATH",
                Some(PathBuf::from("cuda")),
                vec![PathBuf::from("cuda").join("bin")],
                vec![],
            ),
            ("CUDA 12 layout", Some(root()), vec![bin.clone()], vec![bin.clone()]),
            (
                "CUDA 13 layout",
                Some(root()),
                vec![bin.clone(), x64.clone()],
                vec![x64.clone(), bin.clone()],
            ),
            ("toolkit removed", Some(root()), vec![], vec![]),
        ];
        for (name, cuda_path, existing, want) in cases {
            let got = cuda_runtime_dll_dirs(cuda_path.as_deref(), |p| existing.iter().any(|d| d == p));
            assert_eq!(got, want, "{name}");
        }
    }
}
