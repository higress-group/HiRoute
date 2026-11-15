//! Native account/token fixtures. Synthetic passwords never leave memory except stdin.
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use hiroute_platform::windows::{background, service};
use windows::Win32::Foundation::HANDLE as ShellHandle;
use windows::Win32::UI::Shell::{
    DeleteProfileW, FOLDERID_Profile, LoadUserProfileW, PROFILEINFOW, SHGetKnownFolderPath,
    UnloadUserProfile,
};
use windows::core::{PCWSTR, PWSTR};
use windows_sys::Win32::Foundation::{GetLastError, LUID, LocalFree};
use windows_sys::Win32::NetworkManagement::NetManagement::*;
use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::System::RemoteDesktop::*;
use windows_sys::Win32::System::Threading::*;
use zeroize::Zeroizing;

#[link(name = "userenv")]
unsafe extern "system" {
    fn CreateEnvironmentBlock(
        value: *mut *mut std::ffi::c_void,
        token: *mut std::ffi::c_void,
        inherit: i32,
    ) -> i32;
    fn DestroyEnvironmentBlock(value: *mut std::ffi::c_void) -> i32;
}
#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(value: *mut std::ffi::c_void);
}

pub fn wide(value: &str) -> Vec<u16> {
    assert!(!value.contains('\0'));
    value.encode_utf16().chain([0]).collect()
}

pub fn profile_for_token(token: &File) -> PathBuf {
    let value = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_Profile,
            Default::default(),
            Some(ShellHandle(token.as_raw_handle())),
        )
    }
    .unwrap();
    let result = PathBuf::from(unsafe { value.to_string() }.unwrap());
    unsafe {
        CoTaskMemFree(value.0.cast());
    }
    assert!(result.is_absolute());
    result
}

pub fn information(token: &File, class: TOKEN_INFORMATION_CLASS) -> Vec<usize> {
    let mut length = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            class,
            null_mut(),
            0,
            &mut length,
        );
    }
    assert!((1..=65536).contains(&length));
    let mut value = vec![0; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    assert_ne!(
        unsafe {
            GetTokenInformation(
                token.as_raw_handle().cast(),
                class,
                value.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        },
        0
    );
    value
}

pub fn token_sid(token: &File) -> Vec<u8> {
    let value = information(token, TokenUser);
    let user = unsafe { &*value.as_ptr().cast::<TOKEN_USER>() };
    unsafe {
        std::slice::from_raw_parts(user.User.Sid.cast(), GetLengthSid(user.User.Sid) as usize)
            .to_vec()
    }
}

pub fn process_token(pid: u32) -> File {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    assert!(
        !process.is_null(),
        "native gate cannot inspect actual process token"
    );
    let process = unsafe { File::from_raw_handle(process.cast()) };
    let mut token = null_mut();
    assert_ne!(
        unsafe { OpenProcessToken(process.as_raw_handle().cast(), TOKEN_QUERY, &mut token) },
        0
    );
    unsafe { File::from_raw_handle(token.cast()) }
}

/// Require an interactive user whose token has no Administrators group,
/// including a deny-only SID in a UAC-filtered Administrator token.
pub fn assert_current_standard_account() {
    let token = process_token(std::process::id());
    let sid = background::sid_string(&token_sid(&token)).unwrap();
    assert_ne!(assert_standard(&token, &sid), 0);
    let mut admin = [0_usize; 9];
    let mut size = std::mem::size_of_val(&admin) as u32;
    // SAFETY: admin is an aligned output buffer with its actual capacity.
    assert_ne!(
        unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                null_mut(),
                admin.as_mut_ptr().cast(),
                &mut size,
            )
        },
        0
    );
    let groups = information(&token, TokenGroups);
    // SAFETY: the live token query buffer holds the complete TOKEN_GROUPS array.
    let groups = unsafe { &*groups.as_ptr().cast::<TOKEN_GROUPS>() };
    for group in
        unsafe { std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize) }
    {
        // SAFETY: both SID buffers remain live for comparison.
        assert_eq!(
            unsafe { EqualSid(group.Sid, admin.as_mut_ptr().cast()) },
            0,
            "standard-user evidence rejects deny-only Administrators membership too"
        );
    }
}

