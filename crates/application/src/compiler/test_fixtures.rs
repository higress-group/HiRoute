use std::collections::BTreeMap;

use hiroute_domain::*;

use super::*;

pub(crate) fn compilation_facts() -> AgentPlanCompilationFactsV1 {
    let mut candidates = vec![
        candidate(
            "economy-a",
            BillingClass::Paid,
            46,
            20,
            discrete_reasoning(),
            None,
        ),
        candidate(
            "economy-b",
            BillingClass::Paid,
            35,
            5,
            discrete_reasoning(),
            None,
        ),
        candidate(
            "economy-c",
            BillingClass::Paid,
            49,
            8,
            discrete_reasoning(),
            None,
        ),
        candidate(
            "primary-a",
            BillingClass::Paid,
            49,
            120,
            discrete_reasoning(),
            None,
        ),
        candidate(
            "primary-b",
            BillingClass::Paid,
            45,
            60,
            discrete_reasoning(),
            None,
        ),
        candidate(
            "free-a",
            BillingClass::Free,
            47,
            0,
            discrete_reasoning(),
            Some(FreeAccess::Direct),
        ),
        candidate(
            "free-b",
            BillingClass::Free,
            42,
            0,
            NativeReasoningCapabilityV1::Fixed {
                profile: "fixed".into(),
            },
            Some(FreeAccess::Direct),
        ),
        candidate(
            "free-toggle",
            BillingClass::Free,
            45,
            0,
            NativeReasoningCapabilityV1::Toggle {
                parameter: "reasoning".into(),
            },
            Some(FreeAccess::Direct),
        ),
        candidate(
            "free-budget",
            BillingClass::Free,
            44,
            0,
            NativeReasoningCapabilityV1::Budget {
                parameter: "thinking_budget".into(),
                minimum_tokens: 512,
                maximum_tokens: 4096,
                step_tokens: 512,
            },
            Some(FreeAccess::ApiKeyRequired),
        ),
    ];
    candidates
        .iter_mut()
        .find(|candidate| candidate.binding.binding_id == "binding/economy-c")
        .unwrap()
        .ordering_price = None;
    AgentPlanCompilationFactsV1 {
        schema: AGENT_PLAN_FACTS_SCHEMA_V1.into(),
        compiler_revision: AGENT_PLAN_COMPILER_REVISION_V1.into(),
        candidate_scope: CandidateFactScope::AllMaterializedBindings,
        refs: fact_refs(),
        ordering_price_version: "prices.v1".into(),
        ordering_price_digest: digest("prices"),
        candidates,
    }
}

