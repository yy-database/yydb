//! Read-only `YDPG` / `YYWL` v3 consistency probes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use yydb_types::{DoctorIssue, DoctorSeverity, Error, Result};

use crate::header::{parse_page0, PAGE_MAGIC, PAGE_SIZE, SLOT_BYTES};
use crate::internal::InternalPage;
use crate::key::TreeKey;
use crate::leaf::LeafPage;
use crate::page::{PageHeader, PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};
use crate::wal::parse_wal;
use crate::wal_append::wal_sidecar_path;

/// Probe a `YDPG` main file and optional `YYWL` v3 sidecar.
pub fn diagnose_file(path: &Path) -> Result<Vec<DoctorIssue>> {
    let bytes = std::fs::read(path)?;
    let mut issues = probe_main_bytes(&bytes);
    let wal_path = wal_sidecar_path(path);
    if wal_path.exists() {
        issues.extend(probe_wal_bytes(&std::fs::read(wal_path)?));
    }
    Ok(issues)
}

fn probe_main_bytes(bytes: &[u8]) -> Vec<DoctorIssue> {
    let mut issues = Vec::new();
    if bytes.len() < PAGE_SIZE {
        issues.push(issue_error(
            "yydb.doctor.corrupt_header",
            "main file shorter than one page",
            None,
        ));
        return issues;
    }
    if bytes.get(0..5) != Some(PAGE_MAGIC.as_slice()) {
        issues.push(issue_error(
            "yydb.doctor.corrupt_header",
            "main file magic is not YDPG",
            None,
        ));
        return issues;
    }
    issues.extend(probe_header_slots(bytes));
    if issues.iter().any(|issue| {
        issue.code == "yydb.doctor.corrupt_header" && issue.severity == DoctorSeverity::Error
    }) {
        return issues;
    }
    let header = match parse_page0(bytes) {
        Ok(header) => header,
        Err(_) => {
            issues.push(issue_error(
                "yydb.doctor.corrupt_header",
                "page 0 could not be parsed",
                None,
            ));
            return issues;
        }
    };
    if bytes.len() % PAGE_SIZE != 0 {
        issues.push(issue_error(
            "yydb.doctor.corrupt_page",
            "main file length is not a multiple of the page size",
            None,
        ));
    }
    let page_count = bytes.len() / PAGE_SIZE;
    let mut pages = BTreeMap::new();
    for page_id in 0..page_count as u32 {
        let start = page_id as usize * PAGE_SIZE;
        let image = bytes[start..start + PAGE_SIZE].to_vec();
        if page_id == 0 {
            pages.insert(page_id, image);
            continue;
        }
        match PageHeader::parse(&image) {
            Ok(header) => {
                if header.page_id != page_id {
                    issues.push(issue_error(
                        "yydb.doctor.corrupt_page",
                        format!("page id {page_id} header disagrees with offset"),
                        Some(page_id.to_string()),
                    ));
                }
                pages.insert(page_id, image);
            }
            Err(_) => {
                issues.push(issue_error(
                    "yydb.doctor.corrupt_page",
                    format!("page {page_id} checksum or layout is invalid"),
                    Some(page_id.to_string()),
                ));
            }
        }
    }
    if header.slot.record_root != 0 {
        issues.extend(probe_record_tree(&pages, header.slot.record_root));
    }
    issues
}

fn probe_header_slots(bytes: &[u8]) -> Vec<DoctorIssue> {
    let mut issues = Vec::new();
    let slot_a_ok = slot_checksum_ok(bytes, 0);
    let slot_b_ok = slot_checksum_ok(bytes, SLOT_BYTES);
    if !slot_a_ok {
        issues.push(issue_warn(
            "yydb.doctor.corrupt_header",
            "header slot A checksum mismatch",
            Some("slot_a".into()),
        ));
    }
    if !slot_b_ok {
        issues.push(issue_warn(
            "yydb.doctor.corrupt_header",
            "header slot B checksum mismatch",
            Some("slot_b".into()),
        ));
    }
    if !slot_a_ok && !slot_b_ok {
        issues.push(issue_error(
            "yydb.doctor.corrupt_header",
            "both header slots failed checksum validation",
            None,
        ));
    }
    issues
}

