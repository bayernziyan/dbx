pub fn redact_absolute_paths(value: &str) -> String {
    value.replace('\\', "/")
}
