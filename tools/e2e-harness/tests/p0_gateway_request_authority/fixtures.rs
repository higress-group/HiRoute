use super::*;

pub(super) fn wire_snapshot(provider_authority: &str) -> GatewayPublicationSnapshotV3 {
    let candidate = |local_id| {
        sealed_native_candidate(
            local_id,
            &format!("wire-target-{local_id}"),
            &[format!("wire-credential-{local_id}")],
            provider_authority,
            &format!("wire-native-model-{local_id}"),
            &[
                (IngressProtocol::Responses, IngressProtocol::Responses),
                (IngressProtocol::Messages, IngressProtocol::Responses),
            ],
        )
    };
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "wire-authority",
        9,
        31,
        "wire-renderer/v1",
        vec![
            AliasPlanV1 {
                served_model_id: "wire-fast".into(),
                purpose: "wire fast".into(),
                agent_plan_revision: 41,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 1_500,
                max_attempts: 1,
                routing: None,
                candidates: vec![candidate(1)],
            },
            AliasPlanV1 {
                served_model_id: "wire-deep".into(),
                purpose: "wire deep".into(),
                agent_plan_revision: 42,
                protocols: vec![IngressProtocol::Responses, IngressProtocol::Messages],
                overall_timeout_ms: MAX_LOGICAL_REQUEST_DURATION_MS,
                max_attempts: 4,
                routing: None,
                candidates: vec![candidate(2)],
            },
            AliasPlanV1 {
                served_model_id: "wire-private".into(),
                purpose: "wire private".into(),
                agent_plan_revision: 43,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 200,
                max_attempts: 1,
                routing: None,
                candidates: vec![candidate(3)],
            },
        ],
        vec![
            GrantV1 {
                grant_id: "wire-grant".into(),
                generation: 5,
                bearer_token_sha256: token_sha256("wire-token"),
                protocol: IngressProtocol::Responses,
                routes: plan_routes(&[("wire-fast", 41), ("wire-deep", 42)]),
            },
            GrantV1 {
                grant_id: "wire-messages-grant".into(),
                generation: 5,
                bearer_token_sha256: token_sha256("wire-messages-token"),
                protocol: IngressProtocol::Messages,
                routes: plan_routes(&[("wire-deep", 42)]),
            },
        ],
    )
    .unwrap()
}

pub(super) fn live_snapshot(
    publication_revision: u64,
    plan_revision: u64,
    provider_authority: &str,
    credential_ref: &str,
) -> GatewayPublicationSnapshotV3 {
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "live-authority",
        17,
        publication_revision,
        format!("live-renderer/v{publication_revision}"),
        vec![AliasPlanV1 {
            served_model_id: "wire-live".into(),
            purpose: "live publication cutover".into(),
            agent_plan_revision: plan_revision,
            protocols: vec![IngressProtocol::Responses],
            overall_timeout_ms: 60_000,
            max_attempts: 1,
            routing: None,
            candidates: vec![sealed_native_candidate(
                1,
                "live-target",
                &[credential_ref.into()],
                provider_authority,
                "live-native-model",
                &[(IngressProtocol::Responses, IngressProtocol::Responses)],
            )],
        }],
        vec![GrantV1 {
            grant_id: "live-grant".into(),
            generation: 1,
            bearer_token_sha256: token_sha256("live-wire-token"),
            protocol: IngressProtocol::Responses,
            routes: plan_routes(&[("wire-live", plan_revision)]),
        }],
    )
    .unwrap()
}

pub(super) fn write_live_credentials(directory: &Path) -> PathBuf {
    std::fs::write(
        directory.join("live-credential.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credential-leases/v1",
            "credential_ref": "live-credential",
            "keys": [{
                "key_id": "live-key",
                "generation": 1,
                "authorization": "Bearer live-provider",
            }],
        }))
        .unwrap(),
    )
    .unwrap();
    let path = directory.join("live-credentials.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "hiroute.gateway.credentials/v1",
            "credentials": {
                "live-credential": "live-credential.json",
            }
        }))
        .unwrap(),
    )
    .unwrap();
    path
}
pub(super) fn write_control_nonce(directory: &Path, label: &str, mode: u32) -> (PathBuf, String) {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).unwrap();
    let nonce = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = directory.join(format!("e2e-control-{label}.nonce"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(mode);
    let mut file = options.open(&path).unwrap();
    file.write_all(nonce.as_bytes()).unwrap();
    file.sync_all().unwrap();
    (path, nonce)
}
