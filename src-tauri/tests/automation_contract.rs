use agent_office_lib::types::{AutomationDefinition, AutomationRunRecord, AutomationStatus};
use serde::{de::DeserializeOwned, Serialize};

fn roundtrip<T: DeserializeOwned + Serialize>(json: &str) {
    let parsed: T = serde_json::from_str(json).expect("TypeScript wire contract must deserialize");
    let actual = serde_json::to_value(parsed).unwrap();
    let expected: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(
        actual, expected,
        "wire casing and omitted optional fields must be preserved"
    );
}

#[test]
fn all_five_steps_use_the_typescript_wire_contract() {
    roundtrip::<AutomationDefinition>(include_str!(
        "../../src/shared/contract-fixtures/automation-definition.json"
    ));
}

#[test]
fn optional_step_fields_are_omitted_instead_of_null() {
    let json = r#"{"schemaVersion":1,"id":"minimal","revision":1,"name":"minimal","inputs":[],"steps":[{"kind":"launchCli","id":"launch","cliProfileId":"codex"},{"kind":"llmTask","id":"task","label":"task","promptTemplate":"hello"}]}"#;
    roundtrip::<AutomationDefinition>(json);
}

#[test]
fn restored_run_and_decision_status_roundtrip() {
    roundtrip::<AutomationRunRecord>(include_str!(
        "../../src/shared/contract-fixtures/automation-run-record.json"
    ));
    roundtrip::<AutomationStatus>(include_str!(
        "../../src/shared/contract-fixtures/automation-status.json"
    ));
}

#[test]
fn v2_definition_and_cli_return_status_roundtrip() {
    roundtrip::<AutomationDefinition>(include_str!("../../src/shared/contract-fixtures/automation-definition.v2.json"));
    roundtrip::<AutomationStatus>(include_str!("../../src/shared/contract-fixtures/automation-status.v2.json"));
}