fn candidate(
    id: &str,
    billing_class: BillingClass,
    score: u8,
    cost: u64,
    reasoning: NativeReasoningCapabilityV1,
    free_access: Option<FreeAccess>,
) -> CandidateCompilationFactV1 {
    let binding_id = format!("binding/{id}");
    let source_id = format!("source/{id}");
    let model_configuration_id = format!("model.{id}");
    let offer_ref = format!("offer/{id}");
    let capability_id = format!("capability/{id}");
    let endpoint_profile_id = format!("endpoint/{id}");
    let credential_pool_id = match (billing_class, free_access) {
        (BillingClass::Free, Some(FreeAccess::Direct)) => None,
        _ => Some(format!("pool/{id}")),
    };
    let credential_refs = vec![format!("credential/{id}")];
    let native_model = format!("upstream-{id}");
    let protocol_profiles = [UpstreamProtocol::Responses, UpstreamProtocol::Messages]
        .into_iter()
        .map(|ingress| {
            fixture_protocol_profile(
                ingress,
                &capability_id,
                &model_configuration_id,
                &native_model,
                &reasoning,
            )
        })
        .collect();
    CandidateCompilationFactV1 {
        authority: super::CandidateFactAuthorityV1::RegisteredCatalog,
        connection_option_id: format!("option/{id}"),
        offer_revision: 1,
        binding: SourceBindingV1 {
            binding_id,
            revision: 1,
            source_id,
            source_revision: 1,
            source_identity_digest: digest(&format!("source-identity-{id}")),
            model_data_bundle_version: "model-data.v1".into(),
            capability_slice_version: "capabilities.v1".into(),
            offer_ref: offer_ref.clone(),
            offer_evidence_digest: digest(&format!("offer-{id}")),
            billing_class,
            model_configuration_id: model_configuration_id.clone(),
            upstream_model_id: native_model.clone(),
            capability_id: capability_id.clone(),
            credential_pool_id,
        },
        model: ModelDefinitionV1 {
            model_configuration_id: model_configuration_id.clone(),
            revision: 1,
            display_name: format!("Model {id}"),
            publisher_id: "publisher.builtin".into(),
            capabilities: CapabilityFactsV1 {
                tool: true,
                vision: true,
                streaming: true,
                context_tokens: 128_000,
                max_output_tokens: 16_384,
            },
        },
        capability: ModelEndpointCapabilityV1 {
            capability_id,
            revision: 1,
            model_configuration_id: model_configuration_id.clone(),
            connector_id: "connector.builtin".into(),
            connector_revision: 1,
            endpoint_profile_id,
            endpoint_profile_revision: 1,
            protocol_endpoint_id: format!("protocol/{id}"),
            upstream_protocol: UpstreamProtocol::Responses,
            upstream_model_id: format!("upstream-{id}"),
            required_adapter_ref: "adapter.responses".into(),
            required_adapter_revision: 1,
            evidence_digest: digest(&format!("capability-{id}")),
        },
        protocol_endpoint: ProtocolEndpointV1 {
            protocol_endpoint_id: format!("protocol/{id}"),
            protocol: UpstreamProtocol::Responses,
            base_url: format!("https://{id}.provider.invalid"),
            request_path: "/v1/responses".into(),
            adapter_ref: "adapter.responses".into(),
            adapter_revision: 1,
            stable_preference: 1,
            inventory_path: None,
            authentication_semantics: None,
            required_headers: Vec::new(),
        },
        connector_runtime: ConnectorRuntimeKind::BuiltinNative,
        operational_target: GatewayOperationalTargetV1::RegisteredHttps {
            uri: format!("https://{id}.provider.invalid/v1/responses"),
        },
        native_transport_model: native_model,
        protocol_profiles,
        credential_refs,
        credential_destination_ref: None,
        source_state: MaterializationState::Ready,
        inventory_model_matched: true,
        reasoning,
        rating: Some(RatingV1 {
            model_configuration_id: model_configuration_id.clone(),
            overall_score_tenths: score,
            rating_count: 100,
        }),
        ordering_price: Some(OrderingPriceFactV1 {
            price_rate_id: format!("price/{id}"),
            price_rate_revision: 1,
            offer_ref,
            model_configuration_id,
            currency: "USD".into(),
            input_micros_per_million: cost / 2,
            output_micros_per_million: cost - cost / 2,
            frozen_digest: digest(&format!("price-{id}")),
        }),
        free_evidence: free_access.map(|access| FreeCandidateEvidenceV1 {
            free_offer_id: format!("free-offer/{id}"),
            free_offer_revision: 1,
            offer_ref: format!("offer/{id}"),
            access,
            evidence_digest: digest(&format!("free-{id}")),
        }),
    }
}

