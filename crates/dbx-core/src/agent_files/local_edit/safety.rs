use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use super::contract::ContractError;

#[derive(Debug)]
pub struct SafetyOutcome {
    pub existing_findings: Vec<&'static str>,
}

struct Rule {
    category: &'static str,
    regex: Regex,
}

pub fn validate_no_new_sensitive_content(old: &str, new: &str) -> Result<SafetyOutcome, ContractError> {
    let mut existing_findings = Vec::new();
    for rule in rules() {
        let old_values = rule.regex.find_iter(old).map(|found| found.as_str()).collect::<HashSet<_>>();
        let new_values = rule.regex.find_iter(new).map(|found| found.as_str()).collect::<HashSet<_>>();
        let old_count = rule.regex.find_iter(old).count();
        let new_count = rule.regex.find_iter(new).count();
        if new_count > old_count || !new_values.is_subset(&old_values) {
            return Err(ContractError::new(
                "FILE_EDIT_SENSITIVE_CONTENT_BLOCKED",
                format!("the edit introduces blocked sensitive content category: {}", rule.category),
            ));
        }
        if old_count > 0 {
            existing_findings.push(rule.category);
        }
    }
    existing_findings.sort_unstable();
    existing_findings.dedup();
    Ok(SafetyOutcome { existing_findings })
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            Rule {
                category: "private-key",
                regex: Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----").unwrap(),
            },
            Rule {
                category: "credential-assignment",
                regex: Regex::new(
                    r#"(?i)(api[_-]?key|access[_-]?token|password|client[_-]?secret)\s*[:=]\s*["']?[A-Za-z0-9_./+~=-]{8,}"#,
                )
                .unwrap(),
            },
            Rule {
                category: "credential-url",
                regex: Regex::new(r"[A-Za-z][A-Za-z0-9+.-]*://[^/\s:@]+:[^/\s@]+@").unwrap(),
            },
            Rule {
                category: "email",
                regex: Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").unwrap(),
            },
            Rule { category: "phone", regex: Regex::new(r"\b1[3-9][0-9]{9}\b").unwrap() },
            Rule { category: "identity-number", regex: Regex::new(r"\b[0-9]{17}[0-9Xx]\b").unwrap() },
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::validate_no_new_sensitive_content;

    #[test]
    fn allows_existing_finding_but_blocks_new_one() {
        let existing = "contact: a@example.com\nstatus: ready\n";
        let changed = "contact: a@example.com\nstatus: done\n";
        assert_eq!(validate_no_new_sensitive_content(existing, changed).unwrap().existing_findings, ["email"]);

        let error = validate_no_new_sensitive_content("status: ready\n", changed).unwrap_err();
        assert_eq!(error.code, "FILE_EDIT_SENSITIVE_CONTENT_BLOCKED");

        let replacement = "contact: b@example.com\nstatus: ready\n";
        let error = validate_no_new_sensitive_content(existing, replacement).unwrap_err();
        assert_eq!(error.code, "FILE_EDIT_SENSITIVE_CONTENT_BLOCKED");
    }
}
