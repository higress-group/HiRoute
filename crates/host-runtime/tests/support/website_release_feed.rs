use serde_json::Value;

pub fn website_v3_feed_fixture() -> Vec<u8> {
    let mut catalog: Value = serde_json::from_slice(include_bytes!(
        "../../../../apps/website/data/releases.json"
    ))
    .unwrap();
    // The website tests exercise the publisher's actual projection. Keep the
    // native reader fixture on that boundary without adding a Node dependency.
    catalog["releases"]
        .as_array_mut()
        .unwrap()
        .retain_mut(|release| {
            let artifacts = release["artifacts"].as_array_mut().unwrap();
            artifacts.retain(|artifact| artifact["platform"] != "Omarchy");
            !artifacts.is_empty()
        });
    serde_json::to_vec(&catalog).unwrap()
}
