use claw_core::env::load_env;
use std::io::Write;
use std::path::PathBuf;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn write(body: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("claw-core-env-{}-{}.env", std::process::id(), n));
    std::fs::File::create(&p).unwrap().write_all(body.as_bytes()).unwrap();
    p
}

#[test]
fn sets_keys_from_file() {
    let _g = guard();
    std::env::remove_var("CLAW_T_A");
    load_env(Some(&write("CLAW_T_A=hello\n")));
    assert_eq!(std::env::var("CLAW_T_A").unwrap(), "hello");
    std::env::remove_var("CLAW_T_A");
}

#[test]
fn never_overrides_an_existing_variable() {
    // B12. A value already in the environment (set by cron) must win.
    let _g = guard();
    std::env::set_var("CLAW_T_B", "from-cron");
    load_env(Some(&write("CLAW_T_B=from-file\n")));
    assert_eq!(std::env::var("CLAW_T_B").unwrap(), "from-cron");
    std::env::remove_var("CLAW_T_B");
}

#[test]
fn strips_quote_characters_successively_not_as_pairs() {
    // B11. Verified against Python: '"value' -> 'value' (UNPAIRED),
    // and '"\'value\'"' -> 'value' (both layers).
    let _g = guard();
    for (raw, want) in [
        ("CLAW_T_C=\"value\"", "value"),
        ("CLAW_T_C='value'", "value"),
        ("CLAW_T_C=\"value", "value"),
        ("CLAW_T_C=value\"", "value"),
        ("CLAW_T_C=\"'value'\"", "value"),
        ("CLAW_T_C=va\"lue", "va\"lue"),
    ] {
        std::env::remove_var("CLAW_T_C");
        load_env(Some(&write(&format!("{raw}\n"))));
        assert_eq!(std::env::var("CLAW_T_C").unwrap(), want, "input was {raw}");
    }
    std::env::remove_var("CLAW_T_C");
}

#[test]
fn skips_blank_comment_and_keyless_lines() {
    let _g = guard();
    std::env::remove_var("CLAW_T_D");
    load_env(Some(&write("\n   \n# CLAW_T_D=commented\nnoequalshere\nCLAW_T_D=real\n")));
    assert_eq!(std::env::var("CLAW_T_D").unwrap(), "real");
    std::env::remove_var("CLAW_T_D");
}

#[test]
fn splits_on_the_first_equals_only() {
    let _g = guard();
    std::env::remove_var("CLAW_T_E");
    load_env(Some(&write("CLAW_T_E=a=b=c\n")));
    assert_eq!(std::env::var("CLAW_T_E").unwrap(), "a=b=c");
    std::env::remove_var("CLAW_T_E");
}

#[test]
fn missing_file_is_a_silent_noop() {
    let _g = guard();
    load_env(Some(&PathBuf::from("/nonexistent/claw-core/none.env")));
}

#[test]
fn claw_env_variable_selects_the_path() {
    let _g = guard();
    let p = write("CLAW_T_F=via-env\n");
    std::env::set_var("CLAW_ENV", &p);
    std::env::remove_var("CLAW_T_F");
    load_env(None);
    assert_eq!(std::env::var("CLAW_T_F").unwrap(), "via-env");
    std::env::remove_var("CLAW_ENV");
    std::env::remove_var("CLAW_T_F");
}