/// Inspect the actual business process, not an installer token or Task EnginePID.
pub fn assert_standard(token: &File, expected_sid: &str) -> u32 {
    assert_eq!(
        background::sid_string(&token_sid(token)).unwrap(),
        expected_sid
    );
    let elevation = information(token, TokenElevation);
    assert_eq!(
        unsafe { (*elevation.as_ptr().cast::<TOKEN_ELEVATION>()).TokenIsElevated },
        0
    );
    let integrity = information(token, TokenIntegrityLevel);
    let label = unsafe { &*integrity.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    let sid = unsafe {
        std::slice::from_raw_parts(
            label.Label.Sid.cast(),
            GetLengthSid(label.Label.Sid) as usize,
        )
    };
    assert_eq!(background::sid_string(sid).unwrap(), "S-1-16-8192");
    let mut admin = vec![0_usize; 68_usize.div_ceil(std::mem::size_of::<usize>())];
    let mut size = 68;
    assert_ne!(
        unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                null_mut(),
                admin.as_mut_ptr().cast(),
                &mut size,
            )
        },
        0
    );
    let groups = information(token, TokenGroups);
    let groups = unsafe { &*groups.as_ptr().cast::<TOKEN_GROUPS>() };
    for group in
        unsafe { std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize) }
    {
        if unsafe { EqualSid(group.Sid, admin.as_mut_ptr().cast()) } != 0 {
            assert_eq!(
                group.Attributes & 0x10,
                0x10,
                "Administrators must be deny-only"
            );
            assert_eq!(group.Attributes & 0x4, 0);
        }
    }
    let session = information(token, TokenSessionId);
    unsafe { *session.as_ptr().cast::<u32>() }
}

pub fn current_restricted() -> File {
    let mut token = null_mut();
    assert_ne!(
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_ADJUST_DEFAULT,
                &mut token,
            )
        },
        0
    );
    let original = unsafe { File::from_raw_handle(token.cast()) };
    let mut admin = vec![0_usize; 68_usize.div_ceil(std::mem::size_of::<usize>())];
    let mut size = 68;
    assert_ne!(
        unsafe {
            CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                null_mut(),
                admin.as_mut_ptr().cast(),
                &mut size,
            )
        },
        0
    );
    let disabled = SID_AND_ATTRIBUTES {
        Sid: admin.as_mut_ptr().cast(),
        Attributes: 0,
    };
    assert_ne!(
        unsafe {
            CreateRestrictedToken(
                original.as_raw_handle().cast(),
                DISABLE_MAX_PRIVILEGE,
                1,
                &disabled,
                0,
                null(),
                0,
                null(),
                &mut token,
            )
        },
        0
    );
    let token = unsafe { File::from_raw_handle(token.cast()) };
    let mut sid = null_mut();
    assert_ne!(
        unsafe { ConvertStringSidToSidW(wide("S-1-16-8192").as_ptr(), &mut sid) },
        0
    );
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: sid,
            Attributes: 0x20,
        },
    };
    let applied = unsafe {
        SetTokenInformation(
            token.as_raw_handle().cast(),
            TokenIntegrityLevel,
            (&label as *const TOKEN_MANDATORY_LABEL).cast(),
            std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(sid),
        )
    };
    unsafe {
        LocalFree(sid);
    }
    assert_ne!(applied, 0);
    token
}

/// Enumerate WTS sessions, requiring the runner account really logged on. LogonUser
/// alone cannot supply this prerequisite of TASK_LOGON_INTERACTIVE_TOKEN.
pub fn interactive_session(account: &str) -> u32 {
    let mut sessions = null_mut();
    let mut count = 0;
    assert_ne!(
        unsafe {
            WTSEnumerateSessionsW(WTS_CURRENT_SERVER_HANDLE, 0, 1, &mut sessions, &mut count)
        },
        0
    );
    let mut found = None;
    for session in unsafe { std::slice::from_raw_parts(sessions, count as usize) } {
        if session.SessionId == 0 {
            continue;
        }
        let field = |class| {
            let mut text = null_mut();
            let mut size = 0;
            if unsafe {
                WTSQuerySessionInformationW(
                    WTS_CURRENT_SERVER_HANDLE,
                    session.SessionId,
                    class,
                    &mut text,
                    &mut size,
                )
            } == 0
            {
                return String::new();
            }
            let units = unsafe { std::slice::from_raw_parts(text, size as usize / 2) };
            let value =
                String::from_utf16_lossy(units.split(|v| *v == 0).next().unwrap_or_default());
            unsafe {
                WTSFreeMemory(text.cast());
            }
            value
        };
        if format!("{}\\{}", field(WTSDomainName), field(WTSUserName)).eq_ignore_ascii_case(account)
        {
            found = Some(session.SessionId);
        }
    }
    unsafe {
        WTSFreeMemory(sessions.cast());
    }
    found.expect("required Task product gate has no real interactive runner session; not-executed is a failing gate")
}

