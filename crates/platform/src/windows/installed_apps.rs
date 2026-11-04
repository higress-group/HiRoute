//! Bounded, read-only uninstall registration hints. Callers validate the files.
use std::os::windows::ffi::OsStringExt;
use std::{ffi::OsString, path::PathBuf, ptr};
use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS},
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW,
        RegOpenKeyExW,
    },
};

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: owns a successfully opened registry key, never a predefined key.
        unsafe { RegCloseKey(self.0) };
    }
}

fn open(parent: HKEY, name: &[u16], access: u32) -> Option<Key> {
    let mut key = ptr::null_mut();
    // SAFETY: name is NUL-terminated and key is a live output pointer.
    if unsafe { RegOpenKeyExW(parent, name.as_ptr(), 0, access, &mut key) } == ERROR_SUCCESS {
        Some(Key(key))
    } else {
        None
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn string(key: &Key, name: &str) -> Option<OsString> {
    let name = wide(name);
    let mut buffer = [0_u16; 32768];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: bounded UTF-16 output. RegGetValue expands REG_EXPAND_SZ using
    // Windows semantics; no shell or registered command is evaluated.
    let status = unsafe {
        RegGetValueW(
            key.0,
            ptr::null(),
            name.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS
        || size < 2
        || !size.is_multiple_of(2)
        || size as usize > buffer.len() * 2
    {
        return None;
    }
    let value = &buffer[..size as usize / 2];
    if value.last() != Some(&0) || value[..value.len() - 1].contains(&0) {
        return None;
    }
    Some(OsString::from_wide(&value[..value.len() - 1]))
}

/// Inspect both registry views for per-user and machine installations. Returned
/// absolute locations are discovery hints, not executable or ownership proofs.
pub fn uninstall_locations(display_name: &str) -> Vec<PathBuf> {
    let path = wide(r"Software\Microsoft\Windows\CurrentVersion\Uninstall");
    let mut locations = Vec::new();
    for owner in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            if let Some(key) = open(owner, &path, KEY_READ | view) {
                locations.extend(locations_in(&key, display_name));
            }
        }
    }
    locations.dedup();
    locations
}

fn locations_in(key: &Key, display_name: &str) -> Vec<PathBuf> {
    let mut locations = Vec::new();
    // The conventional installation paths are inspected first by the caller.
    // Unrelated installed software cannot make discovery unbounded.
    for index in 0..4096 {
        let mut name = [0_u16; 256];
        let mut size = name.len() as u32;
        // SAFETY: live key and bounded key-name output; all optional outputs null.
        let status = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                name.as_mut_ptr(),
                &mut size,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS || size as usize >= name.len() {
            continue;
        }
        name[size as usize] = 0;
        let Some(entry) = open(key.0, &name, KEY_READ) else {
            continue;
        };
        let Some(label) = string(&entry, "DisplayName") else {
            continue;
        };
        let Some(label) = label.to_str() else {
            continue;
        };
        if label != display_name
            && !label
                .strip_prefix(display_name)
                .is_some_and(|rest| rest.starts_with(' '))
        {
            continue;
        }
        if let Some(path) = string(&entry, "InstallLocation")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            locations.push(path);
        }
    }
    locations
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Registry::{
        KEY_WRITE, REG_SZ, RegCreateKeyExW, RegDeleteTreeW, RegSetKeyValueW,
    };

    struct TestRoot(Vec<u16>);
    impl Drop for TestRoot {
        fn drop(&mut self) {
            // SAFETY: only the unique subtree created by this test is removed.
            unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, self.0.as_ptr()) };
        }
    }

    fn create(parent: HKEY, name: &[u16]) -> Key {
        let mut key = ptr::null_mut();
        // SAFETY: fixed test-owned parent/name and live output pointer.
        assert_eq!(
            unsafe {
                RegCreateKeyExW(
                    parent,
                    name.as_ptr(),
                    0,
                    ptr::null(),
                    0,
                    KEY_READ | KEY_WRITE,
                    ptr::null(),
                    &mut key,
                    ptr::null_mut(),
                )
            },
            ERROR_SUCCESS
        );
        Key(key)
    }

    fn set(key: &Key, name: &str, value: &str) {
        let name = wide(name);
        let value = wide(value);
        // SAFETY: test-owned key and complete bounded UTF-16 value.
        assert_eq!(
            unsafe {
                RegSetKeyValueW(
                    key.0,
                    ptr::null(),
                    name.as_ptr(),
                    REG_SZ,
                    value.as_ptr().cast(),
                    (value.len() * 2) as u32,
                )
            },
            ERROR_SUCCESS
        );
    }

    #[test]
    fn windows_uninstall_locations_select_exact_app_and_versioned_names_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let path = wide(&format!(
            r"Software\HiRouteTests\{}",
            directory.path().file_name().unwrap().to_string_lossy()
        ));
        let _cleanup = TestRoot(path.clone());
        let root = create(HKEY_CURRENT_USER, &path);
        let install = directory.path().join("custom DSH installation");
        let expected = directory.path().join("versioned installation");
        for (id, label, location) in [
            ("one", "DeepSeek Harness", install.to_str().unwrap()),
            (
                "two",
                "DeepSeek Harness 0.2.1-rc.5",
                expected.to_str().unwrap(),
            ),
            ("other", "DeepSeek HarnessOther", install.to_str().unwrap()),
            ("relative", "DeepSeek Harness", "relative/path"),
            ("empty", "DeepSeek Harness", ""),
        ] {
            let entry = create(root.0, &wide(id));
            set(&entry, "DisplayName", label);
            set(&entry, "InstallLocation", location);
        }
        let mut actual = locations_in(&root, "DeepSeek Harness");
        actual.sort();
        let mut wanted = vec![install, expected];
        wanted.sort();
        assert_eq!(actual, wanted);
        assert!(locations_in(&root, "Missing app").is_empty());
        assert_eq!(
            string(
                &open(root.0, &wide("one"), KEY_READ).unwrap(),
                "DisplayName"
            )
            .unwrap(),
            "DeepSeek Harness"
        );
    }
}
