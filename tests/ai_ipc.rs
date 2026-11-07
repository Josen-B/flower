//! No installation, model, user dictionary, host text or global profile access.
use flower::ai::ipc::AiClient;
use flower::ai::{Request, Response, MAX_FRAME_BYTES};
use std::path::PathBuf;

#[test]
fn helper_selection_rejects_relative_and_non_settings_images() {
    assert!(AiClient::start(PathBuf::from("flower-settings.exe")).is_err());
    assert!(AiClient::start(std::env::current_exe().unwrap()).is_err());
}

#[test]
fn wire_contract_is_bounded_and_rejects_unknown_fields() {
    let request = Request {
        raw: String::new(),
        reading: String::new(),
        id: u64::MAX,
        text: "一".repeat(64),
    };
    let bytes = serde_json::to_vec(&request).unwrap();
    assert!(bytes.len() <= MAX_FRAME_BYTES);
    let parsed: Request = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.id, request.id);
    assert_eq!(parsed.text, request.text);
    assert!(
        serde_json::from_str::<Request>(r#"{"id":1,"text":"今天心情很好","unexpected":true}"#)
            .is_err()
    );
    assert!(
        serde_json::from_str::<Response>(r#"{"id":1,"corrected":null,"unexpected":true}"#).is_err()
    );
}

#[test]
fn spelling_metadata_is_bounded_and_old_frames_remain_text_only() {
    let request = Request {
        id: 9,
        text: "一".repeat(64),
        raw: "a".repeat(128),
        reading: std::iter::repeat("xiang")
            .take(64)
            .collect::<Vec<_>>()
            .join("'"),
    };
    assert!(flower::ai::eligible_request(&request));
    let bytes = serde_json::to_vec(&request).unwrap();
    assert!(bytes.len() <= MAX_FRAME_BYTES);
    let parsed: Request = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.raw, request.raw);
    assert_eq!(parsed.reading, request.reading);
    let legacy: Request = serde_json::from_str(r#"{"id":1,"text":"今天心情很好"}"#).unwrap();
    assert!(legacy.raw.is_empty() && legacy.reading.is_empty());
    let response: Response = serde_json::from_str(r#"{"id":1,"corrected":null}"#).unwrap();
    assert!(response.reading.is_empty());
    assert!(flower::ai::valid_response(&legacy, &response));
}
