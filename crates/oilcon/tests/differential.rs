//! Differential: Rust vs Python on committed oil fixtures (no network).
//!
//! Drives `fixtures/drive_python.py`, which imports run.py as a module and
//! substitutes `oil_fetch`, `oil_store`, and `cst_now` — the same seam the
//! Task 6 probe proved works without editing run.py.
//!
//! Compares three artefacts byte-for-byte:
//!   1. skill status
//!   2. full rendered deliver message
//!   3. history record line
//!
//! Also asserts that after seeding each side through its own writer, the
//! window row sequences match (same data, different schemas).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use market_fetch::yahoo::FetchError;
use oilcon::analysis::{classify_oil_trend, Row};
use oilcon::render::{format_message, format_record_line};
use oilcon::snapshot::build_snapshot;
use oilcon::store::{load_window, YAHOO_SOURCE};
use price_store::{ensure_schema, upsert};

/// Must match fixtures/drive_python.py.
const PINNED_NOW: &str = "2026-07-30 22:00";
const PINNED_NOW_SECS: &str = "2026-07-30 22:00:00 CST";
const PINNED_TODAY: &str = "2026-07-30";

const SYMBOLS: &[(&str, &str)] = &[("CL=F", "CL_F_rows.json"), ("BZ=F", "BZ_F_rows.json"), ("HO=F", "HO_F_rows.json")];

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
    trend: String,
    #[allow(dead_code)] // captured for diagnostics; warning path not exercised on these sets
    warning: String,
    record: String,
    message: String,
    /// symbol → ordered (day, close) from the SEED dump
    seed: BTreeMap<String, Vec<(String, f64)>>,
}

fn parse_python_output(stdout: &str) -> Vec<PyBlock> {
    let mut blocks = Vec::new();
    let mut set_name = String::new();
    let mut skill = String::new();
    let mut trend = String::new();
    let mut warning = String::new();
    let mut record = String::new();
    let mut message = String::new();
    let mut seed: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
    let mut mode = "";
    let mut cur_sym = String::new();

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("===SET===") {
            if !set_name.is_empty() {
                blocks.push(PyBlock {
                    set_name: set_name.clone(),
                    skill: skill.clone(),
                    trend: trend.clone(),
                    warning: warning.clone(),
                    record: record.clone(),
                    message: message.clone(),
                    seed: seed.clone(),
                });
            }
            set_name = rest.to_string();
            skill.clear();
            trend.clear();
            warning.clear();
            record.clear();
            message.clear();
            seed.clear();
            mode = "";
            cur_sym.clear();
            continue;
        }
        if let Some(rest) = line.strip_prefix("===SKILL===") {
            skill = rest.to_string();
            mode = "";
            continue;
        }
        if let Some(rest) = line.strip_prefix("===TREND===") {
            trend = rest.to_string();
            mode = "";
            continue;
        }
        if let Some(rest) = line.strip_prefix("===WARNING===") {
            warning = rest.to_string();
            mode = "";
            continue;
        }
        if line == "===SEED===" {
            mode = "seed";
            continue;
        }
        if line == "===END_SEED===" {
            mode = "";
            cur_sym.clear();
            continue;
        }
        if let Some(rest) = line.strip_prefix("===SYMBOL===") {
            cur_sym = rest.to_string();
            seed.entry(cur_sym.clone()).or_default();
            mode = "seed";
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
        if line.starts_with("===NOW") || line.starts_with("===TODAY") {
            mode = "";
            continue;
        }
        if mode == "skip" {
            continue;
        }
        if mode == "seed" {
            if cur_sym.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, '\t');
            let day = parts.next().unwrap_or("").to_string();
            let close_s = parts.next().unwrap_or("");
            // Python prints with !r so values may be `60.0` or `70.8`.
            let close: f64 = close_s.parse().unwrap_or_else(|_| {
                panic!("bad seed close in set {set_name} {cur_sym}: {close_s:?}")
            });
            seed.get_mut(&cur_sym).unwrap().push((day, close));
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
            trend,
            warning,
            record,
            message,
            seed,
        });
    }
    blocks
}

fn set_dir(name: &str) -> PathBuf {
    let root = fixtures_root();
    if name == "live" {
        let live = root.join("live");
        if live.join("CL_F_rows.json").is_file() {
            return live;
        }
        return root;
    }
    root.join("synthetic").join(name)
}

fn load_fixture_rows(dir: &Path) -> BTreeMap<String, Vec<Row>> {
    let mut out = BTreeMap::new();
    for &(ticker, fname) in SYMBOLS {
        let path = dir.join(fname);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("read {}: {e}", path.display());
        });
        // Fixtures are JSON arrays of [date, close] pairs.
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
        out.insert(ticker.to_string(), rows);
    }
    out
}

async fn mem() -> libsql::Connection {
    let db = libsql::Builder::new_local(":memory:").build().await.unwrap();
    let c = db.connect().unwrap();
    ensure_schema(&c).await.unwrap();
    c
}

/// Seed through price_store::upsert — the real writer, not a raw SQL dump.
async fn seed_rust(conn: &libsql::Connection, data: &BTreeMap<String, Vec<Row>>) {
    for (ticker, rows) in data {
        for r in rows {
            upsert(conn, ticker, &r.day, r.close, YAHOO_SOURCE)
                .await
                .unwrap();
        }
    }
}

async fn rust_windows(conn: &libsql::Connection) -> BTreeMap<String, Vec<(String, f64)>> {
    let mut out = BTreeMap::new();
    for &(ticker, _) in SYMBOLS {
        let rows = load_window(conn, ticker).await.unwrap();
        out.insert(
            ticker.to_string(),
            rows.into_iter().map(|r| (r.day, r.close)).collect(),
        );
    }
    out
}

