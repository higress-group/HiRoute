#[path = "fixtures/website_releases_v2_031.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
mod released;
use hiroute_host_runtime::WebsiteReleasesV3;
#[test]
fn supported_macos_reader_accepts_the_current_v2_projection_and_selects_its_dmg() {
    let current =
        WebsiteReleasesV3::parse(include_bytes!("../../../apps/website/data/releases.json"))
            .unwrap();
    let bytes = current.v2_projection().unwrap();
    let old = released::WebsiteReleasesV2::parse(&bytes).unwrap();
    for architecture in ["arm64", "x86_64"] {
        let update = old.desktop_update("0.0.0", architecture).unwrap().unwrap();
        assert!(
            update
                .filename
                .ends_with(&format!("-macos-{architecture}-trial.dmg"))
        );
    }
}
