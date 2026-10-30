//! Source-owned read/preview/save admission; no provider request in a configuration operation.
use super::*;
use hiroute_application_api::*;
use hiroute_domain::{
    ComputeManagementMutation, MaterializationState, NativeNetworkMutationV1, NativeNetworkPolicyV1,
};
use hiroute_integrations::native_network_route_view;

fn target_url(target: &hiroute_domain::ComputeManagementTargetV2) -> String {
    let host = if target.authority.contains(':') {
        format!("[{}]", target.authority)
    } else {
        target.authority.clone()
    };
    format!(
        "{}://{}:{}{}",
        target.scheme, host, target.port, target.request_path
    )
}

impl LocalControlAdapter {
    pub(super) fn read_native_source_network(
        &self,
        request: NativeSourceNetworkQueryV1,
    ) -> Result<NativeSourceNetworkViewV1, ComputeManagementControlError> {
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let source = stores
            .control()
            .compute_management_source(&request.source_id)
            .map_err(map_port)?
            .ok_or(ComputeManagementControlError::NotFound)?;
        let record = stores
            .control()
            .native_source_network(&source.source_id)
            .map_err(map_port)?
            .ok_or(ComputeManagementControlError::NotFound)?;
        record
            .validate_for(&source)
            .map_err(|_| ComputeManagementControlError::Corrupt)?;
        let snapshot = self
            .native_network
            .pin()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let applied = snapshot
            .source(&source.source_id)
            .is_ok_and(|active| active == &record);
        let routes = std::iter::once(&source.target)
            .chain(source.additional_native_endpoints.iter().map(|e| &e.target))
            .map(target_url)
            .chain(
                source
                    .decision_targets
                    .iter()
                    .map(|target| target.endpoint.clone()),
            )
            .map(|url| {
                native_network_route_view(&snapshot, &record.policy, &url).map_err(map_check_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let operation = stores
            .control()
            .writer_claim_operation()
            .map_err(map_port)?
            .filter(|op| {
                op.plan.spec().resource_id.as_deref() == Some(source.source_id.as_str())
                    && op
                        .plan
                        .control()
                        .get("compute_management_mutation")
                        .cloned()
                        .and_then(|m| serde_json::from_value::<ComputeManagementMutation>(m).ok())
                        .is_some_and(|m| m.network().is_some())
            });
        let pending = operation.as_ref().is_some_and(|op| !op.state.is_terminal());
        let available = routes.iter().all(|r| r.failure.is_none());
        let decisions = source
            .decision_targets
            .iter()
            .flat_map(|target| &target.consumers)
            .map(|consumer| {
                let service = stores
                    .control()
                    .decision_service_record(
                        &WorkspaceId::default(),
                        &consumer.service_id,
                        consumer.revision,
                    )
                    .map_err(map_port)?
                    .ok_or(ComputeManagementControlError::Corrupt)?;
                let display_name = match service {
                    hiroute_domain::DecisionServiceRecord::V1(service) => service.name,
                    hiroute_domain::DecisionServiceRecord::V2(service) => service.name,
                };
                Ok(NativeNetworkAffectedDecisionV1 {
                    service_id: consumer.service_id.clone(),
                    revision: consumer.revision,
                    display_name,
                    model: consumer.model.clone(),
                })
            })
            .collect::<Result<Vec<_>, ComputeManagementControlError>>()?;
        Ok(NativeSourceNetworkViewV1 {
            source_id: source.source_id,
            source_revision: source.revision,
            network_revision: record.network_revision,
            policy: record.policy,
            applied_revision: (applied && available).then_some(record.network_revision),
            application_state: if pending {
                NativeNetworkApplicationStateV1::Pending
            } else if applied && available {
                NativeNetworkApplicationStateV1::Applied
            } else {
                NativeNetworkApplicationStateV1::Unavailable
            },
            operation: operation.map(|op| OperationReferenceV1 {
                operation_id: op.operation_id.to_string(),
                state: op.state.as_str().into(),
                sequence: op.generation,
                cancellable: !op.state.is_terminal(),
            }),
            routes,
            decisions,
            models: source
                .models
                .into_iter()
                .map(|m| NativeNetworkAffectedModelV1 {
                    binding_id: m.binding_id,
                    display_name: m.display_name,
                })
                .collect(),
        })
    }

    pub(super) fn resolve_native_network_preview(
        &self,
        request: NativeNetworkRoutePreviewRequestV1,
    ) -> Result<NativeNetworkRouteViewV1, ComputeManagementControlError> {
        let (url, source) = match request.target {
            NativeNetworkPreviewTargetV1::Draft { endpoint_url } => (endpoint_url, None),
            NativeNetworkPreviewTargetV1::Decision { id, revision } => {
                let stores = self
                    .stores_lock()
                    .map_err(|_| ComputeManagementControlError::Unavailable)?;
                let service = stores
                    .control()
                    .decision_service_record(&WorkspaceId::default(), &id, revision)
                    .map_err(map_port)?
                    .ok_or(ComputeManagementControlError::NotFound)?;
                let hiroute_domain::DecisionServiceRecord::V2(service) = service else {
                    return Err(ComputeManagementControlError::Invalid);
                };
                let hiroute_domain::DecisionConnectionV2::Native {
                    endpoint,
                    authentication:
                        hiroute_domain::DecisionAuthenticationV2::SharedSource {
                            source_id,
                            source_lineage_digest,
                        },
                    ..
                } = service.connection
                else {
                    return Err(ComputeManagementControlError::Invalid);
                };
                let source = stores
                    .control()
                    .compute_management_source(&source_id)
                    .map_err(map_port)?
                    .ok_or(ComputeManagementControlError::NotFound)?;
                if source.lineage_digest != source_lineage_digest {
                    return Err(ComputeManagementControlError::Conflict);
                }
                (endpoint, Some((source_id, source_lineage_digest)))
            }
            NativeNetworkPreviewTargetV1::Saved {
                source_id,
                expected_source_revision,
                protocol,
            } => {
                let stores = self
                    .stores_lock()
                    .map_err(|_| ComputeManagementControlError::Unavailable)?;
                let source = stores
                    .control()
                    .compute_management_source(&source_id)
                    .map_err(map_port)?
                    .ok_or(ComputeManagementControlError::NotFound)?;
                if source.revision != expected_source_revision || !source.provenance.is_native() {
                    return Err(ComputeManagementControlError::Conflict);
                }
                let target = std::iter::once(&source.target)
                    .chain(source.additional_native_endpoints.iter().map(|e| &e.target))
                    .find(|t| protocol.is_none_or(|p| t.upstream_protocol == p))
                    .ok_or(ComputeManagementControlError::Invalid)?;
                (target_url(target), Some((source_id, source.lineage_digest)))
            }
            NativeNetworkPreviewTargetV1::Registered {
                connection_option_id,
                protocol,
            } => {
                let catalog = self
                    .current_catalog()
                    .ok_or(ComputeManagementControlError::Unavailable)?;
                let resolved = catalog
                    .resolve_connection_option(&connection_option_id)
                    .map_err(|_| ComputeManagementControlError::RegisteredOptionUnavailable)?;
                if resolved.connector.runtime_kind
                    != hiroute_domain::ConnectorRuntimeKind::BuiltinNative
                {
                    return Err(ComputeManagementControlError::Invalid);
                }
                let endpoint = resolved
                    .endpoint_profile
                    .protocol_endpoints
                    .iter()
                    .filter(|e| e.protocol == protocol)
                    .min_by_key(|e| e.stable_preference)
                    .ok_or(ComputeManagementControlError::Invalid)?;
                (endpoint.base_url.clone(), None)
            }
        };
        let snapshot = self
            .native_network
            .pin()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let record = source
            .as_ref()
            .map(|(id, lineage)| {
                let record = snapshot
                    .source(id)
                    .map_err(|_| ComputeManagementControlError::Unavailable)?;
                if &record.lineage_digest != lineage {
                    return Err(ComputeManagementControlError::Conflict);
                }
                Ok(record)
            })
            .transpose()?;
        let policy = match request.policy {
            Some(policy) => policy,
            None => match record {
                Some(record) => record.policy.clone(),
                None => NativeNetworkPolicyV1::Direct,
            },
        };
        match record {
            Some(record) => hiroute_integrations::native_network_route_view_at_revision(
                &snapshot,
                &policy,
                &url,
                record.network_revision,
            ),
            None => native_network_route_view(&snapshot, &policy, &url),
        }
        .map_err(map_check_error)
    }

    pub(super) fn preview_native_network_edit(
        &self,
        request: NativeSourceNetworkPreviewV1,
    ) -> Result<NativeNetworkSavePreviewV1, ComputeManagementControlError> {
        let before = self.read_native_source_network(NativeSourceNetworkQueryV1 {
            source_id: request.source_id.clone(),
        })?;
        if before.source_revision != request.expected_source_revision {
            return Err(ComputeManagementControlError::Conflict);
        }
        let change = {
            let stores = self
                .stores_lock()
                .map_err(|_| ComputeManagementControlError::Unavailable)?;
            let source = stores
                .control()
                .compute_management_source(&request.source_id)
                .map_err(map_port)?
                .ok_or(ComputeManagementControlError::NotFound)?;
            if source.revision != request.expected_source_revision {
                return Err(ComputeManagementControlError::Conflict);
            }
            ComputeManagementChangeV2 {
                schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
                subject: ComputeManagementSubjectV2::SavedSource {
                    source_id: source.source_id,
                },
                expected_revisions: stores
                    .control()
                    .current_revisions(&WorkspaceId::default())
                    .map_err(map_port)?,
                selected_model_refs: Vec::new(),
                intent: if source.state == MaterializationState::Ready {
                    ComputeManagementIntentV2::SaveReady
                } else {
                    ComputeManagementIntentV2::SaveDisabled
                },
                key_edits: Vec::new(),
                validation: None,
                edit: Some(ComputeManagementEditV1::Rename {
                    display_name: source.display_name,
                }),
            }
        };
        let after = request
            .network_edit
            .policy
            .normalized()
            .map_err(|_| ComputeManagementControlError::Invalid)?;
        let preview = self.preview_network_compute_save(ComputeManagementChangeV4 {
            schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4.into(),
            change: serde_json::to_value(change)
                .map_err(|_| ComputeManagementControlError::Invalid)?,
            network_edit: request.network_edit,
        })?;
        Ok(NativeNetworkSavePreviewV1 {
            preview,
            before,
            after,
        })
    }

    pub(super) fn validate_native_network_save(
        &self,
        change: &ComputeManagementChangeV2,
        edit: Option<&NativeNetworkEditV1>,
    ) -> Result<(), ComputeManagementControlError> {
        let ComputeManagementSubjectV2::Candidate { candidate } = &change.subject else {
            return Ok(());
        };
        let facts = self
            .model_connections
            .candidate_port()
            .resolve_compute_candidate(candidate)
            .map_err(map_port)?;
        if facts.producer != ComputeCandidateProducerV2::Native {
            return if edit.is_some() {
                Err(ComputeManagementControlError::Invalid)
            } else {
                Ok(())
            };
        }
        let context =
            self.native_network_check_context(facts.existing_source_id.as_deref(), edit.cloned())?;
        let evidence = match self.model_connections.network_check_evidence(candidate) {
            Ok(evidence) => evidence,
            // Local discovery has no remote route evidence to preserve or relabel. Its exact
            // configuration/credential guard is rechecked at Apply; policy shape and CAS are
            // admitted independently, without making a provider call a save prerequisite.
            Err(_) if facts.discovery_guard.is_some() => return Ok(()),
            Err(_) => return Err(ComputeManagementControlError::Conflict),
        };
        if evidence.policy != context.edit.policy
            || evidence.expected_network_revision != context.edit.expected_network_revision
        {
            return Err(ComputeManagementControlError::Conflict);
        }
        let current = native_network_route_view(
            &context.snapshot,
            &context.edit.policy,
            &evidence.route.destination,
        )
        .map_err(map_check_error)?;
        if current.state_revision != evidence.route.state_revision {
            return Err(ComputeManagementControlError::Conflict);
        }
        Ok(())
    }

    pub(super) fn validate_saved_network_state(
        &self,
        desired: &serde_json::Value,
        change: &ComputeManagementChangeV2,
    ) -> Result<(), ComputeManagementControlError> {
        let edit = if desired.get("schema").and_then(serde_json::Value::as_str)
            == Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4)
        {
            let network: NativeNetworkMutationV1 = serde_json::from_value(
                desired
                    .get("network")
                    .cloned()
                    .ok_or(ComputeManagementControlError::Invalid)?,
            )
            .map_err(|_| ComputeManagementControlError::Invalid)?;
            Some(NativeNetworkEditV1 {
                expected_network_revision: network
                    .before
                    .as_ref()
                    .map_or(0, |r| r.network_revision),
                policy: network
                    .after
                    .ok_or(ComputeManagementControlError::Invalid)?
                    .policy,
            })
        } else {
            None
        };
        self.validate_native_network_save(change, edit.as_ref())
    }

    pub(super) fn preview_network_compute_save(
        &self,
        mut change: ComputeManagementChangeV4,
    ) -> Result<ComputeSavePreviewV2, ComputeManagementControlError> {
        let catalog = self
            .current_catalog()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        let base = saved_compute_change(change.change.clone())
            .map_err(|_| ComputeManagementControlError::Invalid)?;
        self.validate_subscription_save_change(&base)?;
        self.bind_saved_key_inputs(&base)?;
        self.validate_native_network_save(&base, Some(&change.network_edit))?;
        if change
            .change
            .get("schema")
            .and_then(serde_json::Value::as_str)
            == Some(hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3)
        {
            let mut defaults: ComputeManagementChangeV3 = serde_json::from_value(change.change)
                .map_err(|_| ComputeManagementControlError::Invalid)?;
            self.establish_save_defaults(&mut defaults, &catalog)?;
            change.change = serde_json::to_value(defaults)
                .map_err(|_| ComputeManagementControlError::Invalid)?;
        }
        let stores = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let preview = ComputeManagementPlanner::new(
            self.model_connections.candidate_port(),
            stores.control(),
            stores.secrets(),
            self,
        )
        .preview_v4(change)
        .map_err(map_planning)?;
        self.validate_feed_save_plan(preview.plan(), &catalog)?;
        if self
            .current_catalog()
            .is_none_or(|current| current.generation != catalog.generation)
        {
            return Err(ComputeManagementControlError::Conflict);
        }
        Ok(preview.result)
    }
}