fn fixture_protocol_profile(
    ingress: UpstreamProtocol,
    capability_id: &str,
    model_configuration_id: &str,
    native_model: &str,
    reasoning: &NativeReasoningCapabilityV1,
) -> GatewayCandidateProtocolProfileV1 {
    let reasoning_profiles = fixture_reasoning_profiles(reasoning);
    let selected_reasoning_profile_id = reasoning_profiles[0].profile_id.clone();
    let exact = GatewayFidelityV1::Exact;
    GatewayCandidateProtocolProfileV1 {
        schema_version: "hiroute.candidate-protocol-profile/v1".into(),
        path_id: format!("fixture-{ingress:?}-to-responses").to_ascii_lowercase(),
        ingress_protocol: ingress,
        adapter_revision: "builtin-protocol-adapter/v1".into(),
        serializer_revision: "hiroute-target-json/v1".into(),
        decoder_revision: "hiroute-native-response/v1".into(),
        capability: GatewayCandidateCapabilityProfileV1 {
            schema_version: "hiroute.candidate-capability/v1".into(),
            capability_id: capability_id.into(),
            capability_revision: "1".into(),
            upstream_protocol: UpstreamProtocol::Responses,
            model_configuration_id: model_configuration_id.into(),
            native_model: native_model.into(),
            request: GatewayRequestFeatureProfileV1 {
                text: exact,
                initial_instructions: exact,
                mid_conversation_instructions: exact,
                image_url: exact,
                image_base64: exact,
                image_base64_media_types: GatewayCriticalFactV1::Exact(vec![
                    "image/gif".into(),
                    "image/jpeg".into(),
                    "image/png".into(),
                    "image/webp".into(),
                ]),
                function_tools: exact,
                strict_tools: exact,
                tool_choice_none: exact,
                tool_choice_auto: exact,
                tool_choice_required_any: exact,
                tool_choice_required_named: exact,
                parallel_tools: exact,
                tool_roundtrip: exact,
                tool_result_text: exact,
                tool_result_json: exact,
                logical_tool_id_mapping: exact,
                provider_state: GatewayFidelityV1::Unsupported,
                state_affinity: GatewayStateAffinityV1::Unsupported,
            },
            response: GatewayResponseFeatureProfileV1 {
                text: exact,
                reasoning: exact,
                refusal: exact,
                tool_calls: exact,
                logical_tool_id_mapping: exact,
                usage: exact,
                finish_reason: exact,
                typed_error: exact,
                provider_state: GatewayFidelityV1::Unsupported,
                state_affinity: GatewayStateAffinityV1::Unsupported,
                stream_refusal: GatewayStreamingRefusalSemanticsV1::ExactDelta,
                stream_text_delta: exact,
                stream_tool_argument_delta: exact,
                stream_reasoning_delta: exact,
                stream_usage: exact,
            },
            reasoning_profiles,
            selected_reasoning_profile_id,
            context: GatewayContextLimitsV1 {
                max_input_tokens: GatewayCriticalFactV1::Exact(128_000),
                max_output_tokens: GatewayCriticalFactV1::Exact(16_384),
                max_total_tokens: GatewayCriticalFactV1::Exact(Some(144_384)),
                estimator: GatewayCriticalFactV1::Exact(GatewayTokenEstimatorProfileV1 {
                    revision: "byte-upper-bound/v1".into(),
                    bytes_per_token: 1,
                    fixed_overhead_tokens: 0,
                }),
            },
            native_streaming: GatewayCriticalFactV1::Exact(true),
            native_provider_state: GatewayNativeProviderStateEmissionV1::Never,
        },
        connector: GatewayConnectorProfileV1 {
            schema_version: "hiroute.connector-profile/v1".into(),
            provider_id: "publisher.builtin".into(),
            endpoint_id: format!("endpoint/{native_model}"),
            entitlement_id: format!("entitlement/{native_model}"),
            connector_id: "connector.builtin".into(),
            connector_revision: "1".into(),
            upstream_protocol: UpstreamProtocol::Responses,
            request_path: "/v1/responses".into(),
            authentication: GatewayCriticalFactV1::Exact(GatewayAuthenticationSemanticsV1::Bearer),
            headers: GatewayCriticalFactV1::Exact(GatewayHeaderSemanticsV1 {
                content_type: "application/json".into(),
                required_headers: Vec::new(),
                forbidden_forward_headers: vec!["authorization".into()],
            }),
            errors: GatewayCriticalFactV1::Exact(GatewayErrorSemanticsV1 {
                http_status_typed: true,
                sse_error_typed: true,
                retry_after_header: Some("retry-after".into()),
            }),
        },
    }
}

