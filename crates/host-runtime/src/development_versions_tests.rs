use crate::{WebsiteReleasesV2, WebsiteReleasesV3};
use serde_json::json;

fn catalog(version: &str, windows: bool) -> Vec<u8> {
    let artifact = if windows {
        json!({"kind":"desktop","platform":"Windows","architecture":"x86_64",
            "format":"nsis","distribution":"unsigned",
            "filename":format!("HiRoute-{version}-aaaaaaaaaaaa-windows-x86_64-setup.exe"),
            "sha256":"a".repeat(64),"size":100})
    } else {
        json!({"kind":"desktop","platform":"macOS","architecture":"arm64",
            "format":"dmg","minimum_os":"15.0","distribution":"self-signed",
            "filename":format!("HiRoute-{version}-aaaaaaaaaaaa-macos-arm64-trial.dmg"),
            "sha256":"a".repeat(64),"size":100})
    };
    serde_json::to_vec(&json!({"schema":"hiroute.website.releases/v3",
        "releases":[{"version":version,"channel":"stable","published_at":"2026-10-01T00:00:00Z",
        "notes":{"en":"Test","zh":"测试"},"artifacts":[artifact]}]}))
    .unwrap()
}

#[test]
fn development_version_sits_between_previous_and_next_stable() {
    let parse = |value| semver::Version::parse(value).unwrap();
    assert!(parse("0.3.1") < parse("0.3.2-dev.1"));
    assert!(parse("0.3.2-dev.1") < parse("0.3.2"));
}

#[test]
fn macos_development_build_ignores_previous_release_and_selects_next_stable() {
    let macos = |version| {
        let projection = WebsiteReleasesV3::parse(&catalog(version, false))
            .unwrap()
            .v2_projection()
            .unwrap();
        WebsiteReleasesV2::parse(&projection).unwrap()
    };
    for (previous, current, next) in [
        ("0.3.1", "0.3.2-dev.1", "0.3.2"),
        ("0.3.1", "0.3.2-dev.1", "0.5.0"),
        ("0.5.0", "0.5.1-dev.1", "0.5.1"),
    ] {
        assert!(
            macos(previous)
                .desktop_update(current, "arm64")
                .unwrap()
                .is_none()
        );
        let update = macos(next);
        assert_eq!(
            update
                .desktop_update(current, "arm64")
                .unwrap()
                .unwrap()
                .version,
            next
        );
        assert!(update.desktop_update(next, "arm64").unwrap().is_none());
    }
}

#[test]
fn windows_development_build_ignores_previous_release_and_selects_next_stable() {
    for (previous, current, next) in [
        ("0.3.1", "0.3.2-dev.1", "0.3.2"),
        ("0.3.1", "0.3.2-dev.1", "0.5.0"),
        ("0.5.0", "0.5.1-dev.1", "0.5.1"),
    ] {
        let old = WebsiteReleasesV3::parse(&catalog(previous, true)).unwrap();
        assert!(old.windows_update(current).unwrap().is_none());
        let update = WebsiteReleasesV3::parse(&catalog(next, true)).unwrap();
        assert_eq!(
            update.windows_update(current).unwrap().unwrap().version,
            next
        );
        assert!(update.windows_update(next).unwrap().is_none());
    }
}
