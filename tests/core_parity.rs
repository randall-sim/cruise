use cc_daemon::{
    dispatch, mcp,
    store::{Store, id},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs};

fn replace(value: &Value, ids: &BTreeMap<String, String>, normalize: bool) -> Value {
    match value {
        Value::String(text) => {
            let mut text = text.clone();
            for (from, to) in ids {
                text = text.replace(from, to);
            }
            if normalize
                && text.len() >= 20
                && text.as_bytes().get(10) == Some(&b'T')
                && text.ends_with('Z')
            {
                text = "NOW".into();
            }
            json!(text)
        }
        Value::Array(values) => json!(
            values
                .iter()
                .map(|value| replace(value, ids, normalize))
                .collect::<Vec<_>>()
        ),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), replace(value, ids, normalize)))
                .collect(),
        ),
        Value::Number(number) if normalize => json!(number.as_f64().unwrap()),
        _ => value.clone(),
    }
}

#[test]
fn course_and_lecture_mcp_contract_matches_previous_engine() {
    // Recorded once from the previous engine against a throwaway workspace.
    // The regression test itself needs only Rust and never starts Node.
    let cases: Value = serde_json::from_str(include_str!("fixtures/core_parity.json")).unwrap();
    let root = std::env::temp_dir().join(format!("cruise-parity-{}", id()));
    let store = Store::new(root.clone());
    let mut ids = BTreeMap::new();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let input = replace(&case["input"], &ids, false);
        let output = dispatch(&store, name, mcp::validate_input(name, input).unwrap()).unwrap();
        if let Some(placeholder) = match name {
            "create_course" => Some("COURSE"),
            "import_lecture" => Some("LECTURE"),
            "queue_study_job" => Some("JOB"),
            _ => None,
        } {
            ids.insert(placeholder.into(), output["id"].as_str().unwrap().into());
        }
        let inverse = ids
            .iter()
            .map(|(placeholder, id)| (id.clone(), placeholder.clone()))
            .collect();
        assert_eq!(
            replace(&output, &inverse, true),
            replace(&case["result"], &BTreeMap::new(), true),
            "MCP return contract: {name}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}