fn fixture_reasoning_profiles(
    capability: &NativeReasoningCapabilityV1,
) -> Vec<GatewayReasoningProfileCapabilityV1> {
    match capability {
        NativeReasoningCapabilityV1::Fixed { profile } => vec![fixture_reasoning_profile(
            profile,
            GatewayReasoningControlKindV1::Fixed,
            GatewayNativeReasoningRenderV1::NoControlParameter,
        )],
        NativeReasoningCapabilityV1::Toggle { parameter } => [false, true]
            .into_iter()
            .map(|enabled| {
                fixture_reasoning_profile(
                    if enabled { "enabled" } else { "disabled" },
                    GatewayReasoningControlKindV1::Toggle,
                    GatewayNativeReasoningRenderV1::ExactFields {
                        protocol: UpstreamProtocol::Responses,
                        fields: vec![GatewayNativeReasoningFieldAssignmentV1 {
                            path: vec![parameter.clone()],
                            value: GatewayNativeReasoningValueV1::Bool(enabled),
                        }],
                    },
                )
            })
            .collect(),
        NativeReasoningCapabilityV1::Discrete {
            parameter,
            profiles,
        } => profiles
            .iter()
            .map(|profile| {
                fixture_reasoning_profile(
                    profile,
                    GatewayReasoningControlKindV1::Discrete,
                    GatewayNativeReasoningRenderV1::ExactFields {
                        protocol: UpstreamProtocol::Responses,
                        fields: vec![GatewayNativeReasoningFieldAssignmentV1 {
                            path: vec![parameter.clone()],
                            value: GatewayNativeReasoningValueV1::String(profile.clone()),
                        }],
                    },
                )
            })
            .collect(),
        NativeReasoningCapabilityV1::Budget {
            parameter,
            minimum_tokens,
            maximum_tokens,
            step_tokens,
        } => (*minimum_tokens..=*maximum_tokens)
            .step_by(*step_tokens as usize)
            .map(|tokens| {
                let path = vec![parameter.clone()];
                fixture_reasoning_profile(
                    &format!("budget-{tokens}"),
                    GatewayReasoningControlKindV1::Budget,
                    GatewayNativeReasoningRenderV1::ExactBudget {
                        protocol: UpstreamProtocol::Responses,
                        fields: vec![GatewayNativeReasoningFieldAssignmentV1 {
                            path: path.clone(),
                            value: GatewayNativeReasoningValueV1::U64(u64::from(tokens)),
                        }],
                        budget_path: path,
                        selected_tokens: u64::from(tokens),
                        min_tokens: u64::from(*minimum_tokens),
                        max_tokens: u64::from(*maximum_tokens),
                        step_tokens: u64::from(*step_tokens),
                    },
                )
            })
            .collect(),
    }
}

fn fixture_reasoning_profile(
    profile_id: &str,
    control_kind: GatewayReasoningControlKindV1,
    render: GatewayNativeReasoningRenderV1,
) -> GatewayReasoningProfileCapabilityV1 {
    GatewayReasoningProfileCapabilityV1 {
        profile_id: profile_id.into(),
        control_kind,
        render,
        accounting: GatewayReasoningAccountingV1::WithinOutputCap,
        additional_reservation_tokens: 0,
    }
}

fn discrete_reasoning() -> NativeReasoningCapabilityV1 {
    NativeReasoningCapabilityV1::Discrete {
        parameter: "reasoning_effort".into(),
        profiles: vec!["low".into(), "medium".into(), "high".into()],
    }
}

pub(crate) fn smart_desired() -> AgentPlanDesiredV1 {
    AgentPlanDesiredV1 {
        schema: AGENT_PLAN_DESIRED_SCHEMA_V1.into(),
        display_name: AgentPlanDisplayName::parse("Daily saving").unwrap(),
        purpose: AgentPlanPurpose::parse("Routine coding with a quality-protected economy path")
            .unwrap(),
        requirements: requirements(),
        limits: limits(),
        strategy: AgentPlanStrategyV1::SmartSaving {
            economy_candidates: vec![
                selection("economy-b", None),
                selection("economy-a", None),
                selection("economy-c", None),
            ],
            quality_anchor_binding_id: "binding/economy-a".into(),
            primary_candidates: vec![selection("primary-b", None), selection("primary-a", None)],
            quality_guard_score_gap_tenths: 2,
            complex_keywords: vec!["performance regression".into(), "性能回归".into()],
        },
    }
}

