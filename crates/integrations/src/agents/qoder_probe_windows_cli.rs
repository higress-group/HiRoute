//! Account-free Git Bash command controls with the actual candidate CLI.
//! This does not run Qoder, change its shell selection or borrow a user profile.
//! The compiler triplet is advisory: the fixed KnownFolder Git wrapper, actual
//! shell, Windows Git and MSYS DLL stay held through all commands. Both import
//! tables must identify MSYS without Cygwin; original FI/permissions/bytes remain
//! unchanged. This test does not provide a general-purpose Cygwin admission rule.
#[cfg(test)]
mod tests {
    use super::super::*;
    use hiroute_platform::{file_security, windows::security};
    use std::io::{Read, Seek, SeekFrom};

    // These fixed facts describe the first captured line; they never authorize
    // a shell or retain its arbitrary version text. Unknown remains unknown.
    #[derive(Debug, Eq, PartialEq)]
    struct VersionFacts {
        bytes: usize,
        lines: usize,
        utf8: bool,
        empty: bool,
        first_line_empty: bool,
        gnu_header: bool,
        gnu_header_present: bool,
        msys_suffix: bool,
        triplet: &'static str,
        bom: bool,
        nul: bool,
    }

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct RuntimeImports {
        msys: bool,
        cygwin: bool,
    }

    // A compiler triplet is not a distribution identity. Git SDK 7f70a71d uses
    // x86_64-pc-cygwin while its runtime package still exports msys-2.0.dll.
    // This narrow, test-only reader checks both ordinary and delayed imports;
    // the package/artifact PE producer remains windows-native-runtime-audit.py.
    fn runtime_imports(data: &[u8]) -> Option<RuntimeImports> {
        fn u16_at(data: &[u8], at: usize) -> Option<u16> {
            Some(u16::from_le_bytes(
                data.get(at..at.checked_add(2)?)?.try_into().ok()?,
            ))
        }
        fn u32_at(data: &[u8], at: usize) -> Option<u32> {
            Some(u32::from_le_bytes(
                data.get(at..at.checked_add(4)?)?.try_into().ok()?,
            ))
        }
        if data.get(..2)? != b"MZ" {
            return None;
        }
        let nt = usize::try_from(u32_at(data, 60)?).ok()?;
        if nt < 64
            || data.get(nt..nt.checked_add(4)?)? != b"PE\0\0"
            || u16_at(data, nt + 4)? != 0x8664
        {
            return None;
        }
        let count = usize::from(u16_at(data, nt + 6)?);
        let optional_size = usize::from(u16_at(data, nt + 20)?);
        let optional = nt.checked_add(24)?;
        if !(1..=96).contains(&count) || optional_size < 112 || u16_at(data, optional)? != 0x20b {
            return None;
        }
        let image_base =
            u64::from_le_bytes(data.get(optional + 24..optional + 32)?.try_into().ok()?);
        let headers = usize::try_from(u32_at(data, optional + 60)?).ok()?;
        let directories = usize::try_from(u32_at(data, optional + 108)?).ok()?;
        let sections = optional.checked_add(optional_size)?;
        if directories > 16
            || 112 + directories * 8 > optional_size
            || sections.checked_add(count * 40)? > headers
            || headers > data.len()
        {
            return None;
        }
        let mut ranges = Vec::new();
        for index in 0..count {
            let at = sections + index * 40;
            let virtual_size = usize::try_from(u32_at(data, at + 8)?).ok()?;
            let address = usize::try_from(u32_at(data, at + 12)?).ok()?;
            let size = usize::try_from(u32_at(data, at + 16)?).ok()?;
            let offset = usize::try_from(u32_at(data, at + 20)?).ok()?;
            let end = address.checked_add(virtual_size.max(size))?;
            if address < headers
                || (size > 0 && (offset < headers || offset.checked_add(size)? > data.len()))
                || end > u32::MAX as usize
                || ranges
                    .iter()
                    .any(|&(start, other_end, _, _)| address < other_end && start < end)
            {
                return None;
            }
            ranges.push((address, end, offset, size));
        }
        let mapped = |address: usize, size: usize| {
            let end = address.checked_add(size)?;
            if address == 0 || size == 0 {
                return None;
            }
            if end <= headers {
                return Some((address, headers - address));
            }
            let mut found = ranges.iter().filter_map(|&(start, _, raw, length)| {
                if start <= address && end <= start.checked_add(length)? {
                    Some((raw + address - start, length - (address - start)))
                } else {
                    None
                }
            });
            let result = found.next()?;
            found.next().is_none().then_some(result)
        };
        let mut result = RuntimeImports::default();
        for (index, width) in [(1, 20), (13, 32)] {
            if index >= directories {
                continue;
            }
            let entry = optional + 112 + index * 8;
            let address = usize::try_from(u32_at(data, entry)?).ok()?;
            let size = usize::try_from(u32_at(data, entry + 4)?).ok()?;
            if address == 0 && size == 0 {
                continue;
            }
            if size < width || size > 1024 * 1024 {
                return None;
            }
            let (offset, _) = mapped(address, size)?;
            let mut terminated = false;
            for row in 0..size / width {
                let at = offset + row * width;
                let descriptor = data.get(at..at + width)?;
                if descriptor.iter().all(|&byte| byte == 0) {
                    terminated = true;
                    break;
                }
                let name = if index == 1 {
                    if u32_at(data, at + 16)? == 0 {
                        return None;
                    }
                    u64::from(u32_at(data, at + 12)?)
                } else {
                    let attributes = u32_at(data, at)?;
                    if attributes & !1 != 0 || u32_at(data, at + 12)? == 0 {
                        return None;
                    }
                    let value = u64::from(u32_at(data, at + 4)?);
                    if attributes & 1 == 0 {
                        value.checked_sub(image_base)?
                    } else {
                        value
                    }
                };
                let (offset, remaining) = mapped(usize::try_from(name).ok()?, 1)?;
                let bytes = data.get(offset..offset.checked_add(remaining.min(256))?)?;
                let end = bytes.iter().position(|&byte| byte == 0)?;
                let name = std::str::from_utf8(&bytes[..end]).ok()?;
                if !name.to_ascii_lowercase().ends_with(".dll")
                    || !name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
                {
                    return None;
                }
                result.msys |= name.eq_ignore_ascii_case("msys-2.0.dll");
                result.cygwin |= name.eq_ignore_ascii_case("cygwin1.dll");
            }
            if !terminated {
                return None;
            }
        }
        Some(result)
    }

