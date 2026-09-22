use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use hiroute_e2e::p0::production::{
    ProductionBundle, ProductionError, ProductionRunOptions, ScenarioState, run_production_oracle,
    write_report as write_production_report,
};
use hiroute_e2e::{Profile, RunOptions, Scenario, run_suite};

#[derive(Debug, Parser)]
#[command(name = "hiroute-e2e")]
#[command(about = "Run hermetic black-box HiRoute product-path scenarios")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Prepare the current Gateway binary without executing a product scenario.
    PrepareCurrent,
    /// Verify the clean current candidate using the existing production runtime.
    RunCurrent {
        #[arg(long)]
        run_id: String,
        #[arg(long)]
        result: PathBuf,
        #[arg(long, default_value_t = 30)]
        timeout_seconds: u64,
    },
    /// Parse and semantically validate a scenario and process profile.
    Validate {
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long)]
        profile: Option<PathBuf>,
        #[arg(long = "case")]
        case_id: Option<String>,
    },
    /// Seal the deterministic production Oracle artifact manifest.
    SealProduction {
        #[arg(long, default_value = "e2e")]
        e2e_root: PathBuf,
    },
    /// Regenerate the P0 matrix index from its frozen typed registry.
    SealCoverage {
        #[arg(long, default_value = "e2e")]
        e2e_root: PathBuf,
    },
    /// Start a native mock and a normal production hirouted listener.
    Run {
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long)]
        profile: PathBuf,
        #[arg(long)]
        result: PathBuf,
        #[arg(long = "case")]
        case_id: Option<String>,
        #[arg(long, default_value_t = 30)]
        timeout_seconds: u64,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match Cli::parse().command {
        Command::PrepareCurrent => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()?;
            let result = hiroute_e2e::p0::production::prepare_current_candidate(&root)?;
            println!("{result}");
        }
        Command::RunCurrent {
            run_id,
            result,
            timeout_seconds,
        } => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()?;
            let parent = result
                .parent()
                .ok_or("result has no parent")?
                .canonicalize()?;
            if !parent.starts_with(root.join("target")) || result.exists() {
                return Err("current result must be new and inside candidate target".into());
            }
            let run = hiroute_e2e::p0::production::run_current_candidate(
                &root,
                &run_id,
                Duration::from_secs(timeout_seconds),
            )
            .await;
            let run = match run {
                Ok(run) => run,
                Err(error) => {
                    let code = match &error {
                        ProductionError::Collector(_) => "gateway_observation_incomplete",
                        ProductionError::Readiness(_) => "gateway_readiness_failed",
                        ProductionError::Provenance(_) => "gateway_identity_failed",
                        ProductionError::Contract(_) => "gateway_contract_failed",
                        _ => "gateway_process_failed",
                    };
                    println!(
                        "{}",
                        serde_json::json!({"schema":"hiroute.e2e.current-failure/v1", "request_run_id":run_id, "code":code})
                    );
                    return Err(error.into());
                }
            };
            let green = run.report.scenario_state == ScenarioState::Green;
            hiroute_e2e::p0::production::write_current_report(&result, &run)?;
            if !green {
                return Err(ProductionError::ScenarioRed.into());
            }
        }
        Command::Validate {
            schema,
            scenario,
            profile,
            case_id,
        } => {
            if let Some(schema) = schema {
                if case_id.is_some() {
                    return Err("--case is available only for a generic E2E scenario".into());
                }
                if !is_production_scenario(&scenario)? {
                    return Err("--schema is reserved for a production E2E scenario".into());
                }
                let bundle = ProductionBundle::load(&scenario, profile.as_deref(), &schema)?;
                println!("{}", serde_json::to_string_pretty(&bundle.summary())?);
            } else {
                let profile = profile.ok_or("legacy validation requires --profile")?;
                let scenario = Scenario::read(&scenario)?;
                let scenario = match case_id {
                    Some(case_id) => scenario.select_case(&case_id)?,
                    None => scenario,
                };
                let profile = Profile::read(&profile)?;
                let summary = scenario.validate_with(&profile)?;
                println!("{}", serde_json::to_string_pretty(&summary)?);
            }
        }
        Command::SealProduction { e2e_root } => {
            ProductionBundle::write_sealed_manifest(&e2e_root)?;
            let manifest = ProductionBundle::seal_manifest(&e2e_root)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        Command::SealCoverage { e2e_root } => {
            hiroute_e2e::p0::coverage::write_manifest(&e2e_root)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&hiroute_e2e::p0::coverage::manifest_value())?
            );
        }
        Command::Run {
            schema,
            scenario,
            profile,
            result,
            case_id,
            timeout_seconds,
        } => {
            let inferred_schema = scenario
                .parent()
                .and_then(|directory| directory.parent())
                .map(|root| root.join("schema"));
            let p0_schema = schema
                .or(inferred_schema)
                .filter(|path| path.join("p0-production-manifest.json").is_file());
            if is_production_scenario(&scenario)? {
                if case_id.is_some() {
                    return Err("--case cannot split a sealed production E2E scenario".into());
                }
                let schema = p0_schema.ok_or(
                    "production E2E scenario requires a schema directory with p0-production-manifest.json",
                )?;
                let bundle = ProductionBundle::load(&scenario, Some(&profile), &schema)?;
                let report = run_production_oracle(
                    &bundle,
                    ProductionRunOptions {
                        sut: bundle.resolve_sut()?,
                        timeout: Duration::from_secs(timeout_seconds),
                    },
                )
                .await?;
                write_production_report(&result, report.as_report())?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "schema_version": "hiroute.e2e.production-run-summary/v1",
                        "contract_digest": report.contract_digest,
                        "result_payload_digest": report.result_payload_digest,
                        "process_exit": report.process_exit,
                        "scenario_state": report.scenario_state,
                        "checkpoints": report.checkpoints,
                    }))?
                );
                if report.scenario_state != ScenarioState::Green {
                    return Err(ProductionError::ScenarioRed.into());
                }
            } else {
                let scenario = Scenario::read(&scenario)?;
                let scenario = match case_id {
                    Some(case_id) => scenario.select_case(&case_id)?,
                    None => scenario,
                };
                let profile = Profile::read(&profile)?;
                scenario.validate_with(&profile)?;
                match run_suite(
                    &scenario,
                    &profile,
                    RunOptions {
                        timeout: Duration::from_secs(timeout_seconds),
                    },
                )
                .await
                {
                    Ok(report) => {
                        hiroute_e2e::write_report(&result, &report)?;
                        println!("{}", serde_json::to_string_pretty(&report)?);
                    }
                    Err(failure) => {
                        hiroute_e2e::write_report(&result, &failure.report)?;
                        return Err(Box::new(failure) as Box<dyn std::error::Error + Send + Sync>);
                    }
                }
            }
        }
    }
    Ok(())
}

fn is_production_scenario(
    path: &std::path::Path,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        == Some("hiroute.e2e.production-scenario/v1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_scenarios_keep_generic_and_production_dispatch_distinct() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(!is_production_scenario(&root.join("e2e/scenarios/core-routing.json")).unwrap());
        assert!(is_production_scenario(&root.join("e2e/scenarios/p0-gateway.json")).unwrap());
    }
}
