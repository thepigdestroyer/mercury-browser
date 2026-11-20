// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Where the PDFium library is.
//!
//! [`locate_library`] looks, in this order:
//!
//! 1. the environment variable `MERCURY_PDFIUM_LIBRARY` (a file). **If it is set and not
//!    empty, nothing else is looked at**: a path that does not exist means "no viewer", which is
//!    how a test (or a person) turns the viewer off;
//! 2. `<directory of the executable>/lib/libpdfium.dylib` (`.so` on Linux): the app bundle's
//!    `Contents/MacOS/lib` and the build's `target/release/lib`;
//! 3. on macOS, the cache that `scripts/macos/pdfium.sh` fills:
//!    `~/Library/Caches/mercury-browser/pdfium/<release>/<arch>/lib/libpdfium.dylib`, where
//!    `<release>` is the number of the `chromium/<release>` tag of the pinned PDFium build
//!    ([`PDFIUM_RELEASE`]) and `<arch>` is `arm64` or `x64`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The environment variable that names the library.
pub const LIBRARY_VARIABLE: &str = "MERCURY_PDFIUM_LIBRARY";

/// The PDFium release the pin script fetches (the number in `chromium/<release>`).
pub const PDFIUM_RELEASE: &str = "8165";

/// The file name of the library on this platform.
pub const LIBRARY_FILE: &str = if cfg!(target_os = "macos") {
    "libpdfium.dylib"
} else {
    "libpdfium.so"
};

/// The architecture folder of the cache: `arm64` or `x64`.
pub const fn cache_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    }
}

/// The places [`locate_library`] reads, so the order can be tested.
#[derive(Clone, Debug, Default)]
pub struct Search {
    /// The value of `MERCURY_PDFIUM_LIBRARY`.
    pub variable: Option<OsString>,
    /// The directory of the running executable.
    pub executable_dir: Option<PathBuf>,
    /// The home directory (for the macOS cache).
    pub home: Option<PathBuf>,
}

impl Search {
    /// The real places.
    pub fn from_environment() -> Self {
        Self {
            variable: std::env::var_os(LIBRARY_VARIABLE),
            executable_dir: std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().map(Path::to_path_buf)),
            home: std::env::var_os("HOME").map(PathBuf::from),
        }
    }

    /// The first library that exists, by the rules in the module documentation.
    pub fn find(&self) -> Option<PathBuf> {
        if let Some(variable) = self.variable.as_ref().filter(|value| !value.is_empty()) {
            let path = PathBuf::from(variable);
            return path.is_file().then_some(path);
        }
        let beside_executable = self
            .executable_dir
            .as_ref()
            .map(|dir| dir.join("lib").join(LIBRARY_FILE));
        let in_cache = cfg!(target_os = "macos")
            .then(|| {
                self.home.as_ref().map(|home| {
                    home.join("Library/Caches/mercury-browser/pdfium")
                        .join(PDFIUM_RELEASE)
                        .join(cache_arch())
                        .join("lib")
                        .join(LIBRARY_FILE)
                })
            })
            .flatten();
        [beside_executable, in_cache]
            .into_iter()
            .flatten()
            .find(|path| path.is_file())
    }
}

/// The PDFium library, if there is one (see the module documentation).
pub fn locate_library() -> Option<PathBuf> {
    Search::from_environment().find()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mercury-pdfium-library-test-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_variable_wins_and_must_name_a_file() {
        let scratch = Scratch::new("variable");
        let library = scratch.file("anywhere/lib.dylib");
        let beside = scratch.file(&format!("exe/lib/{LIBRARY_FILE}"));
        let search = Search {
            variable: Some(library.clone().into_os_string()),
            executable_dir: Some(scratch.0.join("exe")),
            home: None,
        };
        assert_eq!(search.find(), Some(library));
        assert!(beside.is_file());
    }

    #[test]
    fn a_variable_that_names_nothing_means_no_viewer() {
        let scratch = Scratch::new("missing");
        scratch.file(&format!("exe/lib/{LIBRARY_FILE}"));
        let search = Search {
            variable: Some(OsString::from("/nonexistent/libpdfium.dylib")),
            executable_dir: Some(scratch.0.join("exe")),
            home: None,
        };
        assert_eq!(search.find(), None, "the other places are not tried");
    }

    #[test]
    fn an_empty_variable_is_the_same_as_none() {
        let scratch = Scratch::new("empty");
        let beside = scratch.file(&format!("exe/lib/{LIBRARY_FILE}"));
        let search = Search {
            variable: Some(OsString::new()),
            executable_dir: Some(scratch.0.join("exe")),
            home: None,
        };
        assert_eq!(search.find(), Some(beside));
    }

    #[test]
    fn the_executable_directory_comes_before_the_cache() {
        let scratch = Scratch::new("order");
        let beside = scratch.file(&format!("exe/lib/{LIBRARY_FILE}"));
        scratch.file(&format!(
            "home/Library/Caches/mercury-browser/pdfium/{PDFIUM_RELEASE}/{}/lib/{LIBRARY_FILE}",
            cache_arch()
        ));
        let search = Search {
            variable: None,
            executable_dir: Some(scratch.0.join("exe")),
            home: Some(scratch.0.join("home")),
        };
        assert_eq!(search.find(), Some(beside));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_cache_layout_is_release_then_arch() {
        let scratch = Scratch::new("cache");
        let cached = scratch.file(&format!(
            "home/Library/Caches/mercury-browser/pdfium/{PDFIUM_RELEASE}/{}/lib/libpdfium.dylib",
            cache_arch()
        ));
        let search = Search {
            variable: None,
            executable_dir: Some(scratch.0.join("exe-without-lib")),
            home: Some(scratch.0.join("home")),
        };
        assert_eq!(search.find(), Some(cached));
        assert_eq!(
            Search::default().find(),
            None,
            "with nothing to look at there is nothing"
        );
    }
}