    fn msys_banner(facts: &VersionFacts, imports: RuntimeImports) -> bool {
        facts.utf8
            && facts.gnu_header
            && !facts.bom
            && !facts.nul
            && matches!(facts.triplet, "msys-x64" | "cygwin-x64")
            && imports.msys
            && !imports.cygwin
    }

    fn windows_git_version(text: &str) -> bool {
        let Some((version, release)) = text
            .strip_prefix("git version ")
            .and_then(|text| text.split_once(".windows."))
        else {
            return false;
        };
        let numbers: Vec<_> = version.split('.').collect();
        (2..=4).contains(&numbers.len())
            && numbers.into_iter().chain([release]).all(|part| {
                !part.is_empty()
                    && part.len() <= 3
                    && part.bytes().all(|byte| byte.is_ascii_digit())
            })
    }

    fn runtime_identity_controls() {
        fn pe(name: &[u8], delayed_cygwin: bool) -> Vec<u8> {
            fn put16(data: &mut [u8], at: usize, value: u16) {
                data[at..at + 2].copy_from_slice(&value.to_le_bytes());
            }
            fn put32(data: &mut [u8], at: usize, value: u32) {
                data[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            let mut data = vec![0; 1024];
            data[..2].copy_from_slice(b"MZ");
            put32(&mut data, 60, 128);
            data[128..132].copy_from_slice(b"PE\0\0");
            put16(&mut data, 132, 0x8664);
            put16(&mut data, 134, 1);
            put16(&mut data, 148, 240);
            put16(&mut data, 152, 0x20b);
            data[176..184].copy_from_slice(&0x140000000_u64.to_le_bytes());
            put32(&mut data, 212, 512);
            put32(&mut data, 260, 16);
            put32(&mut data, 272, 0x1000);
            put32(&mut data, 276, 40);
            for (at, value) in [
                (400, 512),
                (404, 0x1000),
                (408, 512),
                (412, 512),
                (524, 0x1080),
                (528, 0x10a0),
            ] {
                put32(&mut data, at, value);
            }
            data[640..640 + name.len()].copy_from_slice(name);
            if delayed_cygwin {
                for (at, value) in [
                    (368, 0x10c0),
                    (372, 64),
                    (704, 1),
                    (708, 0x1140),
                    (716, 0x1160),
                ] {
                    put32(&mut data, at, value);
                }
                data[832..844].copy_from_slice(b"cygwin1.dll\0");
            }
            data
        }
        let official = version_facts(b"GNU bash, version 5.3.3(1)-release (x86_64-pc-cygwin)\n");
        assert!(
            official.gnu_header && !official.msys_suffix,
            "the former banner-only gate rejects the observed official build"
        );
        let image = pe(b"msys-2.0.dll\0", false);
        let imports = runtime_imports(&image).unwrap();
        assert!(msys_banner(&official, imports));
        for bytes in [
            pe(b"cygwin1.dll\0", false),
            pe(b"msys-2.0.dll\0", true),
            pe(b"kernel32.dll\0", false),
        ] {
            assert!(!msys_banner(&official, runtime_imports(&bytes).unwrap()));
        }
        for length in [0, 64, 511, 640] {
            assert!(runtime_imports(&image[..length]).is_none());
        }
        let mut malformed = image.clone();
        malformed[276..280].copy_from_slice(&20_u32.to_le_bytes());
        assert!(
            runtime_imports(&malformed).is_none(),
            "unterminated table cannot supply runtime identity"
        );
        let mut malformed = image.clone();
        malformed[524..528].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            runtime_imports(&malformed).is_none(),
            "foreign RVA cannot supply runtime identity"
        );
        assert!(!msys_banner(
            &version_facts(b"GNU bash, version 5 (x86_64-pc-linux-gnu)\n"),
            imports
        ));
        assert!(windows_git_version("git version 2.55.0.windows.5"));
        for value in [
            "git version 2.55.0",
            "git version 2.55.0.windows.5\nextra",
            "arbitrary windows.5",
        ] {
            assert!(!windows_git_version(value));
        }
    }

    struct GitDistribution {
        images: Vec<security::ReadLease>,
        descriptors: Vec<hiroute_platform::file_security::WindowsPermissions>,
        digests: Vec<CanonicalDigest>,
        shell: PathBuf,
        git: PathBuf,
        imports: RuntimeImports,
    }

    impl GitDistribution {
        fn retain(bash: &Path, selected: &File) -> Self {
            let root = hiroute_platform::windows::background::program_files()
                .unwrap()
                .join("Git");
            let wrapper = root.join("bin/bash.exe");
            // KnownFolder, original no-reparse inspection and actual FileID
            // bind this explicit test input to the fixed installed wrapper.
            assert!(
                super::super::super::managed_launch::windows_executable_path(&wrapper).unwrap()
                    == bash,
                "explicit shell must be the fixed Git installation wrapper"
            );
            let shell = root.join("usr/bin/bash.exe");
            let git = root.join("cmd/git.exe");
            let locations = [&wrapper, &shell, &git, &root.join("usr/bin/msys-2.0.dll")];
            let images: Vec<_> = locations
                .iter()
                .map(|path| {
                    security::open_installed_image_lease(path).unwrap_or_else(|error| {
                        let links = security::open_identity(path, false)
                            .and_then(|file| security::file_link_count(&file));
                        panic!(
                            "fixed Git image {:?}: read lease failed: {error}; link count: {links:?}",
                            path.file_name()
                        )
                    })
                })
                .collect();
            assert!(
                security::file_identity(images[0].file()).unwrap()
                    == security::file_identity(selected).unwrap(),
                "selected wrapper must retain the fixed Git file identity"
            );
            let descriptors = images
                .iter()
                .map(|lease| file_security::capture(lease.file()).unwrap())
                .collect();
            let digests = images.iter().map(|lease| image(lease.file()).1).collect();
            let imports = runtime_imports(&image_bytes(images[1].file()))
                .expect("bounded native Git Bash import table");
            assert!(
                imports.msys && !imports.cygwin,
                "fixed Git Bash must import its MSYS runtime, not an external Cygwin runtime"
            );
            Self {
                images,
                descriptors,
                digests,
                shell,
                git,
                imports,
            }
        }

        fn verify(&self) {
            for ((lease, descriptor), digest) in
                self.images.iter().zip(&self.descriptors).zip(&self.digests)
            {
                lease.verify().unwrap();
                assert!(file_security::capture(lease.file()).unwrap() == *descriptor);
                assert!(image(lease.file()).1 == *digest);
            }
        }
    }

    fn version_facts(bytes: &[u8]) -> VersionFacts {
        let text = std::str::from_utf8(bytes).ok();
        let first = text
            .and_then(|text| text.lines().next())
            .unwrap_or_default();
        let triplet = first
            .rsplit_once(" (")
            .and_then(|(_, tail)| tail.strip_suffix(')'));
        let triplet = match triplet {
            Some("x86_64-pc-msys") => "msys-x64",
            Some("aarch64-pc-msys") => "msys-arm64",
            Some("i686-pc-msys") => "msys-x86",
            Some("x86_64-pc-cygwin") => "cygwin-x64",
            Some("x86_64-pc-linux-gnu" | "x86_64-unknown-linux-gnu") => "linux-x64",
            Some("x86_64-w64-mingw32") => "mingw-x64",
            Some(_) => "unknown",
            None => "absent",
        };
        VersionFacts {
            bytes: bytes.len(),
            lines: text.map_or(0, |text| text.lines().count()),
            utf8: text.is_some(),
            empty: bytes.is_empty(),
            first_line_empty: first.is_empty(),
            gnu_header: first.starts_with("GNU bash, version "),
            gnu_header_present: text.is_some_and(|text| {
                text.lines()
                    .any(|line| line.starts_with("GNU bash, version "))
            }),
            msys_suffix: first.contains("-pc-msys"),
            triplet,
            bom: bytes.starts_with(&[0xef, 0xbb, 0xbf]),
            nul: bytes.contains(&0),
        }
    }

    fn version_diagnostic_controls() {
        let known = b"GNU bash, version 5.2.37(1)-release (x86_64-pc-msys)\r\nCopyright\r\n";
        let facts = version_facts(known);
        assert!(facts.gnu_header && facts.gnu_header_present && facts.msys_suffix);
        assert_eq!(facts.triplet, "msys-x64");
        assert!(facts.utf8 && !facts.empty && !facts.bom && !facts.nul);
        assert_eq!(facts.lines, 2);
        let empty = version_facts(b"");
        assert!(empty.empty && empty.first_line_empty && !empty.gnu_header);
        assert_eq!(empty.triplet, "absent");
        let linux = version_facts(b"GNU bash, version 5.2.37(1)-release (x86_64-pc-linux-gnu)\n");
        assert!(linux.gnu_header && !linux.msys_suffix);
        assert_eq!(linux.triplet, "linux-x64");
        let prefixed = version_facts(b"\nGNU bash, version 5.2.37(1)-release (x86_64-pc-msys)\n");
        assert!(prefixed.first_line_empty && prefixed.gnu_header_present && !prefixed.gnu_header);
        let unknown = version_facts(b"GNU bash, version 5.2.37(1)-release (unknown)\n");
        assert_eq!(unknown.triplet, "unknown");
        assert!(!unknown.msys_suffix);
        assert!(
            version_facts(b"\xef\xbb\xbfGNU bash, version 5.2.37(1)-release (x86_64-pc-msys)\n")
                .bom
        );
        assert!(!version_facts(b"\xff").utf8);
        assert!(version_facts(b"GNU bash\0").nul);
        assert!(!version_facts(b"arbitrary shell (x86_64-pc-msys)\n").gnu_header);
    }

    fn selected(name: &str) -> PathBuf {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .expect("required explicit native command control image")
    }

    fn image_bytes(file: &File) -> Vec<u8> {
        let mut file = file.try_clone().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.take(256 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .unwrap();
        assert!((64..=256 * 1024 * 1024).contains(&bytes.len()));
        bytes
    }

    fn image(file: &File) -> (u16, CanonicalDigest) {
        let bytes = image_bytes(file);
        assert!(bytes.starts_with(b"MZ"));
        let offset = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
        assert!(offset <= bytes.len() - 6 && offset <= 1024 * 1024);
        assert!(&bytes[offset..offset + 4] == b"PE\0\0");
        let machine = u16::from_le_bytes(bytes[offset + 4..offset + 6].try_into().unwrap());
        assert!(
            matches!(machine, 0x8664 | 0xaa64),
            "actual Windows native PE image"
        );
        (machine, CanonicalDigest::of_bytes(&bytes))
    }

    fn run(bash: &Path, args: &[&str], root: &Path, label: &str) -> (i32, Vec<u8>, bool) {
        use hiroute_platform::windows::job::Stream;
        let output = private_file(&root.join(format!("{label}.stdout"))).unwrap();
        let error = private_file(&root.join(format!("{label}.stderr"))).unwrap();
        let mut command = Command::new(bash);
        command
            .args(args)
            .env_clear()
            .env("HOME", root)
            .env("TMPDIR", root)
            .env(
                "PATH",
                super::super::super::native_ingress_probe::native_probe_path(),
            )
            .current_dir(root);
        let child = super::super::super::windows_runtime::spawn(
            &command,
            false,
            Stream::Null,
            Stream::File(&output),
            Stream::File(&error),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let code = loop {
            assert!(output.metadata().unwrap().len() <= 128 * 1024);
            assert!(error.metadata().unwrap().len() <= 128 * 1024);
            if let Some(code) = child.exit_code().unwrap()
                && child.empty().unwrap()
            {
                break code;
            }
            assert!(
                Instant::now() < deadline,
                "actual Bash control must exit and naturally empty its Job"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(child.empty().unwrap());
        let read = |file: &File| {
            let mut file = file.try_clone().unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            let mut bytes = Vec::new();
            file.take(128 * 1024 + 1).read_to_end(&mut bytes).unwrap();
            assert!(bytes.len() <= 128 * 1024);
            bytes
        };
        (code, read(&output), read(&error).is_empty())
    }

    #[test]
    #[ignore = "explicit actual Git for Windows Bash and source-bound candidate hiroute.exe; required CI selects exactly this case"]
    fn windows_qoder_bash_executes_the_same_cli_with_literal_drive_path() {
        version_diagnostic_controls();
        runtime_identity_controls();
        let selected_bash = selected("HIROUTE_VALIDATION_GIT_BASH_BINARY");
        let bash =
            super::super::super::managed_launch::windows_executable_path(&selected_bash).unwrap();
        let cli = super::super::super::managed_launch::windows_executable_path(&selected(
            "HIROUTE_WORKER_CLI_BINARY",
        ))
        .unwrap();
        let bash_file = security::open_identity(&bash, false).unwrap();
        let cli_file = security::open_identity(&cli, false).unwrap();
        let bash_identity = security::file_identity(&bash_file).unwrap();
        let cli_identity = security::file_identity(&cli_file).unwrap();
        let bash_security = file_security::capture(&bash_file).unwrap();
        let cli_security = file_security::capture(&cli_file).unwrap();
        let (bash_machine, bash_digest) = image(&bash_file);
        let (cli_machine, cli_digest) = image(&cli_file);
        let distribution = GitDistribution::retain(&bash, &bash_file);
        assert_eq!(
            cli_machine, 0x8664,
            "the selected released-target candidate is x64"
        );
        let root = super::super::super::probe_files::temporary("hiroute-qoder-bash-").unwrap();
        let (code, version, stderr_empty) = run(&bash, &["--version"], root.path(), "version");
        assert_eq!(code, 0);
        assert!(stderr_empty);
        let facts = version_facts(&version);
        eprintln!(
            "native Qoder Bash version diagnostic: bytes={} lines={} utf8={} empty={} first_line_empty={} gnu_header={} gnu_header_present={} msys_suffix={} triplet={} bom={} nul={}",
            facts.bytes,
            facts.lines,
            facts.utf8,
            facts.empty,
            facts.first_line_empty,
            facts.gnu_header,
            facts.gnu_header_present,
            facts.msys_suffix,
            facts.triplet,
            facts.bom,
            facts.nul,
        );
        assert!(
            msys_banner(&facts, distribution.imports),
            "GNU header requires the held fixed Git distribution and actual MSYS runtime imports"
        );
        let (shell_code, shell_version, shell_error_empty) = run(
            &distribution.shell,
            &["--version"],
            root.path(),
            "actual-shell-version",
        );
        assert_eq!(shell_code, 0);
        assert!(
            shell_error_empty && shell_version == version,
            "fixed wrapper must report its retained same-root actual shell"
        );
        let (git_code, git_version, git_error_empty) = run(
            &distribution.git,
            &["--version"],
            root.path(),
            "same-root-git-version",
        );
        assert_eq!(git_code, 0);
        let git_version = std::str::from_utf8(&git_version).unwrap().trim();
        assert!(
            git_error_empty && windows_git_version(git_version),
            "same-root Git must identify a Windows distribution"
        );
        let expected = cli_baseline(&cli, root.path()).unwrap();
        let special = root.path().join("literal space ' ; $probe & 中文");
        private_dir(&special).unwrap();
        let target = special.join("hiroute ' literal.exe");
        let mut copy = private_file(&target).unwrap();
        let mut source = cli_file.try_clone().unwrap();
        source.seek(SeekFrom::Start(0)).unwrap();
        std::io::copy(&mut source, &mut copy).unwrap();
        copy.sync_all().unwrap();
        drop(copy);
        let held_copy = security::open_owner_only(&target, false).unwrap();
        assert_eq!(image(&held_copy), (cli_machine, cli_digest.clone()));
        let copy_identity = security::file_identity(&held_copy).unwrap();
        let copy_security = file_security::capture(&held_copy).unwrap();
        let old = format!(
            "& '{}' schema list --output json",
            target.to_str().unwrap().replace('\'', "''")
        );
        let (old_exit, old_stdout, _) = run(&bash, &["-n", "-c", &old], root.path(), "old-parse");
        assert_eq!(
            old_exit, 2,
            "the original PowerShell command is not a Bash program"
        );
        assert!(old_stdout.is_empty());
        let canonical = std::fs::canonicalize(&target).unwrap();
        for (index, path) in [&target, &canonical].into_iter().enumerate() {
            let command = cli_command(path).unwrap();
            assert!(!command.starts_with("& ") && !command.starts_with("'\\\\?\\"));
            assert!(
                command.contains("'\\''"),
                "a literal apostrophe needs Bash escaping"
            );
            let (code, bytes, stderr_empty) = run(
                &bash,
                &["-c", &command],
                root.path(),
                &format!("new-{index}"),
            );
            assert_eq!(code, 0);
            assert!(stderr_empty);
            assert!(
                std::str::from_utf8(&bytes).unwrap().trim() == expected.trim(),
                "actual Bash must execute this same CLI and preserve its complete schema envelope"
            );
        }
        let alias = root.path().join("foreign-alias");
        hiroute_platform_test_support::junction::create(&alias, &special).unwrap();
        assert!(cli_command(&alias.join(target.file_name().unwrap())).is_err());
        for (path, held, identity, descriptor, digest) in [
            (
                &bash,
                &bash_file,
                bash_identity,
                &bash_security,
                &bash_digest,
            ),
            (&cli, &cli_file, cli_identity, &cli_security, &cli_digest),
            (
                &target,
                &held_copy,
                copy_identity,
                &copy_security,
                &cli_digest,
            ),
        ] {
            assert_eq!(
                security::file_identity(&security::open_identity(path, false).unwrap()).unwrap(),
                identity
            );
            assert!(file_security::capture(held).unwrap() == *descriptor);
            assert!(image(held).1 == *digest);
        }
        distribution.verify();
        eprintln!(
            "native Qoder Bash command control: bash-pe-machine={bash_machine} cli-pe-machine={cli_machine} bash-msys=true old-parse-exit={old_exit} new-cli-exact=true native-identities-unchanged=true native-permissions-unchanged=true bash-image={bash_digest} cli-image={cli_digest}"
        );
    }
}
