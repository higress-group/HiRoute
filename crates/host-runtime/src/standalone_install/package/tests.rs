use super::*;
use sha2::{Digest, Sha256};

fn image(machine: u16) -> Vec<u8> {
    let mut bytes = vec![0; 70];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
    bytes[64..68].copy_from_slice(b"PE\0\0");
    bytes[68..70].copy_from_slice(&machine.to_le_bytes());
    bytes
}
pub(crate) fn fixture(target: &str, machine: u16) -> (tempfile::TempDir, Package) {
    let root = tempfile::tempdir().unwrap();
    let mut files = BTreeMap::new();
    for name in [
        "bin/hiroute.exe",
        "bin/hirouted.exe",
        "libexec/cliproxyapi.exe",
        "licenses/CLIProxyAPI-LICENSE",
        "licenses/HiRoute-LICENSE",
        "licenses/THIRD-PARTY-LICENSES.txt",
        "licenses/third-party-licenses.json",
        "skills/hiroute-management/SKILL.md",
        "docs/standalone-cli.md",
    ] {
        let executable = name.ends_with(".exe");
        let bytes = if executable {
            image(machine)
        } else {
            b"resource".to_vec()
        };
        let path = root.path().join(Package::relative(name));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &bytes).unwrap();
        files.insert(
            name.into(),
            PackageFile {
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                size: bytes.len() as u64,
                mode: if executable {
                    "executable"
                } else {
                    "read_only"
                }
                .into(),
            },
        );
    }
    let package = Package {
        schema: "hiroute.standalone-package/v1".into(),
        version: "0.3.1".into(),
        revision: "a".repeat(40),
        target: target.into(),
        archive: Archive {
            filename: "package.tar.gz".into(),
            sha256: "b".repeat(64),
            size: 1,
        },
        cpa: Cpa {
            version: "fixture".into(),
            sha256: files["libexec/cliproxyapi.exe"].sha256.clone(),
        },
        files,
        distribution: "integration-candidate".into(),
    };
    fs::write(
        root.path().join("package-manifest.json"),
        package.canonical().unwrap(),
    )
    .unwrap();
    (root, package)
}
#[test]
fn both_windows_targets_verify_complete_native_inventories() {
    for (target, machine) in [
        ("x86_64-pc-windows-msvc", 0x8664),
        ("aarch64-pc-windows-msvc", 0xaa64),
    ] {
        let (root, _) = fixture(target, machine);
        assert_eq!(Package::read(root.path()).unwrap().target, target);
    }
}
#[test]
fn declared_target_cannot_accept_the_other_executable_architecture() {
    for (target, machine) in [
        ("x86_64-pc-windows-msvc", 0xaa64),
        ("aarch64-pc-windows-msvc", 0x8664),
    ] {
        let (root, _) = fixture(target, machine);
        assert!(Package::read(root.path()).is_err());
    }
}
