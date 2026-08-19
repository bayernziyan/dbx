use serde::{Deserialize, Serialize};

use super::contract::{ContractError, EditOperation, EditSpec};
use super::snapshot::{normalize_tool_text, EolKind};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedRange {
    pub operation: EditOperation,
    pub old_start_line: usize,
    pub old_end_line: usize,
    pub new_start_line: usize,
    pub new_end_line: usize,
}

#[derive(Debug)]
pub struct AppliedEdits {
    pub text: String,
    pub changed_ranges: Vec<ChangedRange>,
    pub additions: usize,
    pub deletions: usize,
}

struct LocatedEdit {
    operation: EditOperation,
    start: usize,
    end: usize,
    point: usize,
    old_text: String,
    new_text: String,
}

pub fn apply_edits(original: &str, eol: EolKind, edits: &[EditSpec]) -> Result<AppliedEdits, ContractError> {
    let mut located = Vec::with_capacity(edits.len());
    for edit in edits {
        let old_text = normalize_tool_text(&edit.old_text, eol)?;
        let new_text = normalize_tool_text(edit.new_text.as_deref().unwrap_or_default(), eol)?;
        let (start, end) = unique_match(original, &old_text)?;
        let point = match edit.op {
            EditOperation::InsertAfter => end,
            _ => start,
        };
        located.push(LocatedEdit { operation: edit.op, start, end, point, old_text, new_text });
    }
    ensure_non_overlapping(&located)?;

    let mut output = original.to_string();
    let mut descending = located.iter().collect::<Vec<_>>();
    descending.sort_by_key(|edit| std::cmp::Reverse((edit.point, edit.start, edit.end)));
    for edit in descending {
        match edit.operation {
            EditOperation::Replace => output.replace_range(edit.start..edit.end, &edit.new_text),
            EditOperation::Delete => output.replace_range(edit.start..edit.end, ""),
            EditOperation::InsertBefore | EditOperation::InsertAfter => output.insert_str(edit.point, &edit.new_text),
        }
    }

    let mut ascending = located.iter().collect::<Vec<_>>();
    ascending.sort_by_key(|edit| (edit.point, edit.start, edit.end));
    let mut byte_delta = 0isize;
    let mut changed_ranges = Vec::with_capacity(ascending.len());
    let mut additions = 0usize;
    let mut deletions = 0usize;
    for edit in ascending {
        let old_start_line = line_at(original, edit.start);
        let old_end_line = line_at(original, edit.end.saturating_sub(1).max(edit.start));
        let new_start_byte = (edit.point as isize + byte_delta).max(0) as usize;
        let new_end_byte = new_start_byte.saturating_add(edit.new_text.len());
        let new_start_line = line_at(&output, new_start_byte.min(output.len()));
        let new_end_line = line_at(&output, new_end_byte.saturating_sub(1).min(output.len()));
        let old_len = match edit.operation {
            EditOperation::InsertBefore | EditOperation::InsertAfter => 0,
            _ => edit.old_text.len(),
        };
        let new_len = match edit.operation {
            EditOperation::Delete => 0,
            _ => edit.new_text.len(),
        };
        byte_delta += new_len as isize - old_len as isize;
        additions += line_span_count(if new_len == 0 { "" } else { &edit.new_text });
        deletions += line_span_count(if old_len == 0 { "" } else { &edit.old_text });
        changed_ranges.push(ChangedRange {
            operation: edit.operation,
            old_start_line,
            old_end_line,
            new_start_line,
            new_end_line,
        });
    }
    Ok(AppliedEdits { text: output, changed_ranges, additions, deletions })
}

fn unique_match(content: &str, needle: &str) -> Result<(usize, usize), ContractError> {
    let mut matches = content.match_indices(needle);
    let Some((start, _)) = matches.next() else {
        return Err(ContractError::new("FILE_EDIT_ANCHOR_NOT_FOUND", "old_text was not found in the current file"));
    };
    if matches.next().is_some() {
        return Err(ContractError::new(
            "FILE_EDIT_ANCHOR_AMBIGUOUS",
            "old_text matched more than once; include more surrounding context",
        ));
    }
    Ok((start, start + needle.len()))
}

fn ensure_non_overlapping(edits: &[LocatedEdit]) -> Result<(), ContractError> {
    for (index, left) in edits.iter().enumerate() {
        for right in edits.iter().skip(index + 1) {
            let left_insert = matches!(left.operation, EditOperation::InsertBefore | EditOperation::InsertAfter);
            let right_insert = matches!(right.operation, EditOperation::InsertBefore | EditOperation::InsertAfter);
            let conflict = match (left_insert, right_insert) {
                (true, true) => left.point == right.point,
                (true, false) => left.point >= right.start && left.point <= right.end,
                (false, true) => right.point >= left.start && right.point <= left.end,
                (false, false) => left.start < right.end && right.start < left.end,
            };
            if conflict {
                return Err(ContractError::new(
                    "FILE_EDIT_OVERLAP",
                    "edit ranges overlap or insert at the same protected boundary",
                ));
            }
        }
    }
    Ok(())
}

fn line_at(content: &str, byte_offset: usize) -> usize {
    content.as_bytes()[..byte_offset.min(content.len())].iter().filter(|byte| **byte == b'\n').count() + 1
}

fn line_span_count(value: &str) -> usize {
    if value.is_empty() {
        0
    } else {
        value.bytes().filter(|byte| *byte == b'\n').count() + 1
    }
}

#[cfg(test)]
mod tests {
    use super::apply_edits;
    use crate::agent_files::local_edit::contract::{EditOperation, EditSpec};
    use crate::agent_files::local_edit::snapshot::EolKind;

    fn edit(op: EditOperation, old_text: &str, new_text: Option<&str>) -> EditSpec {
        EditSpec { op, old_text: old_text.to_string(), new_text: new_text.map(str::to_string), near_line: None }
    }

    #[test]
    fn applies_multiple_non_overlapping_edits_against_one_snapshot() {
        let result = apply_edits(
            "alpha\nbeta\ngamma\n",
            EolKind::Lf,
            &[edit(EditOperation::Replace, "alpha", Some("one")), edit(EditOperation::Replace, "gamma", Some("three"))],
        )
        .unwrap();
        assert_eq!(result.text, "one\nbeta\nthree\n");
    }

    #[test]
    fn refuses_ambiguous_anchor() {
        let error = apply_edits("same\nsame\n", EolKind::Lf, &[edit(EditOperation::Replace, "same", Some("changed"))])
            .unwrap_err();
        assert_eq!(error.code, "FILE_EDIT_ANCHOR_AMBIGUOUS");
    }

    #[test]
    fn supports_delete_and_insert_operations() {
        let result = apply_edits(
            "alpha\nbeta\ngamma\ndelta\n",
            EolKind::Lf,
            &[
                edit(EditOperation::InsertAfter, "alpha", Some("-inserted")),
                edit(EditOperation::Delete, "beta\n", None),
                edit(EditOperation::InsertBefore, "delta", Some("before-")),
            ],
        )
        .unwrap();
        assert_eq!(result.text, "alpha-inserted\ngamma\nbefore-delta\n");
    }
}
