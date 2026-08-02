//! Differential: Rust vs Python on captured FRED fixtures (no network).
//!
//! Compares against `fixtures/python-oracle.txt` — what the Python said when
//! stubs `fred_fetch.fetch_series` to read the CSVs — the same seam as
//! inflation-con/scripts/test_run.py. Compares three artefacts:
//!   1. history record line
//!   2. full rendered deliver message
//!   3. skill status
//!
//! Clock is pinned to PINNED_NOW on both sides (Python driver post-processes
//! the timestamp; Rust passes it into record_line).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use inflation_con::analysis::{classify, Obs, Series};
use inflation_con::config::{Config, DEFAULT_SERIES};
use inflation_con::fetch::fred_rows_or_empty;
use inflation_con::render::{format_message, record_line};
use market_fetch::fred::{parse_fred_csv, CreditError};

/// Must match the recorded oracle in fixtures/python-oracle.txt.
const PINNED_NOW: &str = "2026-07-30 12:00:00 CST";

const POLICY_STANCE: &str = "restrictive";

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Parse one fixture set's block from the recorded oracle.
#[derive(Debug)]
struct PyBlock {
    set_name: String,
    skill: String,
    classification: String,
    record: String,
    message: String,
}

fn parse_python_output(stdout: &str) -> Vec<PyBlock> {
    let mut blocks = Vec::new();
    let mut set_name = String::new();
    let mut skill = String::new();
    let mut classification = String::new();
    let mut record = String::new();
    let mut message = String::new();
    let mut mode = "";

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("===SET===") {
            // flush previous
            if !set_name.is_empty() {
                blocks.push(PyBlock {
                    set_name: set_name.clone(),
                    skill: skill.clone(),
                    classification: classification.clone(),
                    record: record.clone(),
                    message: message.clone(),
                });
            }
            set_name = rest.to_string();
            skill.clear();
            classification.clear();
            record.clear();
            message.clear();
            mode = "";
            continue;
        }
        if let Some(rest) = line.strip_prefix("===SKILL===") {
            skill = rest.to_string();
            mode = "";
            continue;
        }
        if let Some(rest) = line.strip_prefix("===CLASSIFICATION===") {
            classification = rest.to_string();
            mode = "";
            continue;
        }
        if line == "===RECORD===" {
            mode = "record";
            continue;
        }
        if line == "===MESSAGE===" {
            mode = "message";
            continue;
        }
        if line == "===END===" {
            mode = "";
            continue;
        }
        if line.starts_with("===DIAG===") || line.starts_with("===END_DIAG===") {
            mode = "skip";
            continue;
        }
        if line.starts_with("===NOW===") || line.starts_with("===WARNING===") {
            mode = "";
            continue;
        }
        if mode == "skip" {
            continue;
        }
        if mode == "record" {
            if !record.is_empty() {
                record.push('\n');
            }
            record.push_str(line);
        } else if mode == "message" {
            if !message.is_empty() {
                message.push('\n');
            }
            message.push_str(line);
        }
    }
    if !set_name.is_empty() {
        blocks.push(PyBlock {
            set_name,
            skill,
            classification,
            record,
            message,
        });
    }
    blocks
}

fn load_fixture_csv(dir: &Path, series_id: &str) -> Result<Vec<Obs>, CreditError> {
    let path = dir.join(format!("{series_id}.csv"));
    let text = std::fs::read_to_string(&path).map_err(|e| {
        CreditError::Http(format!("read {}: {e}", path.display()))
    })?;
    // Map NoData → empty the way fetch::fred_rows_or_empty does.
    let parsed = parse_fred_csv(series_id, &text);
    let rows = fred_rows_or_empty(parsed.map(|v| {
        v.into_iter()
            .map(|o| Obs {
                day: o.date,
                value: o.value,
            })
            .collect()
    }))?;
    Ok(rows)
}

fn load_series_map(dir: &Path) -> BTreeMap<String, Vec<Obs>> {
    let mut out = BTreeMap::new();
    for (key, sid) in DEFAULT_SERIES {
        let rows = load_fixture_csv(dir, sid).unwrap_or_else(|e| {
            panic!("load fixture {sid} from {}: {e}", dir.display())
        });
        out.insert((*key).to_string(), rows);
    }
    out
}

fn rust_artefacts(dir: &Path) -> (String, String, String, String) {
    let state = load_series_map(dir);
    let empty = Vec::new();
    let series = Series {
        core_pce: state.get("core_pce").cloned().unwrap_or_else(|| empty.clone()),
        core_cpi: state.get("core_cpi").cloned().unwrap_or_else(|| empty.clone()),
        breakeven_10y: state
            .get("breakeven_10y")
            .cloned()
            .unwrap_or_else(|| empty.clone()),
    };
    let (status, details) = classify(&series, POLICY_STANCE);
    let cfg = Config {
        series: DEFAULT_SERIES
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        policy_stance: POLICY_STANCE.into(),
    };
    // No warning on fixture paths (all seven series present with rows).
    let warning: Option<&str> = None;
    let (message, skill) = format_message(status, &details, &cfg, warning);
    let record = record_line(status, &details, warning, PINNED_NOW);
    let class = match status {
        inflation_con::analysis::Status::Ok => "OK",
        inflation_con::analysis::Status::Watch => "WATCH",
        inflation_con::analysis::Status::Yellow => "YELLOW",
        inflation_con::analysis::Status::Red => "RED",
        inflation_con::analysis::Status::InsufficientData => "INSUFFICIENT_DATA",
    };
    (record, message, skill.to_string(), class.to_string())
}

