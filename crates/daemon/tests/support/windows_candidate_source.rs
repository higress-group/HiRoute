//! Relocate a compiled native gate without executing a different installer revision.
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

use hiroute_platform::windows::security;
use sha2::{Digest, Sha256};

const PACKAGER: &[u8] = include_bytes!("../../../../scripts/package-standalone.py");

pub struct Sources {
    // Deny mutation/removal through the complete scenario, including owner reexec.
    _held: Vec<File>,
}

impl Sources {
    pub fn open() -> Self {
        let relocated = std::env::var_os("HIROUTE_WINDOWS_SOURCE_ROOT");
        if std::env::var_os("HIROUTE_WINDOWS_EXECUTION_HOST").is_some() {
            assert!(
                relocated.is_some(),
                "artifact replay requires an explicit source root"
            );
        }
        let root = match relocated {
            Some(path) => {
                let root = PathBuf::from(path);
                assert!(root.is_absolute(), "candidate source root must be absolute");
                assert!(
                    !root
                        .components()
                        .any(|part| matches!(part, Component::ParentDir)),
                    "candidate source root cannot contain parent traversal"
                );
                assert!(
                    Self::revision().is_some(),
                    "relocated gates require a compiled candidate SHA"
                );
                root
            }
            None => Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .unwrap(),
        };
        // Explicitly reject a replaced/junction source ancestor before opening files.
        let packager = root.join("scripts/package-standalone.py");
        let mut held = packager
            .parent()
            .unwrap()
            .ancestors()
            .map(retain_directory)
            .collect::<Vec<_>>();
        held.push(exact_source(
            &root.join("scripts/package-standalone.py"),
            PACKAGER,
        ));
        Self { _held: held }
    }

    pub fn revision() -> Option<&'static str> {
        let compiled = option_env!("HIROUTE_BUILD_SOURCE_SHA");
        if let Some(compiled) = compiled {
            assert!(
                compiled.len() == 40
                    && compiled
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
                "compiled gate candidate SHA is invalid"
            );
            assert_eq!(
                std::env::var("HIROUTE_BUILD_SOURCE_SHA").ok().as_deref(),
                Some(compiled),
                "runtime and compiled gate candidate SHA differ"
            );
        }
        compiled
    }
}

fn retain_directory(path: &Path) -> File {
    let identity = security::open_identity(path, true).unwrap();
    let held = OpenOptions::new()
        .read(true)
        // FILE_SHARE_READ | FILE_SHARE_WRITE, deliberately no FILE_SHARE_DELETE.
        .share_mode(3)
        // FILE_FLAG_BACKUP_SEMANTICS opens directory objects, not their contents.
        .custom_flags(0x0200_0000)
        .open(path)
        .unwrap();
    assert_eq!(
        security::file_identity(&identity).unwrap(),
        security::file_identity(&held).unwrap(),
        "candidate source ancestor changed while retaining its object"
    );
    held
}

fn exact_source(path: &Path, expected: &[u8]) -> File {
    let identity = security::open_identity(path, false).unwrap();
    let mut held = OpenOptions::new()
        .read(true)
        // FILE_SHARE_READ only: The packager may read, nobody can replace or write.
        .share_mode(1)
        .open(path)
        .unwrap();
    assert_eq!(
        security::file_identity(&identity).unwrap(),
        security::file_identity(&held).unwrap(),
        "candidate source changed while retaining its object"
    );
    assert_eq!(held.metadata().unwrap().len(), expected.len() as u64);
    let mut actual = Vec::new();
    (&mut held)
        .take(expected.len() as u64 + 1)
        .read_to_end(&mut actual)
        .unwrap();
    assert_eq!(
        Sha256::digest(&actual),
        Sha256::digest(expected),
        "candidate package producer bytes differ from the compiled source"
    );
    held
}
