//! Differential: Rust vs Python on committed chipcon fixtures (no network).
//!
//! Drives `fixtures/drive_python.py`, which imports run.py as a module and
//! substitutes `oil_fetch` and the `datetime` *class* — the seams the
//! pre-dispatch probe proved work without editing run.py.
//!
//! Compares three artefacts byte-for-byte:
//!   1. skill status
//!   2. full rendered deliver message
//!   3. history record line
//!
//! Clock is pinned to PINNED_NOW on both sides (Python via FakeDT.now;
//! Rust passes the string into record_line).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use chipcon::analysis::{classify, Row, Status};
use chipcon::config::{default_events, Config};
use chipcon::render::{format_message, record_line};

/// Must match fixtures/drive_python.py PINNED_NOW.
const PINNED_NOW: &str = "2026-04-15 12:00:00 CST";

const SYMBOLS: &[&str] = &["SMH", "QQQ", "SOXX"];

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn drive_python_path() -> PathBuf {
    fixtures_root().join("drive_python.py")
}

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

fn set_dir(name: &str) -> PathBuf {
    let root = fixtures_root();
    if name == "live" {
        let live = root.join("live");
        if live.join("SMH_rows.json").is_file() {
            return live;
        }
        return root;
    }
    root.join("synthetic").join(name)
}

fn load_fixture_rows(dir: &Path) -> BTreeMap<String, Vec<Row>> {
    let mut out = BTreeMap::new();
    for &sym in SYMBOLS {
        let path = dir.join(format!("{sym}_rows.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("read {}: {e}", path.display());
        });
        let v: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("json {}: {e}", path.display()));
        let rows: Vec<Row> = v
            .as_array()
            .unwrap_or_else(|| panic!("array expected in {}", path.display()))
            .iter()
            .map(|pair| {
                let a = pair
                    .as_array()
                    .unwrap_or_else(|| panic!("pair array in {}", path.display()));
                let day = a[0]
                    .as_str()
                    .unwrap_or_else(|| panic!("date str in {}", path.display()))
                    .to_string();
                let close = a[1]
                    .as_f64()
                    .unwrap_or_else(|| panic!("close f64 in {}", path.display()));
                Row { day, close }
            })
            .collect();
        out.insert(sym.to_string(), rows);
    }
    out
}

fn status_str(status: Status) -> &'static str {
    match status {
        Status::Ok => "OK",
        Status::Yellow => "YELLOW",
        Status::Orange => "ORANGE",
        Status::Red => "RED",
        Status::ProfitProtect => "PROFIT_PROTECT",
        Status::InsufficientHistory => "INSUFFICIENT_HISTORY",
    }
}

fn rust_artefacts(dir: &Path) -> (String, String, String, String) {
    let data = load_fixture_rows(dir);
    let empty: Vec<Row> = vec![];
    let smh = data.get("SMH").unwrap_or(&empty);
    let qqq = data.get("QQQ").unwrap_or(&empty);
    let soxx = data.get("SOXX").unwrap_or(&empty);

    let (status, details) = classify(smh, qqq, soxx);
    let cfg = Config {
        symbols: vec![
            ("SMH".into(), "SMH".into()),
            ("QQQ".into(), "QQQ".into()),
            ("SOXX".into(), "SOXX".into()),
        ],
        position_label: String::new(),
        manual_events: default_events(),
    };
    // No warning on fixture paths (all three symbols present with rows).
    let warning: Option<&str> = None;
    let (message, skill) = format_message(status, &details, &cfg, warning);
    let record = record_line(status, &details, warning, PINNED_NOW);
    (
        record,
        message,
        skill.to_string(),
        status_str(status).to_string(),
    )
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
            out.push_str(&format!("  py bytes: {:?}\n", pl.as_bytes()));
            out.push_str(&format!("  rs bytes: {:?}\n", rl.as_bytes()));
        }
    }
    if python.len() < 4000 && rust.len() < 4000 {
        out.push_str("\n--- full python ---\n");
        out.push_str(python);
        out.push_str("\n--- full rust ---\n");
        out.push_str(rust);
        out.push('\n');
    }
    out
}

fn run_python_driver() -> String {
    let script = drive_python_path();
    assert!(
        script.is_file(),
        "drive_python.py missing at {}",
        script.display()
    );
    let out = Command::new("python3")
        .arg(&script)
        .env("FIXTURE_ROOT", fixtures_root())
        // Refuse accidental network: clear proxy envs; driver never opens sockets.
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn python3: {e}"));
    if !out.status.success() {
        panic!(
            "drive_python.py failed ({}):\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8(out.stdout).expect("python stdout utf-8")
}

#[test]
fn differential_vs_python_on_fixtures() {
    let py_stdout = run_python_driver();
    let blocks = parse_python_output(&py_stdout);
    assert!(
        !blocks.is_empty(),
        "no SET blocks from drive_python.py:\n{py_stdout}"
    );

    let mut failures: Vec<String> = Vec::new();
    let mut report = String::new();

    // Track which classifications we covered (live + synthetic).
    let mut covered: Vec<String> = Vec::new();

    for block in &blocks {
        let dir = set_dir(&block.set_name);
        assert!(
            dir.join("SMH_rows.json").is_file(),
            "fixture set '{}' missing SMH_rows.json under {}",
            block.set_name,
            dir.display()
        );

        let (rs_record, rs_message, rs_skill, rs_class) = rust_artefacts(&dir);
        covered.push(format!("{}={}", block.set_name, block.classification));

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

    report.push_str(&format!("\ncovered sets: {}\n", covered.join(", ")));

    // Always print the full comparison so Task 5's report has verbatim sides.
    eprintln!("{report}");

    if !failures.is_empty() {
        panic!(
            "differential mismatches (report every difference; do not fix in Task 5):\n{}",
            failures.join("\n")
        );
    }
}
