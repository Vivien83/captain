use std::path::{Path, PathBuf};

use crate::{cli_captain_home, ui};

use super::DoctorReport;

#[derive(Debug)]
struct BrandFinding {
    path: PathBuf,
    line: usize,
    term: String,
}

pub(super) fn check_brand_audit(report: &mut DoctorReport) {
    if !report.json {
        println!("\n  Brand Audit:");
    }
    let mut findings = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        collect_brand_findings(&cwd, &mut findings, 200);
    }
    let captain_dir = cli_captain_home();
    if captain_dir.exists() {
        collect_brand_findings(&captain_dir, &mut findings, 200);
    }
    if findings.is_empty() {
        if !report.json {
            ui::check_ok("No legacy public branding found in scanned text files");
        }
        report.push(serde_json::json!({"check": "brand_audit", "status": "ok"}));
    } else {
        print_brand_findings(report, &findings);
        report.push(serde_json::json!({
            "check": "brand_audit",
            "status": "fail",
            "count": findings.len(),
            "findings": findings.iter().take(50).map(|f| serde_json::json!({
                "path": f.path.display().to_string(),
                "line": f.line,
                "term": f.term,
            })).collect::<Vec<_>>()
        }));
        report.fail();
    }
}

fn print_brand_findings(report: &DoctorReport, findings: &[BrandFinding]) {
    if report.json {
        return;
    }
    ui::check_fail(&format!(
        "Legacy branding found in {} location(s)",
        findings.len()
    ));
    for f in findings.iter().take(25) {
        println!("    {}:{}  {}", f.path.display(), f.line, f.term);
    }
    if findings.len() > 25 {
        println!("    ... {} more", findings.len() - 25);
    }
}

fn brand_audit_should_skip_dir(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    matches!(
        name,
        ".git"
            | "target"
            | "node_modules"
            | ".code-review-graph"
            | "dist"
            | ".next"
            | ".claude"
            | ".hora"
    )
}

fn brand_audit_should_skip_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    name.starts_with("RAPPORT_")
        || name.starts_with("REPRISE_")
        || matches!(name, "rapport_codex.md")
}

fn brand_audit_line_allowlisted(path: &Path, line: &str) -> bool {
    let path_str = path.display().to_string();
    (path_str.ends_with("crates/captain-cli/src/commands/doctor/brand.rs")
        && (line.contains("let terms =")
            || line.contains(".horaclaw")
            || line.contains("HoraClaw")
            || line.contains("horaclaw")))
        || (path_str.ends_with("crates/captain-kernel/src/config.rs")
            && line.contains("home.join(\".horaclaw\")"))
}

fn brand_audit_text_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()).unwrap_or(""),
        "md" | "rs"
            | "toml"
            | "json"
            | "sh"
            | "ps1"
            | "py"
            | "yml"
            | "yaml"
            | "txt"
            | "service"
            | "env"
    )
}

fn collect_brand_findings(root: &Path, out: &mut Vec<BrandFinding>, max: usize) {
    if out.len() >= max || brand_audit_should_skip_dir(root) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let terms = ["horaclaw", "HoraClaw"];
    for entry in entries.flatten() {
        if out.len() >= max {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_brand_findings(&path, out, max);
            continue;
        }
        if !brand_audit_text_file(&path) || brand_audit_should_skip_file(&path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (idx, line) in text.lines().enumerate() {
            if brand_audit_line_allowlisted(&path, line) {
                continue;
            }
            for term in terms {
                if line.contains(term) {
                    out.push(BrandFinding {
                        path: path.clone(),
                        line: idx + 1,
                        term: term.to_string(),
                    });
                    break;
                }
            }
            if out.len() >= max {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_audit_detects_text_files() {
        assert!(brand_audit_text_file(Path::new("README.md")));
        assert!(brand_audit_text_file(Path::new("service.env")));
        assert!(!brand_audit_text_file(Path::new("image.png")));
    }

    #[test]
    fn brand_audit_allows_own_legacy_terms() {
        let path = Path::new("crates/captain-cli/src/commands/doctor/brand.rs");
        assert!(brand_audit_line_allowlisted(
            path,
            "    let terms = [\"horaclaw\", \"HoraClaw\"];"
        ));
    }
}
