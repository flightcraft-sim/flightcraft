#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
#[cfg(test)]
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/region_data.rs"));
use flightcraft_control::Session;
use serde_json::json;

#[test]
fn commands_have_schemas_and_validate_atomically_without_opening_network() {
    let mut session = Session::new().unwrap();
    let status = session.execute("efb.status", json!({})).unwrap();
    assert_eq!(status["available"], false);
    assert_eq!(status["settings"]["enabled"], false);
    assert!(
        session
            .execute(
                "efb.broadcast",
                json!({"enabled":true,"destination":"127.0.0.1","gdl90":true,"simulator":false})
            )
            .is_err()
    );
    session.initialize_efb(123456).unwrap();
    let before = session.execute("efb.status", json!({})).unwrap();
    for input in [
        json!({"enabled":true,"destination":"example.com","gdl90":true,"simulator":false}),
        json!({"enabled":true,"destination":"0.0.0.0","gdl90":true,"simulator":false}),
        json!({"enabled":true,"destination":"224.0.0.1","gdl90":true,"simulator":false}),
        json!({"enabled":true,"destination":"127.0.0.1:4000","gdl90":true,"simulator":false}),
        json!({"enabled":true,"destination":"127.0.0.1","gdl90":false,"simulator":false}),
        json!({"enabled":true,"destination":null,"gdl90":true,"simulator":false}),
        json!({"enabled":true,"destination":"127.0.0.1","gdl90":true,"simulator":false,"extra":1}),
    ] {
        assert!(session.execute("efb.broadcast", input).is_err());
        assert_eq!(
            session.execute("efb.status", json!({})).unwrap()["settings"],
            before["settings"]
        );
    }
    let input = json!({"enabled":true,"destination":"127.0.0.1","gdl90":true,"simulator":true});
    let result = session.execute("efb.broadcast", input.clone()).unwrap();
    assert!(result["reason"].as_str().unwrap().contains("geodetic"));
    assert_eq!(result["gdl90_packets"], 0);
    assert_eq!(result["simulator_packets"], 0);
    let registry = Session::commands();
    let command = registry.iter().find(|c| c.name == "efb.broadcast").unwrap();
    assert!(
        jsonschema::validator_for(&command.input_schema)
            .unwrap()
            .is_valid(&input)
    );
    assert!(
        jsonschema::validator_for(&command.output_schema)
            .unwrap()
            .is_valid(&result)
    );
}

#[test]
fn region_can_be_selected_while_broadcast_is_disabled() {
    require_region_data!();
    let mut session = Session::new().unwrap();
    session.initialize_efb(42).unwrap();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/terrain/new-york");
    session
        .install_region(flightcraft_control::read_region(&directory).unwrap())
        .unwrap();
    session
        .execute(
            "sim.reset",
            json!({"world":"new-york","start":{"runway":{"airport":"1N7","end":"07"}}}),
        )
        .unwrap();
    // No socket is opened while off; geodesy can become available independently.
    session.update_efb();
    let status = session.execute("efb.status", json!({})).unwrap();
    assert_eq!(status["reason"], "disabled");
    assert_eq!(status["gdl90_packets"], 0);
}