pub fn environment(token: &File) -> Vec<(OsString, OsString)> {
    let mut block = null_mut();
    assert_ne!(
        unsafe { CreateEnvironmentBlock(&mut block, token.as_raw_handle().cast(), 0) },
        0
    );
    let pointer = block.cast::<u16>();
    let mut offset = 0;
    let mut result: Vec<(OsString, OsString)> = Vec::new();
    loop {
        assert!(
            offset < 1024 * 1024,
            "native environment exceeds gate bound"
        );
        if unsafe { *pointer.add(offset) } == 0 {
            break;
        }
        let start = offset;
        while unsafe { *pointer.add(offset) } != 0 {
            offset += 1;
        }
        let entry = String::from_utf16(unsafe {
            std::slice::from_raw_parts(pointer.add(start), offset - start)
        })
        .unwrap();
        if let Some((key, value)) = entry.split_once('=')
            && !key.is_empty()
        {
            result.push((key.into(), value.into()));
        }
        offset += 1;
    }
    assert_ne!(unsafe { DestroyEnvironmentBlock(block) }, 0);
    // Hosted Python may need its toolcache DLL directories; no test password is
    // ever an environment value. User profile/TEMP remain token-derived.
    if let Some(path) = std::env::var_os("PATH") {
        result.retain(|(key, _)| !key.to_string_lossy().eq_ignore_ascii_case("PATH"));
        result.push(("PATH".into(), path));
    }
    result
}

pub struct Privileges {
    token: File,
    previous: Vec<TOKEN_PRIVILEGES>,
}
impl Privileges {
    pub fn enable() -> Self {
        let mut token = null_mut();
        assert_ne!(
            unsafe {
                OpenProcessToken(
                    GetCurrentProcess(),
                    TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES,
                    &mut token,
                )
            },
            0
        );
        let mut guard = Self {
            token: unsafe { File::from_raw_handle(token.cast()) },
            previous: vec![],
        };
        for name in [
            "SeBackupPrivilege",
            "SeRestorePrivilege",
            "SeImpersonatePrivilege",
            "SeIncreaseQuotaPrivilege",
        ] {
            let mut luid = LUID::default();
            assert_ne!(
                unsafe { LookupPrivilegeValueW(null(), wide(name).as_ptr(), &mut luid) },
                0
            );
            let requested = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let mut previous = TOKEN_PRIVILEGES::default();
            let mut size = 0;
            assert_ne!(
                unsafe {
                    AdjustTokenPrivileges(
                        guard.token.as_raw_handle().cast(),
                        0,
                        &requested,
                        std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                        &mut previous,
                        &mut size,
                    )
                },
                0
            );
            assert_ne!(
                unsafe { GetLastError() },
                1300,
                "elevated native fixture lacks a required profile/process privilege"
            );
            guard.previous.push(previous);
        }
        guard
    }
}
impl Drop for Privileges {
    fn drop(&mut self) {
        for previous in self.previous.iter().rev() {
            unsafe {
                AdjustTokenPrivileges(
                    self.token.as_raw_handle().cast(),
                    0,
                    previous,
                    0,
                    null_mut(),
                    null_mut(),
                );
            }
        }
    }
}