fn slot_checksum_ok(bytes: &[u8], offset: usize) -> bool {
    let end = offset + SLOT_BYTES;
    if bytes.len() < end {
        return false;
    }
    if bytes.get(offset..offset + 5) != Some(PAGE_MAGIC.as_slice()) {
        return false;
    }
    let slot = &bytes[offset..end];
    let stored = u32::from_le_bytes(slot[2044..2048].try_into().unwrap_or([0, 0, 0, 0]));
    crate::crc32c::crc32c(&slot[..2044]) == stored
}

fn probe_record_tree(pages: &BTreeMap<u32, Vec<u8>>, root: u32) -> Vec<DoctorIssue> {
    let mut issues = Vec::new();
    let mut visited = BTreeSet::new();
    visit_tree_page(pages, root, &mut visited, &mut issues);
    issues
}

fn visit_tree_page(
    pages: &BTreeMap<u32, Vec<u8>>,
    page_id: u32,
    visited: &mut BTreeSet<u32>,
    issues: &mut Vec<DoctorIssue>,
) {
    if !visited.insert(page_id) {
        issues.push(issue_error(
            "yydb.doctor.corrupt_tree",
            format!("record tree revisits page {page_id}"),
            Some(page_id.to_string()),
        ));
        return;
    }
    let image = pages.get(&page_id);
    let image = match image {
        Some(image) => image,
        None => {
            issues.push(issue_error(
                "yydb.doctor.corrupt_tree",
                format!("record tree references missing page {page_id}"),
                Some(page_id.to_string()),
            ));
            return;
        }
    };
    let header = match PageHeader::parse(image) {
        Ok(header) => header,
        Err(_) => {
            issues.push(issue_error(
                "yydb.doctor.corrupt_page",
                format!("record tree page {page_id} failed checksum validation"),
                Some(page_id.to_string()),
            ));
            return;
        }
    };
    match header.page_type {
        PAGE_TYPE_LEAF => {
            let leaf = match LeafPage::decode(page_id, image) {
                Ok(leaf) => leaf,
                Err(_) => {
                    issues.push(issue_error(
                        "yydb.doctor.corrupt_tree",
                        format!("leaf page {page_id} could not be decoded"),
                        Some(page_id.to_string()),
                    ));
                    return;
                }
            };
            let mut prev: Option<&TreeKey> = None;
            for cell in leaf.cells() {
                if prev.is_some_and(|left| left >= &cell.key) {
                    issues.push(issue_error(
                        "yydb.doctor.corrupt_tree",
                        format!("leaf page {page_id} keys are out of order"),
                        Some(page_id.to_string()),
                    ));
                    break;
                }
                prev = Some(&cell.key);
            }
        }
        PAGE_TYPE_INTERNAL => {
            let node = match InternalPage::decode(page_id, image) {
                Ok(node) => node,
                Err(_) => {
                    issues.push(issue_error(
                        "yydb.doctor.corrupt_tree",
                        format!("internal page {page_id} could not be decoded"),
                        Some(page_id.to_string()),
                    ));
                    return;
                }
            };
            for idx in 2..node.entries.len() {
                if node.entries[idx - 1].separator >= node.entries[idx].separator {
                    issues.push(issue_error(
                        "yydb.doctor.corrupt_tree",
                        format!("internal page {page_id} separators are out of order"),
                        Some(page_id.to_string()),
                    ));
                    break;
                }
            }
            for idx in 0..node.entries.len() {
                visit_tree_page(pages, node.child_page_id(idx), visited, issues);
            }
        }
        _ => {
            issues.push(issue_error(
                "yydb.doctor.corrupt_tree",
                format!("record tree page {page_id} has unexpected page type"),
                Some(page_id.to_string()),
            ));
        }
    }
}

fn probe_wal_bytes(bytes: &[u8]) -> Vec<DoctorIssue> {
    match parse_wal(bytes) {
        Ok(_) => Vec::new(),
        Err(Error::Corrupt(message)) => vec![issue_error("yydb.doctor.corrupt_wal", message, None)],
        Err(error) => vec![issue_error(
            "yydb.doctor.corrupt_wal",
            error.to_string(),
            None,
        )],
    }
}

fn issue_error(code: &str, message: impl Into<String>, key_hint: Option<String>) -> DoctorIssue {
    DoctorIssue {
        severity: DoctorSeverity::Error,
        code: code.into(),
        message: message.into(),
        key_hint,
    }
}

fn issue_warn(code: &str, message: impl Into<String>, key_hint: Option<String>) -> DoctorIssue {
    DoctorIssue {
        severity: DoctorSeverity::Warn,
        code: code.into(),
        message: message.into(),
        key_hint,
    }
}
