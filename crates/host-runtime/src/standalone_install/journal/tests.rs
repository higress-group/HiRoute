use super::*;
use serde_json::json;

#[derive(Default)]
struct Manager {
    calls: Vec<&'static str>,
}
impl ManagerHost for Manager {
    fn quiesce(&mut self, _: &[&Value]) -> io::Result<()> {
        self.calls.push("stop");
        Ok(())
    }
    fn register(&mut self, _: &Value, _: Option<&Value>) -> io::Result<()> {
        self.calls.push("register");
        Ok(())
    }
    fn unregister(&mut self, _: &Value) -> io::Result<()> {
        self.calls.push("unregister");
        Ok(())
    }
}
fn fixture() -> (tempfile::TempDir, StandaloneLayout, Journal) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let layout =
        StandaloneLayout::from_values(false, Some(home.clone()), None, None, None).unwrap();
    let root = home.join(".local/lib/hiroute/0.3.1-aaaaaaaaaaaa");
    directory(&root).unwrap();
    directory(&layout.data_root.join("service")).unwrap();
    directory(&home.join(".local/bin")).unwrap();
    let source = root.join("hiroute.exe");
    fs::write(&source, b"new").unwrap();
    let marker = StandaloneInstallRecordV1 {
        schema_version: crate::STANDALONE_INSTALL_SCHEMA_V1.into(),
        version: "0.3.1".into(),
        target: "x86_64-pc-windows-msvc".into(),
        install_root: root.clone(),
        service_definition: layout.data_root.join("service/windows-background.json"),
        installed_skills: [".agents", ".claude"]
            .iter()
            .map(|p| home.join(p).join("skills/hiroute-management"))
            .collect(),
        cpa_binary: Some(root.join("libexec/cliproxyapi.exe")),
        cpa_sha256: Some("a".repeat(64)),
    };
    let entries = vec![Entry {
        destination: home.join(".local/bin/hiroute.exe"),
        source,
        backup: layout.data_root.join("service/install-backups/batch/0"),
        old_sha256: None,
        new_sha256: Some(format!("{:x}", sha2::Sha256::digest(b"new"))),
        kind: EntryKind::Hardlink,
    }];
    let journal = Journal {
        schema: JOURNAL_SCHEMA.into(),
        operation: Operation::Install,
        phase: Phase::Prepared,
        old_marker: None,
        new_marker: marker,
        old_definition: None,
        new_definition: json!({"manager":"task"}),
        files: entries,
    };
    (temp, layout, journal)
}
use sha2::Digest;
#[test]
fn pending_manager_delete_keeps_prepared_journal_and_all_owned_files() {
    struct Pending;
    impl ManagerHost for Pending {
        fn quiesce(&mut self, _: &[&Value]) -> io::Result<()> {
            Ok(())
        }
        fn register(&mut self, _: &Value, _: Option<&Value>) -> io::Result<()> {
            unreachable!()
        }
        fn unregister(&mut self, _: &Value) -> io::Result<()> {
            Err(io::Error::other("deletion pending"))
        }
    }
    let (_temp, layout, mut journal) = fixture();
    journal.apply_files().unwrap();
    journal.operation = Operation::Uninstall;
    journal.old_marker = Some(journal.new_marker.clone());
    journal.old_definition = Some(journal.new_definition.clone());
    journal.files[0].old_sha256 = journal.files[0].new_sha256.take();
    journal.persist(&layout).unwrap();
    assert!(journal.recover(&layout, &mut Pending).is_err());
    assert!(Journal::read(&layout).unwrap().unwrap().phase == Phase::Prepared);
    assert_eq!(
        fs::read(layout.home.join(".local/bin/hiroute.exe")).unwrap(),
        b"new"
    );
}
#[test]
fn duplicate_json_object_keys_are_rejected() {
    let (_temp, layout, _) = fixture();
    fs::write(
        Journal::path(&layout),
        br#"{"schema":"first","schema":"second"}"#,
    )
    .unwrap();
    assert!(Journal::read(&layout).is_err());
}
#[test]
fn first_install_crash_before_or_after_each_file_rolls_back_without_changing_business_data() {
    for applied in [false, true] {
        let (_temp, layout, journal) = fixture();
        let mut manager = Manager::default();
        directory(&layout.state_root).unwrap();
        fs::write(layout.state_root.join("business"), b"keep").unwrap();
        journal.persist(&layout).unwrap();
        if applied {
            journal.apply_files().unwrap();
            write_json(&layout.marker_path, &journal.new_marker).unwrap();
        }
        Journal::read(&layout)
            .unwrap()
            .unwrap()
            .recover(&layout, &mut manager)
            .unwrap();
        assert!(!journal.files[0].destination.exists());
        assert!(!layout.marker_path.exists());
        assert_eq!(
            fs::read(layout.state_root.join("business")).unwrap(),
            b"keep"
        );
        assert_eq!(manager.calls, ["stop", "unregister"]);
        assert!(Journal::read(&layout).unwrap().is_none());
    }
}
#[test]
fn prepared_upgrade_restores_exact_old_bytes_and_registration() {
    for replaced in [false, true] {
        let (_temp, layout, mut journal) = fixture();
        let old_root = layout.home.join(".local/lib/hiroute/0.3.0-bbbbbbbbbbbb");
        directory(&old_root).unwrap();
        let old = old_root.join("hiroute.exe");
        fs::write(&old, b"old").unwrap();
        let entry = &mut journal.files[0];
        fs::write(&entry.destination, b"old").unwrap();
        directory(entry.backup.parent().unwrap()).unwrap();
        copy_new(&old, &entry.backup).unwrap();
        entry.old_sha256 = digest(&old).unwrap();
        let mut marker = journal.new_marker.clone();
        marker.install_root = old_root.clone();
        marker.version = "0.3.0".into();
        marker.cpa_binary = Some(old_root.join("libexec/cliproxyapi.exe"));
        journal.old_marker = Some(marker.clone());
        journal.old_definition = Some(json!({"manager":"task","old":true}));
        journal.persist(&layout).unwrap();
        if replaced {
            journal.apply_files().unwrap();
        }
        let mut host = Manager::default();
        Journal::read(&layout)
            .unwrap()
            .unwrap()
            .recover(&layout, &mut host)
            .unwrap();
        assert_eq!(fs::read(&journal.files[0].destination).unwrap(), b"old");
        assert_eq!(
            read_json::<StandaloneInstallRecordV1>(&layout.marker_path, MAX_RECORD).unwrap(),
            Some(marker)
        );
        assert_eq!(host.calls, ["stop", "register"]);
    }
}
#[test]
fn foreign_change_refuses_all_replacement_and_recovery_without_deleting_it() {
    let (_temp, layout, journal) = fixture();
    journal.persist(&layout).unwrap();
    fs::write(&journal.files[0].destination, b"foreign").unwrap();
    assert!(journal.apply_files().is_err());
    assert!(journal.recover(&layout, &mut Manager::default()).is_err());
    assert_eq!(
        fs::read(layout.home.join(".local/bin/hiroute.exe")).unwrap(),
        b"foreign"
    );
    assert!(Journal::path(&layout).exists());
}
#[test]
fn committed_install_never_replays_file_effects_or_registration() {
    let (_temp, layout, mut journal) = fixture();
    journal.apply_files().unwrap();
    journal.commit(&layout).unwrap();
    let mut host = Manager::default();
    Journal::read(&layout)
        .unwrap()
        .unwrap()
        .recover(&layout, &mut host)
        .unwrap();
    assert!(host.calls.is_empty());
    assert_eq!(fs::read(&journal.files[0].destination).unwrap(), b"new");
}
#[test]
fn prepared_uninstall_completes_after_each_crash_boundary_and_keeps_data() {
    for already_removed in [false, true] {
        let (_temp, layout, mut journal) = fixture();
        journal.apply_files().unwrap();
        write_json(&layout.marker_path, &journal.new_marker).unwrap();
        write_json(
            &journal.new_marker.service_definition,
            &journal.new_definition,
        )
        .unwrap();
        journal.operation = Operation::Uninstall;
        journal.old_marker = Some(journal.new_marker.clone());
        journal.old_definition = Some(journal.new_definition.clone());
        let entry = &mut journal.files[0];
        entry.old_sha256 = entry.new_sha256.take();
        directory(entry.backup.parent().unwrap()).unwrap();
        copy_new(&entry.source, &entry.backup).unwrap();
        journal.persist(&layout).unwrap();
        if already_removed {
            journal.apply_files().unwrap();
        }
        let mut host = Manager::default();
        journal.recover(&layout, &mut host).unwrap();
        assert_eq!(host.calls, ["stop", "unregister"]);
        assert!(!layout.marker_path.exists());
        assert!(!layout.home.join(".local/bin/hiroute.exe").exists());
    }
}
#[test]
fn current_journal_has_closed_fields_and_refuses_escaped_paths() {
    let (_temp, layout, journal) = fixture();
    let bytes = serde_json::to_value(&journal).unwrap();
    assert_eq!(
        bytes
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "files",
            "new_definition",
            "new_marker",
            "old_definition",
            "old_marker",
            "operation",
            "phase",
            "schema"
        ])
    );
    for field in ["destination", "source", "backup"] {
        let mut changed = bytes.clone();
        changed["files"][0][field] = json!(layout.home.join("foreign/entry"));
        assert!(
            serde_json::from_value::<Journal>(changed)
                .unwrap()
                .validate(&layout)
                .is_err()
        );
    }
    let mut changed = bytes.clone();
    changed["secret"] = json!("unexpected");
    assert!(serde_json::from_value::<Journal>(changed).is_err());
    let mut changed = bytes;
    changed["schema"] = json!("hiroute.windows-install-journal/v2");
    assert!(
        serde_json::from_value::<Journal>(changed)
            .unwrap()
            .validate(&layout)
            .is_err()
    );
}
