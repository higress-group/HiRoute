use super::*;
use serde::Deserialize;
use serde_json::Value;
pub async fn dispatch(
    name: &str,
    args: Value,
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    dependencies: &WorkerDependencyConfirmationState,
) -> Result<Value, DesktopFailure> {
    match name {
        "compute_subscriptions" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                source_id: Option<String>,
                expected_source_revision: Option<u64>,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = compute_subscriptions(
                window,
                state,
                input.source_id,
                input.expected_source_revision,
            )
            .await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "check_subscription" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                candidate: ComputeCandidateRefV2,
                language: String,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = check_subscription(window, state, input.candidate, input.language).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "get_subscription_check_result" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                operation: OperationReferenceV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = get_subscription_check_result(window, state, input.operation).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "recover_subscription_check" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = recover_subscription_check(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "close_subscription_check" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            close_subscription_check(window, state).await?;
            serde_json::to_value(()).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "release_subscription_check" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                validation: WebValidationRefV2,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = release_subscription_check(window, state, input.validation).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "compute_key_masks" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                query: ComputeKeyMasksQueryV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = compute_key_masks(window, state, input.query).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "trial_setup_snapshot" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = trial_setup_snapshot(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "compute_management_snapshot" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = compute_management_snapshot(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "compute_scan" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                native_target: Option<hiroute_domain::AgentNativeTargetV1>,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = compute_scan(window, state, input.native_target).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "prepare_discovered_model_connection" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                discovery: WebComputeDiscoveryRefV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = prepare_discovered_model_connection(window, state, input.discovery).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "compute_connection_options" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                include_request_model_ids: Option<bool>,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value =
                compute_connection_options(window, state, input.include_request_model_ids).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "register_protected_model_input" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                secret: String,
                source_id: Option<String>,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value =
                register_protected_model_input(window, state, input.secret, input.source_id)
                    .await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "release_protected_model_input" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: ComputeCandidateRefV2,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            release_protected_model_input(window, state, input.input).await?;
            serde_json::to_value(()).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "check_model_connection" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                request: NativeModelConnectionCheckRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = check_model_connection(window, state, input.request).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "check_saved_model_connection" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                request: SavedModelConnectionCheckRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = check_saved_model_connection(window, state, input.request).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "check_registered_model_connection" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                request: RegisteredModelConnectionCheckRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = check_registered_model_connection(window, state, input.request).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "cancel_model_connection_check" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                check_id: String,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            cancel_model_connection_check(window, state, input.check_id).await?;
            serde_json::to_value(()).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "preview_compute_save" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                change: WebComputeManagementChangeV2,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = preview_compute_save(window, state, input.change).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "apply_compute_save" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                request: WebComputeConnectionApplyRequestV1,
                language: String,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = apply_compute_save(window, state, input.request, input.language).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "get_compute_save_result" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                operation: OperationReferenceV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = get_compute_save_result(window, state, input.operation).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "manage_subscription_login" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                request: ComputeSubscriptionLoginRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = manage_subscription_login(window, state, input.request).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "submit_subscription_login_callback" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                login_ref: String,
                callback: String,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value =
                submit_subscription_login_callback(window, state, input.login_ref, input.callback)
                    .await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "observation_capacity_get" => {
            let _: hiroute_application_api::ClientEmptyRequestV1 = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            serde_json::to_value(observation_capacity_get(window, state).await?)
                .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "observation_capacity_set" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                input: hiroute_application_api::ObservationCapacityUpdateV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            serde_json::to_value(observation_capacity_set(window, state, input.input).await?)
                .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "observation_delete" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: ObservationDeleteInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = observation_delete(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_settings_get" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_settings_get(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_settings_set" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerSettingsV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_settings_set(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_executor_availability" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_executor_availability(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_task_plans" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerPlansRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_task_plans(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_dependencies_discover" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerDependenciesDiscoverRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_dependencies_discover(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_dependencies_select_prepare" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerDependenciesSelectRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value =
                worker_dependencies_select_prepare(window, dependencies, state, input.input)
                    .await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_dependencies_select_confirm" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerDependencyConfirmationInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value =
                worker_dependencies_select_confirm(window, state, dependencies, input.input)
                    .await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_dependencies_select_cancel" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerDependencyConfirmationInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            worker_dependencies_select_cancel(window, dependencies, input.input).await?;
            serde_json::to_value(()).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_task_list" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerListRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_task_list(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_task_status" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerStatusRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_task_status(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_task_result" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerResultRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_task_result(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_task_read" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerReadRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_task_read(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_task_wait" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerWaitRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_task_wait(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_task_confirm_residual" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerResidualConfirmRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_task_confirm_residual(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_task_cancel" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerCancelRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_task_cancel(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "worker_task_continue" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: WorkerContinueRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = worker_task_continue(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "decision_services" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = decision_services(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "save_decision_service" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: DecisionServiceSaveInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = save_decision_service(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "test_classifier_decision" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: ClassifierDecisionTestInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = test_classifier_decision(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "save_classifier_header_secret" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: ClassifierHeaderSecretSaveInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = save_classifier_header_secret(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "desktop_snapshot" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = desktop_snapshot(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "refresh_route_references" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                language: String,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = refresh_route_references(window, state, input.language).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "preview_plan_editor" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: EditorInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = preview_plan_editor(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "plan_editor_options" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: hiroute_application_api::PlanEditorOptionsRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = plan_editor_options(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "preview_rename" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: RenameInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = preview_rename(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "preview_restore_name" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: RestoreInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = preview_restore_name(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "preview_price_change" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: PriceEditInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = preview_price_change(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "model_reference_query" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: ModelCatalogQueryV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = model_reference_query(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "list_model_metadata" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                input: ListModelMetadataV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            serde_json::to_value(list_model_metadata(window, state, input.input).await?)
                .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "get_model_metadata" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                input: GetModelMetadataV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            serde_json::to_value(get_model_metadata(window, state, input.input).await?)
                .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "model_feed_status" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = model_feed_status(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "model_feed_refresh" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = model_feed_refresh(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "model_feed_settings_apply" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: ModelFeedSettingsRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = model_feed_settings_apply(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "model_feed_remove_preview" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                #[serde(default)]
                input: Option<ModelFeedRemovalQueryV2>,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = model_feed_remove_preview(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "model_feed_remove_apply" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: ModelFeedRemoveRequestV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = model_feed_remove_apply(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "model_availability" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = model_availability(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "effective_price_query" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: GetEffectivePricesV2,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = effective_price_query(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "agent_home_summary" => {
            let _input: ClientEmptyRequestV1 = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = agent_home_summary(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "agent_model_associations" => {
            let _input: ClientEmptyRequestV1 = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = agent_model_associations(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "trial_agent_choices" => {
            let _input: ClientEmptyRequestV1 = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = trial_agent_choices(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "trial_agent_model" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                context_id: String,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = trial_agent_model(window, state, input.context_id).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "agent_snapshot" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                native_targets: Option<Vec<hiroute_domain::AgentNativeTargetV1>>,
                context_id: Option<String>,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value =
                agent_snapshot(window, state, input.native_targets, input.context_id).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "retry_agent_settings" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: hiroute_application_api::AgentSettingsRetryV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = retry_agent_settings(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "preview_agent_previous_routing" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: AgentPreviousRoutingQueryV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = preview_agent_previous_routing(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "preview_agent_settings" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: AgentSettingsInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = preview_agent_settings(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "check_agent_authentication" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: AgentCheckInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = check_agent_authentication(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "check_agent_live" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: AgentCheckInput,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = check_agent_live(window, state, input.input).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "observe_operation" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = observe_operation(window, state).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "stop_observing" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {}
            let _: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            stop_observing(window, state)
                .await
                .map_err(DesktopFailure::from)?;
            serde_json::to_value(()).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "observation_read" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                request: hiroute_application_api::ObservationReadRequestV2,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            let value = observation_read(window, state, input.request).await?;
            serde_json::to_value(value).map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "web_confirmation_snapshot" => {
            let _: ClientEmptyRequestV1 = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            serde_json::to_value(window.confirmations.snapshot(window.id))
                .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "resolve_web_confirmation" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Arguments {
                input: crate::web_confirmation::WebConfirmationDecision,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            window.confirmations.resolve(
                window.id,
                &input.input.confirmation_id,
                input.input.accepted,
            )?;
            Ok(Value::Null)
        }
        "compute_defaults" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                input: hiroute_application_api::ComputeDefaultsQueryV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            serde_json::to_value(compute_defaults(window, state, input.input).await?)
                .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        "preview_compute_save_v3" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields, rename_all = "camelCase")]
            struct Arguments {
                change: WebComputeManagementChangeV2,
                default_preferences: hiroute_domain::SourceDefaultPreferencesV1,
            }
            let input: Arguments = serde_json::from_value(args)
                .map_err(|_| DesktopFailure::from("REQUEST_INVALID"))?;
            serde_json::to_value(
                preview_compute_save_v3(window, state, input.change, input.default_preferences)
                    .await?,
            )
            .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        _ => Err("COMMAND_UNKNOWN".into()),
    }
}
