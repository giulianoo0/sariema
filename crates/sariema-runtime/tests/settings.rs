// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeSet;

use sariema_runtime::settings::{DOCS, Settings};
use serde_json::Value;

fn leaves(prefix: &str, v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                leaves(&path, v, out);
            }
        }
        _ => {
            out.insert(prefix.to_string());
        }
    }
}

#[test]
fn every_setting_is_documented_and_every_doc_is_a_setting() {
    let mut keys = BTreeSet::new();
    leaves(
        "",
        &serde_json::to_value(Settings::default()).unwrap(),
        &mut keys,
    );
    let docs: BTreeSet<String> = DOCS.iter().map(|(k, _)| k.to_string()).collect();
    assert_eq!(keys, docs);
}

#[test]
fn settings_round_trip_and_refuse_unknown_keys() {
    let json = serde_json::to_string(&Settings::default()).unwrap();
    assert_eq!(
        serde_json::from_str::<Settings>(&json).unwrap(),
        Settings::default()
    );
    let partial: Settings = serde_json::from_str(r#"{"motion":{"turn_rate":90}}"#).unwrap();
    assert_eq!(partial.motion.turn_rate, 90.0);
    assert_eq!(partial.max_entities, Settings::default().max_entities);
    assert!(serde_json::from_str::<Settings>(r#"{"motoin":{}}"#).is_err());
    assert!(serde_json::from_str::<Settings>(r#"{"cutscene":{"end_ease":"wobbly"}}"#).is_err());
}

#[test]
fn time_scale_bounds_must_be_positive_and_in_order() {
    use sariema_format::script::GamePack;
    use sariema_runtime::game::pack_settings;
    let pack = |settings: &str| GamePack {
        name: "t".into(),
        batch_version: 0,
        wasm: Vec::new(),
        cutscenes: Vec::new(),
        names: Vec::new(),
        settings: settings.into(),
    };
    for bad in [
        r#"{ "min_time_scale": 2, "max_time_scale": 1 }"#,
        r#"{ "min_time_scale": 0 }"#,
        r#"{ "max_time_scale": -1 }"#,
    ] {
        let e = pack_settings(&pack(bad)).expect_err(bad);
        assert_eq!(e.code, "E_SETTINGS", "{bad}");
        assert!(e.message.contains("time_scale"), "{}", e.message);
    }
    assert!(pack_settings(&pack(r#"{ "min_time_scale": 1, "max_time_scale": 1 }"#)).is_ok());
}
