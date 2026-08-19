pub const ENV_NAME: &str = "DBX_AGENT_FILE_EDIT_TOOLS";

pub fn enabled_from_env() -> bool {
    enabled_from_value(std::env::var(ENV_NAME).ok().as_deref())
}

fn enabled_from_value(value: Option<&str>) -> bool {
    !value.is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off"))
}

#[cfg(test)]
mod tests {
    use super::enabled_from_value;

    #[test]
    fn default_parser_only_disables_explicit_false_values() {
        assert!(enabled_from_value(None));
        assert!(enabled_from_value(Some("")));
        assert!(enabled_from_value(Some("1")));
        assert!(enabled_from_value(Some("unexpected")));
        assert!(!enabled_from_value(Some("0")));
        assert!(!enabled_from_value(Some(" FALSE ")));
        assert!(!enabled_from_value(Some("off")));
    }
}