pub(crate) fn free_desired(fallback_policy: FreeFallbackPolicy) -> AgentPlanDesiredV1 {
    AgentPlanDesiredV1 {
        schema: AGENT_PLAN_DESIRED_SCHEMA_V1.into(),
        display_name: AgentPlanDisplayName::parse("Free research").unwrap(),
        purpose: AgentPlanPurpose::parse("Research tasks that prefer the pinned free pool")
            .unwrap(),
        requirements: requirements(),
        limits: limits(),
        strategy: AgentPlanStrategyV1::FreeFirst {
            free_pool: FreePoolSpecV1 {
                mode: FreePoolMode::AutomaticAllAvailable,
                candidates: Vec::new(),
                automatic_reasoning: BTreeMap::new(),
            },
            fallback_policy,
            primary_candidates: match fallback_policy {
                FreeFallbackPolicy::FreeOnly => Vec::new(),
                FreeFallbackPolicy::PrimaryFallback => {
                    vec![selection("primary-b", None), selection("primary-a", None)]
                }
            },
        },
    }
}

pub(crate) fn custom_desired() -> AgentPlanDesiredV1 {
    AgentPlanDesiredV1 {
        schema: AGENT_PLAN_DESIRED_SCHEMA_V1.into(),
        display_name: AgentPlanDisplayName::parse("Explicit chain").unwrap(),
        purpose: AgentPlanPurpose::parse("Use the exact user-selected candidate order").unwrap(),
        requirements: requirements(),
        limits: limits(),
        strategy: AgentPlanStrategyV1::Custom {
            candidates: vec![
                selection(
                    "primary-b",
                    Some(ReasoningSelectionV1::Profile {
                        profile: "low".into(),
                    }),
                ),
                selection(
                    "primary-a",
                    Some(ReasoningSelectionV1::Profile {
                        profile: "high".into(),
                    }),
                ),
            ],
        },
    }
}

fn free_fallback_with_exact_automatic_choices() -> AgentPlanDesiredV1 {
    let mut desired = free_desired(FreeFallbackPolicy::PrimaryFallback);
    let AgentPlanStrategyV1::FreeFirst { free_pool, .. } = &mut desired.strategy else {
        unreachable!()
    };
    free_pool.automatic_reasoning.insert(
        "binding/free-toggle".into(),
        ReasoningSelectionV1::Toggle { enabled: true },
    );
    free_pool.automatic_reasoning.insert(
        "binding/free-budget".into(),
        ReasoningSelectionV1::Budget { tokens: 1_024 },
    );
    desired
}

fn selection(id: &str, reasoning: Option<ReasoningSelectionV1>) -> CandidateSelectionV1 {
    CandidateSelectionV1 {
        binding_id: format!("binding/{id}"),
        reasoning,
    }
}

fn requirements() -> CapabilityRequirementsV1 {
    CapabilityRequirementsV1 {
        tool: true,
        vision: false,
        streaming: true,
        minimum_context_tokens: 32_000,
        minimum_output_tokens: 4_096,
    }
}

fn limits() -> RoutingLimitsV1 {
    RoutingLimitsV1 {
        context_window_tokens: None,
        maximum_attempts: 6,
        request_timeout_ms: 120_000,
        attempt_timeout_ms: 60_000,
    }
}

