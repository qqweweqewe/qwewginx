use crate::config::PluginEntry;

pub fn entry_config_json(entry: &PluginEntry) -> String {
    let directives: Vec<_> = entry
        .directives
        .iter()
        .map(|d| {
            serde_json::json!({
                "name": d.name,
                "args": d.args,
            })
        })
        .collect();
    serde_json::json!({
        "name": entry.name,
        "version": entry.version,
        "directives": directives,
    })
    .to_string()
}

pub fn directive_arg<'a>(entry: &'a PluginEntry, name: &str) -> Option<&'a str> {
    entry
        .directives
        .iter()
        .find(|d| d.name == name)
        .and_then(|d| d.args.first())
        .map(|s| s.as_str())
}
