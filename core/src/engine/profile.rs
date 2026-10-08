//! profiles.json model + parser.
//!
//! Responsibility: parse + shape validation of the profile pack.
//! Non-goals: value validation (`validate.rs`), plan building (`plan.rs`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub desc: String,
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilesFile {
    pub schema: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub rom: String,
    pub profiles: Vec<Profile>,
}

pub fn parse_profiles(json: &str) -> Result<ProfilesFile, String> {
    let file: ProfilesFile =
        serde_json::from_str(json).map_err(|e| format!("profiles.json: {e}"))?;
    if file.schema != 1 {
        return Err(format!("unsupported profiles schema {}", file.schema));
    }
    if file.profiles.is_empty() {
        return Err("profiles.json has no profiles".into());
    }
    let mut ids = std::collections::HashSet::new();
    for p in &file.profiles {
        if p.id.is_empty() || p.params.is_empty() {
            return Err(format!("profile '{}' is empty", p.id));
        }
        if !ids.insert(p.id.as_str()) {
            return Err(format!("duplicate profile id '{}'", p.id));
        }
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::catalog;

    #[test]
    fn embedded_profiles_parse_and_validate_shape() {
        let json = include_str!("../../profiles.json");
        let file = parse_profiles(json).expect("bundled profiles.json must parse");
        assert_eq!(file.profiles.len(), 5);
        let ids: Vec<&str> = file.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["powersave", "balance", "game", "sleep", "boost"]);
        // every key of every profile must exist in the catalog
        for prof in &file.profiles {
            for key in prof.params.keys() {
                assert!(
                    catalog::find(key).is_some(),
                    "{}: unknown key {key}",
                    prof.id
                );
            }
        }
    }

    #[test]
    fn embedded_profiles_do_not_fight_the_network_stack() {
        // ConnectivityService + netd rewrite tcp buffers from the carrier's
        // LinkProperties.TcpBufferSizes on network re-apply (observed across
        // display-off cycles, 2026-10-07). The network stack owns these nodes;
        // profiles must not ship values for them.
        let json = include_str!("../../profiles.json");
        let file = parse_profiles(json).expect("parse");
        for p in &file.profiles {
            for k in ["net.tcp_rmem", "net.tcp_wmem"] {
                assert!(
                    !p.params.contains_key(k),
                    "profile {} must not tune {k} (ConnectivityService+netd own it)",
                    p.id
                );
            }
        }
    }
}
