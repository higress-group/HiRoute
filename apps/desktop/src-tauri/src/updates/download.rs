//! Bounded native download and read-only DMG staging; existing work keeps running throughout.
use super::{PreparedUpdate, Updates};
use hiroute_host_runtime::{
    DesktopRelease, MAX_RELEASE_FEED_BYTES, RELEASE_FEED, WebsiteReleasesV2, verify_desktop_app,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::Ordering,
    time::Duration,
};

fn url(production: &str) -> Result<String, String> {
    #[cfg(all(feature = "desktop-pilot", debug_assertions))]
    if let Ok(origin) = std::env::var("HIROUTE_PILOT_UPGRADE_ORIGIN") {
        // Only the explicitly compiled Pilot can exercise a local unpublished candidate.
        let parsed = reqwest::Url::parse(&origin).map_err(|_| "UPGRADE_PILOT_ORIGIN_INVALID")?;
        if parsed.scheme() != "http"
            || parsed.host_str() != Some("127.0.0.1")
            || parsed.port().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err("UPGRADE_PILOT_ORIGIN_INVALID".into());
        }
        let path = production
            .strip_prefix("https://hiroute.ai/")
            .ok_or("UPGRADE_URL_INVALID")?;
        return Ok(format!(
            "{}{path}",
            origin.trim_end_matches('/').to_owned() + "/"
        ));
    }
    Ok(production.into())
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .user_agent("HiRoute-Updater")
        .build()
        .map_err(|_| "UPGRADE_NETWORK_UNAVAILABLE".into())
}
pub(super) async fn check(
    current: &str,
    architecture: &str,
) -> Result<Option<DesktopRelease>, String> {
    let mut response = client()?
        .get(url(RELEASE_FEED)?)
        .send()
        .await
        .map_err(|_| "UPGRADE_CATALOG_UNAVAILABLE")?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|s| s > MAX_RELEASE_FEED_BYTES as u64)
    {
        return Err("UPGRADE_CATALOG_UNAVAILABLE".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "UPGRADE_CATALOG_UNAVAILABLE")?
    {
        if bytes.len() + chunk.len() > MAX_RELEASE_FEED_BYTES {
            return Err("UPGRADE_CATALOG_INVALID".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    WebsiteReleasesV2::parse(&bytes)?
        .desktop_update(current, architecture)
        .map_err(Into::into)
}

pub(super) async fn prepare(
    state: &Updates,
    release: DesktopRelease,
) -> Result<PreparedUpdate, String> {
    release.revision_prefix()?;
    let app = state
        .current_app
        .clone()
        .ok_or("UPGRADE_INSTALLED_APP_REQUIRED")?;
    let root = state.data_root.clone().ok_or("PRIVATE_PATH_UNAVAILABLE")?;
    let parent = app.parent().ok_or("UPGRADE_APP_PATH_INVALID")?;
    let id = crate::random_id()?;
    let download_root = root.join("updates").join(&id);
    crate::bootstrap::private_dir(&download_root)?;
    let stage_root = parent.join(format!(".HiRoute-update-{id}"));
    crate::bootstrap::private_dir(&stage_root)?;
    let dmg = download_root.join("package.dmg");
    let result = async {
        let mut response = client()?
            .get(url(&release.download_url())?)
            .send()
            .await
            .map_err(|_| "UPGRADE_DOWNLOAD_FAILED")?;
        if !response.status().is_success()
            || response.content_length().is_some_and(|s| s != release.size)
        {
            return Err("UPGRADE_DOWNLOAD_SIZE_MISMATCH".to_owned());
        }
        use std::os::unix::fs::OpenOptionsExt;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&dmg)
            .map_err(|_| "UPGRADE_DOWNLOAD_WRITE_FAILED")?;
        let mut hash = Sha256::new();
        let mut downloaded = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "UPGRADE_DOWNLOAD_FAILED")?
        {
            if state.cancelled.load(Ordering::SeqCst) {
                return Err("UPGRADE_CANCELLED".into());
            }
            downloaded += chunk.len() as u64;
            if downloaded > release.size {
                return Err("UPGRADE_DOWNLOAD_SIZE_MISMATCH".into());
            }
            output
                .write_all(&chunk)
                .map_err(|_| "UPGRADE_DOWNLOAD_WRITE_FAILED")?;
            hash.update(&chunk);
            state.edit(|view| view.downloaded_bytes = downloaded)?;
        }
        output
            .sync_all()
            .map_err(|_| "UPGRADE_DOWNLOAD_WRITE_FAILED")?;
        if downloaded != release.size || format!("{:x}", hash.finalize()) != release.sha256 {
            return Err("UPGRADE_DOWNLOAD_DIGEST_MISMATCH".into());
        }
        state.edit(|view| view.phase = "verifying".into())?;
        let source = app.clone();
        let staged = stage_root.join("HiRoute.app");
        let next = staged.clone();
        let image = dmg.clone();
        let mount = download_root.join("volume");
        let selected = release.clone();
        let (old_identity, new_identity) = tokio::task::spawn_blocking(move || {
            let old = verify_desktop_app(&source, "ai.hiroute.desktop", None, None)?;
            let new = stage_dmg(&image, &mount, &next, &selected)?;
            Ok::<_, String>((old, new))
        })
        .await
        .map_err(|_| "UPGRADE_VERIFY_FAILED")??;
        if state.cancelled.load(Ordering::SeqCst) {
            return Err("UPGRADE_CANCELLED".into());
        }
        Ok(PreparedUpdate {
            release,
            current: app,
            staged,
            download_root: download_root.clone(),
            old_identity,
            new_identity,
        })
    }
    .await;
    if result.is_err() {
        // Only this attempt's random private staging directories are removed.
        let _ = fs::remove_dir_all(&stage_root);
        let _ = fs::remove_dir_all(&download_root);
    }
    result
}
fn command(program: &str, arguments: &[&std::ffi::OsStr]) -> Result<(), String> {
    let result = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|_| "UPGRADE_VERIFY_FAILED")?;
    if result.status.success() {
        Ok(())
    } else {
        Err("UPGRADE_VERIFY_FAILED".into())
    }
}
fn stage_dmg(
    image: &Path,
    mount: &Path,
    staged: &Path,
    release: &DesktopRelease,
) -> Result<hiroute_host_runtime::DesktopPackageIdentity, String> {
    use std::ffi::OsStr as O;
    command("/usr/bin/hdiutil", &[O::new("verify"), image.as_os_str()])?;
    command(
        "/usr/bin/hdiutil",
        &[
            O::new("attach"),
            O::new("-readonly"),
            O::new("-nobrowse"),
            O::new("-mountpoint"),
            mount.as_os_str(),
            image.as_os_str(),
        ],
    )?;
    let result = (|| {
        let visible = fs::read_dir(mount)
            .map_err(|_| "UPGRADE_DMG_INVALID")?
            .map(|e| e.map(|e| e.file_name()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "UPGRADE_DMG_INVALID")?;
        let visible = visible
            .into_iter()
            .filter(|n| !n.to_string_lossy().starts_with('.'))
            .collect::<std::collections::BTreeSet<_>>();
        if visible
            != ["HiRoute.app".into(), "Applications".into()]
                .into_iter()
                .collect()
            || fs::read_link(mount.join("Applications")).map_err(|_| "UPGRADE_DMG_INVALID")?
                != Path::new("/Applications")
        {
            return Err("UPGRADE_DMG_INVALID".into());
        }
        let app = mount.join("HiRoute.app");
        verify_desktop_app(&app, "ai.hiroute.desktop", Some(release), None)?;
        command("/usr/bin/ditto", &[app.as_os_str(), staged.as_os_str()])?;
        Ok(verify_desktop_app(
            staged,
            "ai.hiroute.desktop",
            Some(release),
            None,
        )?)
    })();
    let detached = command("/usr/bin/hdiutil", &[O::new("detach"), mount.as_os_str()]);
    if detached.is_err() {
        return Err("UPGRADE_DMG_DETACH_FAILED".into());
    }
    result
}