async fn rust_artefacts(
    conn: &libsql::Connection,
    data: &BTreeMap<String, Vec<Row>>,
) -> (String, String, String, String, Option<String>) {
    // Pre-seeded adequately → history should not be required. Still provide a
    // fixture-backed fetcher so a coverage-guard re-fetch (short span) stays offline.
    let data_h = data.clone();
    let data_l = data.clone();
    let fetch_history = move |sym: &str| -> Result<Vec<Row>, FetchError> {
        Ok(data_h.get(sym).cloned().unwrap_or_default())
    };
    let fetch_latest = move |sym: &str| -> Result<Option<Row>, FetchError> {
        Ok(data_l.get(sym).and_then(|v| v.last().cloned()))
    };

    let snap = build_snapshot(conn, PINNED_TODAY, &fetch_history, &fetch_latest).await;
    if let Some(ref w) = snap.warning {
        let message = format!("🛢️ OILCON 情報\n⚠ 今天沒有報告可出:{w}\n  沒有數字可讀,不是行情平靜。下次排程會再試。\n更新：{PINNED_NOW}");
        return (
            String::new(),
            message,
            "degraded".into(),
            String::new(),
            Some(w.clone()),
        );
    }
    let (message, skill) = format_message(&snap, PINNED_NOW);
    let record = format_record_line(&snap, PINNED_NOW_SECS).unwrap();
    let trend = snap
        .wti
        .rows
        .as_ref()
        .map(|r| classify_oil_trend(r).to_string())
        .unwrap_or_else(|| "insufficient-history".into());
    (record, message, skill, trend, None)
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

fn rows_equal(a: &[(String, f64)], b: &[(String, f64)]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| {
        x.0 == y.0 && (x.1 == y.1 || (x.1.is_nan() && y.1.is_nan()))
    })
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

#[tokio::test]
async fn differential_vs_python_on_fixtures() {
    let py_stdout = run_python_driver();
    let blocks = parse_python_output(&py_stdout);
    assert!(
        !blocks.is_empty(),
        "no SET blocks from drive_python.py:\n{py_stdout}"
    );

    let mut failures: Vec<String> = Vec::new();
    let mut report = String::new();

    for block in &blocks {
        let dir = set_dir(&block.set_name);
        assert!(
            dir.join("CL_F_rows.json").is_file(),
            "fixture set '{}' missing CL_F_rows.json under {}",
            block.set_name,
            dir.display()
        );

        let data = load_fixture_rows(&dir);
        let conn = mem().await;
        seed_rust(&conn, &data).await;

        // Step 3: row sequences equal on both sides before render.
        let rs_seed = rust_windows(&conn).await;
        for &(ticker, _) in SYMBOLS {
            let py = block.seed.get(ticker).cloned().unwrap_or_default();
            let rs = rs_seed.get(ticker).cloned().unwrap_or_default();
            // Python dump is full window; may be longer than 252 if fixture is —
            // both sides cap at 252 via window/load_window. Compare on that cap.
            let py_cap: Vec<_> = if py.len() > 252 {
                py[py.len() - 252..].to_vec()
            } else {
                py.clone()
            };
            if !rows_equal(&py_cap, &rs) {
                failures.push(format!(
                    "[{}] seed window mismatch for {ticker}: py={} rs={}",
                    block.set_name,
                    py_cap.len(),
                    rs.len()
                ));
                // Show first differing row
                for (i, (p, r)) in py_cap.iter().zip(rs.iter()).enumerate() {
                    if p != r {
                        report.push_str(&format!(
                            "seed {ticker} first diff at {i}: py={p:?} rs={r:?}\n"
                        ));
                        break;
                    }
                }
                if py_cap.len() != rs.len() {
                    report.push_str(&format!(
                        "seed {ticker} len py={} rs={} first_py={:?} first_rs={:?} last_py={:?} last_rs={:?}\n",
                        py_cap.len(),
                        rs.len(),
                        py_cap.first(),
                        rs.first(),
                        py_cap.last(),
                        rs.last()
                    ));
                }
            }
        }

        let (rs_record, rs_message, rs_skill, rs_trend, _warn) =
            rust_artefacts(&conn, &data).await;

        report.push_str(&format!(
            "\n######## set={} trend py={} rs={} skill py={} rs={} ########\n",
            block.set_name, block.trend, rs_trend, block.skill, rs_skill
        ));
        report.push_str(&format!("python record: {}\n", block.record));
        report.push_str(&format!("rust   record: {rs_record}\n"));
        report.push_str(&format!("python skill:  {}\n", block.skill));
        report.push_str(&format!("rust   skill:  {rs_skill}\n"));
        report.push_str(&format!("python trend:  {}\n", block.trend));
        report.push_str(&format!("rust   trend:  {rs_trend}\n"));

        if block.skill != rs_skill {
            failures.push(format!(
                "[{}] skill status: python={:?} rust={:?}",
                block.set_name, block.skill, rs_skill
            ));
            report.push_str("SKILL: DIFFER\n");
        } else {
            report.push_str("SKILL: BYTE-IDENTICAL\n");
        }

        if block.trend != rs_trend {
            failures.push(format!(
                "[{}] trend: python={:?} rust={:?}",
                block.set_name, block.trend, rs_trend
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

    // Always print the full comparison so Task 6's report has verbatim sides.
    eprintln!("{report}");

    if !failures.is_empty() {
        panic!(
            "differential mismatches (report every difference; do not fix in Task 6):\n{}",
            failures.join("\n")
        );
    }
}