pub struct Account {
    name: String,
    pub account: String,
    pub password: Zeroizing<String>,
    pub token: Option<File>,
    pub sid: String,
    pub profile: PathBuf,
    hive: Option<ShellHandle>,
}
impl Account {
    pub fn create() -> Self {
        let mut nonce = [0_u8; 24];
        getrandom::fill(&mut nonce).unwrap();
        let hex: String = nonce.iter().map(|v| format!("{v:02x}")).collect();
        let name = format!("hrw{}{}", std::process::id(), &hex[..6]);
        assert!(name.len() <= 20);
        let password = Zeroizing::new(format!("A!a9{hex}"));
        let mut username = wide(&name);
        let mut secret = Zeroizing::new(wide(&password));
        let info = USER_INFO_1 {
            usri1_name: username.as_mut_ptr(),
            usri1_password: secret.as_mut_ptr(),
            usri1_priv: USER_PRIV_USER,
            usri1_flags: UF_SCRIPT | UF_NORMAL_ACCOUNT | UF_DONT_EXPIRE_PASSWD,
            ..Default::default()
        };
        let mut parameter = 0;
        assert_eq!(
            unsafe {
                NetUserAdd(
                    null(),
                    1,
                    (&info as *const USER_INFO_1).cast(),
                    &mut parameter,
                )
            },
            0,
            "native synthetic standard account creation failed"
        );
        // Install the cleanup guard immediately after account creation.
        let mut value = Self {
            name,
            account: String::new(),
            password,
            token: None,
            sid: String::new(),
            profile: PathBuf::new(),
            hive: None,
        };
        let mut users = vec![0_usize; 68_usize.div_ceil(std::mem::size_of::<usize>())];
        let mut size = 68;
        assert_ne!(
            unsafe {
                CreateWellKnownSid(
                    WinBuiltinUsersSid,
                    null_mut(),
                    users.as_mut_ptr().cast(),
                    &mut size,
                )
            },
            0
        );
        let bytes = unsafe { std::slice::from_raw_parts(users.as_ptr().cast(), size as usize) };
        let group = service::account_name_for_sid(bytes).unwrap();
        let group = wide(group.rsplit('\\').next().unwrap());
        let member = LOCALGROUP_MEMBERS_INFO_3 {
            lgrmi3_domainandname: username.as_mut_ptr(),
        };
        let added = unsafe {
            NetLocalGroupAddMembers(
                null(),
                group.as_ptr(),
                3,
                (&member as *const LOCALGROUP_MEMBERS_INFO_3).cast(),
                1,
            )
        };
        assert!(
            added == 0 || added == 1378,
            "synthetic account Users membership failed"
        );
        let mut token = null_mut();
        assert_ne!(
            unsafe {
                LogonUserW(
                    username.as_ptr(),
                    wide(".").as_ptr(),
                    secret.as_ptr(),
                    LOGON32_LOGON_INTERACTIVE,
                    LOGON32_PROVIDER_DEFAULT,
                    &mut token,
                )
            },
            0,
            "native standard owner authentication failed"
        );
        value.token = Some(unsafe { File::from_raw_handle(token.cast()) });
        value.sid = background::sid_string(&token_sid(value.token.as_ref().unwrap())).unwrap();
        value.account =
            service::account_name_for_sid(&token_sid(value.token.as_ref().unwrap())).unwrap();
        // LogonUser authenticates an account; it does not create an SCM host or
        // move that token to session 0. Native CI's caller can be session 2. The
        // owner fixture executes with this real token, while the product SCM
        // gate separately requires its actual daemon to be session 0.
        assert_standard(value.token.as_ref().unwrap(), &value.sid);
        let mut info = PROFILEINFOW {
            dwSize: std::mem::size_of::<PROFILEINFOW>() as u32,
            dwFlags: 1,
            lpUserName: PWSTR(username.as_mut_ptr()),
            ..Default::default()
        };
        unsafe { LoadUserProfileW(ShellHandle(token), &mut info) }
            .expect("load actual standard owner profile");
        value.hive = Some(info.hProfile);
        let folder = unsafe {
            SHGetKnownFolderPath(
                &FOLDERID_Profile,
                Default::default(),
                Some(ShellHandle(token)),
            )
        }
        .unwrap();
        value.profile = PathBuf::from(unsafe { folder.to_string() }.unwrap());
        unsafe {
            CoTaskMemFree(folder.0.cast());
        }
        assert!(value.profile.is_absolute());
        assert!(
            !value
                .profile
                .join(".local/share/hiroute/standalone.json")
                .exists()
        );
        value
    }
    pub fn finish(mut self) {
        self.cleanup()
            .expect("synthetic native account/profile cleanup must complete");
    }
    fn cleanup(&mut self) -> io::Result<()> {
        if let Some(hive) = self.hive {
            unsafe {
                UnloadUserProfile(
                    ShellHandle(self.token.as_ref().unwrap().as_raw_handle()),
                    hive,
                )
            }
            .map_err(io::Error::other)?;
            self.hive = None;
        }
        self.token.take();
        if !self.profile.as_os_str().is_empty() {
            let sid = wide(&self.sid);
            let path = wide(self.profile.to_str().unwrap());
            unsafe { DeleteProfileW(PCWSTR(sid.as_ptr()), PCWSTR(path.as_ptr()), PCWSTR::null()) }
                .map_err(io::Error::other)?;
            self.profile.clear();
        }
        if !self.name.is_empty() {
            let result = unsafe { NetUserDel(null(), wide(&self.name).as_ptr()) };
            if result != 0 {
                return Err(io::Error::from_raw_os_error(result as i32));
            }
            self.name.clear();
        }
        Ok(())
    }
}
impl Drop for Account {
    fn drop(&mut self) {
        if self.cleanup().is_err() {
            eprintln!("native synthetic account cleanup failed");
        }
    }
}
