//! End-to-end replay of public RPC responses against the official Pump SDK oracle.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use sol_parser_sdk::{
    accounts::{parse_account_unified, parse_nonce_account, AccountData},
    core::events::EventMetadata,
    parse_rpc_transaction, DexEvent,
};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

fn corpus() -> Value {
    serde_json::from_str(include_str!("../validation/fixtures/mainnet-2026-09-25.json")).unwrap()
}
fn oracle() -> Value {
    serde_json::from_str(include_str!("../validation/fixtures/mainnet-2026-09-25.expected.json"))
        .unwrap()
}
fn assert_fields(actual: &Value, fields: &Value, context: &str) {
    for (key, expected) in fields.as_object().unwrap() {
        let actual = &actual[key];
        if actual.is_number() && expected.is_string() {
            assert_eq!(actual.to_string(), expected.as_str().unwrap(), "{context}: {key}");
        } else {
            assert_eq!(actual, expected, "{context}: {key}");
        }
    }
}
#[test]
fn recent_mainnet_swaps_match_official_sdk_events() {
    let c = corpus();
    let e = oracle();
    assert_eq!(c["transactions"].as_array().unwrap().len(), 10);
    for (tx, expected) in
        c["transactions"].as_array().unwrap().iter().zip(e["transactions"].as_array().unwrap())
    {
        assert_eq!(tx["signature"], expected["signature"]);
        let rpc_tx = serde_json::from_value(tx["encoded"].clone()).unwrap();
        let events: Vec<Value> = parse_rpc_transaction(&rpc_tx, None)
            .unwrap()
            .into_iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        for wanted in expected["events"].as_array().unwrap() {
            let kind = wanted["kind"].as_str().unwrap();
            let candidates: Vec<_> = events.iter().filter_map(|event| event.get(kind)).collect();
            let matched = candidates.into_iter().find(|event| {
                wanted["fields"].as_object().unwrap().iter().all(|(k, v)| {
                    let actual = &event[k];
                    actual == v
                        || actual.is_number() && v.as_str() == Some(actual.to_string().as_str())
                })
            });
            assert!(
                matched.is_some(),
                "{}: {kind} did not match oracle; wanted {wanted}; actual {events:?}",
                tx["signature"]
            );
            assert_eq!(matched.unwrap()["metadata"]["slot"], tx["encoded"]["slot"]);
        }
    }
}
fn account_fixture(value: &Value) -> AccountData {
    AccountData {
        pubkey: Pubkey::from_str(value["address"].as_str().unwrap()).unwrap(),
        owner: Pubkey::from_str(value["value"]["owner"].as_str().unwrap()).unwrap(),
        executable: false,
        lamports: value["value"]["lamports"].as_u64().unwrap(),
        rent_epoch: 0,
        data: STANDARD.decode(value["value"]["data"][0].as_str().unwrap()).unwrap(),
    }
}
#[test]
fn streamed_account_layouts_match_official_sdk_and_reject_partial_fields() {
    let c = corpus();
    let e = oracle();
    for wanted in e["accounts"].as_array().unwrap() {
        let fixture = c["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["address"] == wanted["address"])
            .unwrap();
        let mut account = account_fixture(fixture);
        account.data = STANDARD.decode(wanted["data"].as_str().unwrap()).unwrap();
        let event = parse_account_unified(&account, EventMetadata::default(), None)
            .expect("complete historical or padded current account");
        let actual = serde_json::to_value(event).unwrap();
        let fields = if wanted["kind"] == "curve" {
            &actual["PumpFunBondingCurveAccount"]["bonding_curve"]
        } else {
            &actual["PumpSwapPoolAccount"]["pool"]
        };
        assert_fields(
            fields,
            &wanted["fields"],
            &format!("{} {} bytes", wanted["kind"], wanted["length"]),
        );
    }
    for fixture in c["accounts"].as_array().unwrap() {
        let mut account = account_fixture(fixture);
        let original = account.data.clone();
        let valid: Vec<usize> = if fixture["kind"] == "curve" {
            vec![49, 81, 82, 83, 115, 123, 124]
        } else {
            vec![211, 243, 244, 245, 261, 269, 270]
        };
        let current = if fixture["kind"] == "curve" { 125 } else { 271 };
        for len in 8..current {
            if valid.contains(&len) {
                continue;
            }
            account.data = original[..len].to_vec();
            if fixture["kind"] == "pool" && len == 252 {
                account.data[245] = 1;
            }
            assert!(
                parse_account_unified(&account, EventMetadata::default(), None).is_none(),
                "unexpected partial {} length {len}",
                fixture["kind"]
            );
        }
        account.data = original;
        account.owner = Pubkey::new_unique();
        assert!(parse_account_unified(&account, EventMetadata::default(), None).is_none());
    }
}
#[test]
fn nonce_dispatch_requires_system_owned_initialized_state() {
    let mut a = AccountData {
        pubkey: Pubkey::new_unique(),
        owner: Pubkey::default(),
        executable: false,
        lamports: 1,
        rent_epoch: 0,
        data: vec![0; 80],
    };
    a.data[..8].copy_from_slice(&[1, 0, 0, 0, 1, 0, 0, 0]);
    assert!(matches!(
        parse_account_unified(&a, EventMetadata::default(), None),
        Some(DexEvent::NonceAccount(_))
    ));
    for mutation in 0..5 {
        let mut bad = a.clone();
        match mutation {
            0 => bad.owner = Pubkey::new_unique(),
            1 => bad.data[4] = 0,
            2 => bad.data[0] = 0,
            3 => bad.data.push(0),
            _ => bad.executable = true,
        };
        assert!(parse_nonce_account(&bad, EventMetadata::default()).is_none());
        assert!(!matches!(
            parse_account_unified(&bad, EventMetadata::default(), None),
            Some(DexEvent::NonceAccount(_))
        ));
    }
}

#[test]
fn malformed_trade_name_is_rejected_through_transaction_event_decoder() {
    let c = corpus();
    let tx = c["transactions"].as_array().unwrap().iter().find(|t| t["venue"] == "pump").unwrap();
    let log = tx["json"]["meta"]["logMessages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|s| {
            s.strip_prefix("Program data: ")
                .and_then(|v| STANDARD.decode(v).ok())
                .is_some_and(|v| v.starts_with(&[189, 219, 127, 211, 78, 230, 97, 238]))
        })
        .unwrap();
    let raw = STANDARD.decode(log.strip_prefix("Program data: ").unwrap()).unwrap();
    let mut body = raw[8..].to_vec();
    assert!(sol_parser_sdk::logs::pump::parse_trade_from_data(
        &body,
        EventMetadata::default(),
        false
    )
    .is_some());
    // TradeEvent fields through last_update_timestamp occupy 250 bytes.
    let name_len = u32::from_le_bytes(body[250..254].try_into().unwrap()) as usize;
    assert!(name_len > 0);
    body[254] = 0xff;
    assert!(sol_parser_sdk::logs::pump::parse_trade_from_data(
        &body,
        EventMetadata::default(),
        false
    )
    .is_none());
    body[250..254].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(sol_parser_sdk::logs::pump::parse_trade_from_data(
        &body,
        EventMetadata::default(),
        false
    )
    .is_none());
}
