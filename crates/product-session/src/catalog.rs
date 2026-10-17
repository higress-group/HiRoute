//! The checked command contract shared by the native bridge and console route admission.
use serde::Deserialize;
use std::sync::OnceLock;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandArgument {
    pub name: String,
    pub r#type: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductCommand {
    pub name: String,
    pub args: Vec<CommandArgument>,
    pub result: String,
    pub host_only: bool,
}
pub fn commands() -> &'static [ProductCommand] {
    static COMMANDS: OnceLock<Vec<ProductCommand>> = OnceLock::new();
    COMMANDS.get_or_init(|| {
        serde_json::from_str(include_str!("commands.json")).expect("checked command catalog")
    })
}
pub fn product_command(name: &str) -> Option<&'static ProductCommand> {
    commands().iter().find(|c| c.name == name && !c.host_only)
}
