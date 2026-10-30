use super::*;
use hiroute_platform::windows::security;
use hiroute_platform_test_support::junction::create as junction;

struct NpmFixture {
    _root: crate::agents::probe_files::PrivateProbeRoot,
    bin: PathBuf,
    wrapper: PathBuf,
    native: PathBuf,
    marker: PathBuf,
}
impl NpmFixture {
    fn new(local: bool) -> Self {
        let root = crate::agents::probe_files::temporary("hiroute-opencode-npm-").unwrap();
        let prefix = root.path().join("npm prefix 中文 ' & literal");
        let modules = prefix.join("node_modules");
        let wrapper = modules.join("opencode-ai");
        let bin = if local { modules.join(".bin") } else { prefix };
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(wrapper.join("bin")).unwrap();
        let marker = root.path().join("shim-must-not-run");
        std::fs::write(
            bin.join("opencode.cmd"),
            format!("@echo forbidden > \"{}\"\r\n", marker.display()),
        )
        .unwrap();
        std::fs::write(
            wrapper.join("bin/opencode"),
            b"throw new Error('must not run')",
        )
        .unwrap();
        let cpu = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        let name = format!("opencode-windows-{cpu}");
        let platform = modules.join(&name);
        std::fs::create_dir_all(platform.join("bin")).unwrap();
        let native = platform.join("bin/opencode.exe");
        std::fs::write(&native, b"unexecuted native fixture").unwrap();
        std::fs::write(
            platform.join("package.json"),
            serde_json::json!({"name":name,"version":"1.19.3","os":["win32"],"cpu":[cpu]})
                .to_string(),
        )
        .unwrap();
        std::fs::write(wrapper.join("package.json"), serde_json::json!({"name":"opencode-ai","version":"1.19.3","bin":{"opencode":"bin/opencode"},"optionalDependencies":{(name):"1.19.3"}}).to_string()).unwrap();
        Self {
            _root: root,
            bin,
            wrapper,
            native,
            marker,
        }
    }

    fn assert_same_native(selected: &Path, expected: &Path) {
        let resolved = opencode_native_executable(selected).unwrap().unwrap();
        assert_eq!(
            security::file_identity(&security::open_identity(&resolved, false).unwrap()).unwrap(),
            security::file_identity(&security::open_identity(expected, false).unwrap()).unwrap()
        );
    }
}

#[test]
fn windows_opencode_npm_shim_path_and_native_share_one_identity_without_execution() {
    const CHILD: &str = "HIROUTE_TEST_OPENCODE_NPM_BIN";
    if let Some(bin) = std::env::var_os(CHILD) {
        let bin = PathBuf::from(bin);
        let expected = std::env::var_os("HIROUTE_TEST_OPENCODE_NATIVE").unwrap();
        NpmFixture::assert_same_native(Path::new("opencode"), Path::new(&expected));
        assert!(bin.join("opencode.cmd").is_file());
        return;
    }
    for local in [false, true] {
        let fixture = NpmFixture::new(local);
        for selected in [
            fixture.bin.join("opencode.cmd"),
            fixture.bin.join("opencode"),
            fixture.native.clone(),
        ] {
            NpmFixture::assert_same_native(&selected, &fixture.native);
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args(["--exact", "agents::opencode_installation::windows_tests::windows_opencode_npm_shim_path_and_native_share_one_identity_without_execution", "--nocapture"])
            .env(CHILD, &fixture.bin).env("HIROUTE_TEST_OPENCODE_NATIVE", &fixture.native)
            .env("PATH", &fixture.bin);
        super::super::windows_runtime::bounded(
            &child,
            true,
            std::time::Duration::from_secs(15),
            64 * 1024,
        )
        .unwrap();
        assert!(!fixture.marker.exists());
    }
}

#[test]
fn windows_opencode_npm_rejects_foreign_producer_and_original_reparse_paths() {
    let fixture = NpmFixture::new(false);
    let manifest = fixture.wrapper.join("package.json");
    let original = std::fs::read(&manifest).unwrap();
    std::fs::write(
        &manifest,
        br#"{"name":"foreign-producer","bin":{"opencode":"bin/opencode"}}"#,
    )
    .unwrap();
    assert!(opencode_native_executable(&fixture.bin.join("opencode.cmd")).is_err());
    std::fs::write(&manifest, original).unwrap();
    let moved = fixture.wrapper.with_file_name("wrapper-original");
    std::fs::rename(&fixture.wrapper, &moved).unwrap();
    junction(&fixture.wrapper, &moved).unwrap();
    assert!(opencode_native_executable(&fixture.bin.join("opencode.cmd")).is_err());
    std::fs::remove_dir(&fixture.wrapper).unwrap();
    std::fs::rename(moved, &fixture.wrapper).unwrap();
    NpmFixture::assert_same_native(&fixture.bin.join("opencode.cmd"), &fixture.native);
    let bin = fixture.wrapper.join("bin");
    let original_bin = fixture.wrapper.join("original-bin");
    std::fs::rename(&bin, &original_bin).unwrap();
    junction(&bin, &original_bin).unwrap();
    assert!(opencode_native_executable(&fixture.bin.join("opencode.cmd")).is_err());
    std::fs::remove_dir(bin).unwrap();
    std::fs::rename(original_bin, fixture.wrapper.join("bin")).unwrap();
}

#[cfg(target_arch = "x86_64")]
#[test]
fn windows_opencode_npm_ambiguous_platform_packages_require_native_selection() {
    let fixture = NpmFixture::new(false);
    let original = fixture.native.parent().unwrap().parent().unwrap();
    let name = original.file_name().unwrap().to_str().unwrap().to_owned();
    let duplicate = original.parent().unwrap().join(format!("{name}-baseline"));
    std::fs::create_dir_all(duplicate.join("bin")).unwrap();
    std::fs::copy(&fixture.native, duplicate.join("bin/opencode.exe")).unwrap();
    let mut metadata: Value =
        serde_json::from_slice(&std::fs::read(original.join("package.json")).unwrap()).unwrap();
    metadata["name"] = format!("{name}-baseline").into();
    std::fs::write(duplicate.join("package.json"), metadata.to_string()).unwrap();
    let manifest = fixture.wrapper.join("package.json");
    let mut metadata: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    metadata["optionalDependencies"][format!("{name}-baseline")] = "1.19.3".into();
    std::fs::write(manifest, metadata.to_string()).unwrap();
    assert!(opencode_native_executable(&fixture.bin.join("opencode.cmd")).is_err());
    NpmFixture::assert_same_native(&fixture.native, &fixture.native);
}