fn set_dir(name: &str) -> PathBuf {
    let root = fixtures_root();
    if name == "live" {
        root
    } else {
        root.join(name)
    }
}

/// Unified diff-ish report: every differing line, including whitespace.
fn report_diff(label: &str, python: &str, rust: &str) -> String {
    if python == rust {
        return format!("{label}: BYTE-IDENTICAL\n");
    }
    let mut out = format!("{label}: DIFFER\n");
    out.push_str(&format!("--- python ({label})\n+++ rust ({label})\n"));
    let p_lines: Vec<&str> = python.split('\n').collect();
    let r_lines: Vec<&str> = rust.split('\n').collect();
    let n = p_lines.len().max(r_lines.len());
    for i in 0..n {
        let pl = p_lines.get(i).copied().unwrap_or("<missing>");
        let rl = r_lines.get(i).copied().unwrap_or("<missing>");
        if pl != rl {
            out.push_str(&format!("@@ line {} @@\n- {pl:?}\n+ {rl:?}\n", i + 1));
            // also show raw with visible whitespace markers
            out.push_str(&format!("  py bytes: {:?}\n", pl.as_bytes()));
            out.push_str(&format!("  rs bytes: {:?}\n", rl.as_bytes()));
        }
    }
    // full dumps when short enough
    if python.len() < 4000 && rust.len() < 4000 {
        out.push_str("\n--- full python ---\n");
        out.push_str(python);
        out.push_str("\n--- full rust ---\n");
        out.push_str(rust);
        out.push('\n');
    }
    out
}

/// The Python's verdict, recorded once and committed.
///
/// This used to spawn `drive_python.py` on every run, which made the Python the
/// live source of truth for a skill that no longer runs it. The comparison
/// below is unchanged and still byte-for-byte — it just reads what the oracle
/// said from disk instead of re-deriving it, so the suite is offline,
/// python-free, and pinned to the answers the port was actually checked
/// against.
///
/// Regenerating it means resurrecting the Python from git history. That is the
/// point: these are answers, not a program, and they should only change when
/// someone decides they should.
fn python_oracle() -> String {
    let path = fixtures_root().join("python-oracle.txt");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert!(
        !text.trim().is_empty(),
        "{} is empty — the comparison would pass vacuously",
        path.display()
    );
    text
}

#[test]
fn differential_vs_python_on_fixtures() {
    let py_stdout = python_oracle();
    let blocks = parse_python_output(&py_stdout);
    assert!(
        !blocks.is_empty(),
        "no SET blocks in the recorded oracle:\n{py_stdout}"
    );

    let mut failures: Vec<String> = Vec::new();
    let mut report = String::new();

    for block in &blocks {
        let dir = set_dir(&block.set_name);
        assert!(
            dir.join("PCEPILFE.csv").is_file(),
            "fixture set '{}' missing PCEPILFE.csv under {}",
            block.set_name,
            dir.display()
        );

        let (rs_record, rs_message, rs_skill, rs_class) = rust_artefacts(&dir);

        report.push_str(&format!(
            "\n######## set={} classification py={} rs={} skill py={} rs={} ########\n",
            block.set_name, block.classification, rs_class, block.skill, rs_skill
        ));
        report.push_str(&format!("python record: {}\n", block.record));
        report.push_str(&format!("rust   record: {rs_record}\n"));
        report.push_str(&format!("python skill:  {}\n", block.skill));
        report.push_str(&format!("rust   skill:  {rs_skill}\n"));

        if block.skill != rs_skill {
            failures.push(format!(
                "[{}] skill status: python={:?} rust={:?}",
                block.set_name, block.skill, rs_skill
            ));
            report.push_str("SKILL: DIFFER\n");
        } else {
            report.push_str("SKILL: BYTE-IDENTICAL\n");
        }

        if block.classification != rs_class {
            failures.push(format!(
                "[{}] classification: python={:?} rust={:?}",
                block.set_name, block.classification, rs_class
            ));
        }

        let rec_diff = report_diff("RECORD", &block.record, &rs_record);
        report.push_str(&rec_diff);
        if block.record != rs_record {
            failures.push(format!("[{}] record line differs", block.set_name));
        }

        let msg_diff = report_diff("MESSAGE", &block.message, &rs_message);
        report.push_str(&msg_diff);
        if block.message != rs_message {
            failures.push(format!("[{}] message differs", block.set_name));
        }
    }

    // Always print the full comparison so Task 5's report has verbatim sides.
    eprintln!("{report}");

    if !failures.is_empty() {
        panic!(
            "differential mismatches (report every difference; do not fix in Task 5):\n{}",
            failures.join("\n")
        );
    }
}
