pub fn enabled_from_env() -> bool {
    enabled_from_value(std::env::var("DBX_AGENT_FILE_DELETE_TOOLS").ok().as_deref())
}

fn enabled_from_value(value: Option<&str>) -> bool {
    !value.is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off"))
}

#[cfg(test)]
mod tests {
    use super::enabled_from_value;

    #[test]
    fn defaults_on_and_only_explicit_false_values_disable() {
        for value in [None, Some(""), Some("1"), Some("true"), Some("on"), Some("anything")] {
            assert!(enabled_from_value(value), "unexpected disabled value: {value:?}");
        }
        for value in [Some("0"), Some(" false "), Some("OFF")] {
            assert!(!enabled_from_value(value), "unexpected enabled value: {value:?}");
        }
    }
}