pub(crate) fn compiled_publication(revision: u64) -> GatewayPublicationV1 {
    let facts = compilation_facts();
    let desired = [
        smart_desired(),
        free_desired(FreeFallbackPolicy::FreeOnly),
        free_fallback_with_exact_automatic_choices(),
        custom_desired(),
    ];
    let ids = [
        "plan/smart",
        "plan/free",
        "plan/free-fallback",
        "plan/custom",
    ];
    let revisions = [3, 2, 1, 1];
    // This golden describes an already-published V1 workspace. Restore its historical alias
    // registry explicitly; new-plan allocation must never generate these names again.
    let mut registry = AliasRegistryV1 {
        next_sequence: 5,
        ..AliasRegistryV1::default()
    };
    for (id, alias) in [
        ("plan/custom", "hiroute/2590c10eeae4f930"),
        ("plan/free", "hiroute/2b52668dd5854ef1"),
        ("plan/free-fallback", "hiroute/6a87b28966202d08"),
        ("plan/smart", "hiroute/89e03a7d2c57bbc9"),
    ] {
        registry.active.insert(
            AgentPlanId::parse(id).unwrap(),
            ModelAlias::parse(alias).unwrap(),
        );
    }
    let plans = ids
        .into_iter()
        .zip(revisions)
        .zip(desired.iter())
        .map(|((id, plan_revision), desired)| {
            let agent_plan_id = AgentPlanId::parse(id).unwrap();
            let model_alias = registry.allocate(agent_plan_id.clone()).unwrap();
            let identity = AgentPlanIdentityV1 {
                agent_plan_id,
                model_alias,
                display_name: desired.display_name.clone(),
                purpose: desired.purpose.clone(),
            };
            compile_agent_plan(identity, plan_revision, desired, &facts).unwrap()
        })
        .collect::<Vec<_>>();
    let grants = publication_grants(&plans);
    compile_publication(
        WorkspaceId::default(),
        "workspace/personal/default/gateway",
        1,
        GatewayPublicationRevision::new(revision).unwrap(),
        DEFAULT_CATALOG_RENDERER_REVISION,
        registry,
        plans,
        grants,
    )
    .unwrap()
}

pub(crate) fn publication_grants(plans: &[CompiledAgentPlanV1]) -> Vec<GatewayAccessGrantV1> {
    let routes = |protocol| {
        plans
            .iter()
            .filter(|plan| {
                protocol == AgentIngressProtocolV1::Responses
                    || matches!(plan.agent_plan_id().as_str(), "plan/smart" | "plan/custom")
            })
            .map(|plan| {
                (
                    plan.model_alias().as_str().to_owned(),
                    AgentModelRouteV2::Plan {
                        plan_id: plan.agent_plan_id().clone(),
                        alias: plan.model_alias().clone(),
                        revision: plan.body.agent_plan_revision,
                        semantic_digest: plan.body.materialized_route_digest.clone(),
                    },
                )
            })
            .collect()
    };
    let responses = AgentModelGrantV2::seal(
        AgentIngressProtocolV1::Responses,
        routes(AgentIngressProtocolV1::Responses),
    )
    .unwrap();
    let messages = AgentModelGrantV2::seal(
        AgentIngressProtocolV1::Messages,
        routes(AgentIngressProtocolV1::Messages),
    )
    .unwrap();
    vec![
        GatewayAccessGrantV1::new(
            "grant/codex",
            7,
            digest("codex-grant-verifier"),
            AgentIngressProtocolV1::Responses,
            responses,
        )
        .unwrap(),
        GatewayAccessGrantV1::new(
            "grant/claude",
            3,
            digest("claude-grant-verifier"),
            AgentIngressProtocolV1::Messages,
            messages,
        )
        .unwrap(),
    ]
}

pub(crate) fn fact_refs() -> AgentPlanFactRefsV1 {
    AgentPlanFactRefsV1 {
        connector_registry_version: "registry.v1".into(),
        connector_registry_digest: digest("registry"),
        model_data_bundle_version: "model-data.v1".into(),
        model_data_digest: digest("model-data"),
        capability_slice_version: "capabilities.v1".into(),
        capability_slice_digest: digest("capabilities"),
        ratings_slice_version: "ratings.v1".into(),
        ratings_slice_digest: digest("ratings"),
        free_offers_slice_version: "free-offers.v1".into(),
        free_offers_slice_digest: digest("free-offers"),
        inventory_revision: 9,
        inventory_digest: digest("inventory"),
        price_tracking: PriceTrackingMode::FollowLatest,
    }
}

fn digest(value: &str) -> CanonicalDigest {
    CanonicalDigest::of_bytes(value.as_bytes())
}
