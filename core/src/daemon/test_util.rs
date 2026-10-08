//! Shared daemon test fixtures (cfg(test) only).

use std::collections::BTreeMap;

/// `[("pkg", "profile")]` -> owned map.
pub fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// Fresh temp dir per test (process-id tagged, removed on entry).
pub fn tmpdir(tag: &str) -> std::path::PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("mifinetune-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
