//! Navigable presentation help from the released command registry. No server operation is
//! invented for a family heading, and Planned descriptors are never advertised as callable.
use hiroute_application_api::released_commands;

pub(crate) fn family(path: &[String]) -> Option<String> {
    if path.is_empty() || path.iter().any(|part| part.starts_with('-')) {
        return None;
    }
    let commands = released_commands();
    if commands.iter().any(|command| command.path == path) {
        return None;
    }
    let children = commands
        .iter()
        .filter(|command| command.path.starts_with(path))
        .collect::<Vec<_>>();
    if children.is_empty() {
        return None;
    }
    let prefix = path.join(" ");
    let mut text =
        format!("HiRoute {prefix}\n\nUsage\n  hiroute {prefix} <command> [options]\n\nCommands\n");
    for command in children {
        text.push_str(&format!(
            "  {}\n    {}\n",
            command.path[path.len()..].join(" "),
            command.help.purpose
        ));
    }
    if path == ["decision"]
        || path == ["decision", "services"]
        || path == ["decision", "connections"]
    {
        text.push_str("  options [--provider ID]\n    Installed decision presets and reusable saved connection identities (read-only local presentation; catalog works offline).\n");
    }
    text.push_str("\nAppend --help to a command for its options and effects.\nRead complete installed contracts with:\n  hiroute schema show --command-id <ID> --include-schema\n\nBusiness commands emit one complete JSON result on stdout; capture stderr and the exit status separately. Add --error-details to opt into bounded structured input diagnostics; ordinary errors retain their machine shape. Input files use --request-file <PATH>, or --request-stdin for one complete JSON document.\n");
    Some(text)
}
