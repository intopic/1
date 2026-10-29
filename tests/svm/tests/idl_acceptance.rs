//! Audit F-01: the published IDL must describe what the program emits.
//!
//! Loads `target/idl/sharded_amm.json` (or `AMM_IDL`), checks the address,
//! every event and error against the program crate, and decodes real
//! `Program data:` logs from the deployed SBF program using only the IDL's
//! discriminators and field types. `scripts/build-linux.sh` sets
//! `AMM_REQUIRE_IDL=1`; elsewhere the test is skipped if no IDL was built.

mod common;

use base64::Engine;
use common::*;
use serde_json::Value;
use solana_signer::Signer;

fn load_idl() -> Option<Value> {
    let path = std::env::var("AMM_IDL").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/idl/sharded_amm.json"
        )
        .to_string()
    });
    match std::fs::read_to_string(&path) {
        Ok(s) => Some(serde_json::from_str(&s).expect("IDL is JSON")),
        Err(_) if std::env::var("AMM_REQUIRE_IDL").is_err() => {
            println!("no IDL at {path}; skipped");
            None
        }
        Err(e) => panic!("IDL required at {path}: {e}"),
    }
}

/// Decodes one Borsh value of an IDL type into JSON.
fn decode(idl: &Value, ty: &Value, data: &[u8], at: &mut usize) -> Value {
    let mut take = |n: usize| {
        let s = &data[*at..*at + n];
        *at += n;
        s.to_vec()
    };
    match ty.as_str() {
        Some("bool") => Value::Bool(take(1)[0] != 0),
        Some("u8") => take(1)[0].into(),
        Some("u16") => u16::from_le_bytes(take(2).try_into().unwrap()).into(),
        Some("u32") => u32::from_le_bytes(take(4).try_into().unwrap()).into(),
        Some("i32") => i32::from_le_bytes(take(4).try_into().unwrap()).into(),
        Some("u64") => u64::from_le_bytes(take(8).try_into().unwrap()).into(),
        Some("i64") => i64::from_le_bytes(take(8).try_into().unwrap()).into(),
        Some("u128") => u128::from_le_bytes(take(16).try_into().unwrap())
            .to_string()
            .into(),
        Some("i128") => i128::from_le_bytes(take(16).try_into().unwrap())
            .to_string()
            .into(),
        Some("pubkey") => Pubkey::new_from_array(take(32).try_into().unwrap())
            .to_string()
            .into(),
        Some(other) => panic!("unsupported IDL type {other}"),
        None => {
            if let Some(name) = ty.pointer("/defined/name").and_then(Value::as_str) {
                let def = idl["types"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|t| t["name"] == name)
                    .unwrap_or_else(|| panic!("type {name} missing from IDL"));
                decode_def(idl, def, data, at)
            } else if let Some(arr) = ty.get("array") {
                let n = arr[1].as_u64().unwrap() as usize;
                Value::Array((0..n).map(|_| decode(idl, &arr[0], data, at)).collect())
            } else {
                panic!("unsupported IDL type {ty}")
            }
        }
    }
}

fn decode_def(idl: &Value, def: &Value, data: &[u8], at: &mut usize) -> Value {
    match def.pointer("/type/kind").and_then(Value::as_str) {
        Some("struct") => {
            let mut obj = serde_json::Map::new();
            for f in def.pointer("/type/fields").unwrap().as_array().unwrap() {
                obj.insert(
                    f["name"].as_str().unwrap().into(),
                    decode(idl, &f["type"], data, at),
                );
            }
            Value::Object(obj)
        }
        Some("enum") => {
            let i = data[*at] as usize;
            *at += 1;
            def.pointer("/type/variants").unwrap()[i]["name"].clone()
        }
        k => panic!("unsupported kind {k:?}"),
    }
}

/// Every `Program data:` log of the last transaction, decoded via the IDL.
fn events(idl: &Value, logs: &[String]) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for l in logs {
        let Some(b64) = l.strip_prefix("Program data: ") else {
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .unwrap();
        let ev = idl["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| {
                let d: Vec<u8> = e["discriminator"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| b.as_u64().unwrap() as u8)
                    .collect();
                bytes.starts_with(&d)
            })
            .unwrap_or_else(|| panic!("event with unknown discriminator {:?}", &bytes[..8]));
        let name = ev["name"].as_str().unwrap();
        let def = idl["types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        let mut at = 8;
        let v = decode_def(idl, def, &bytes, &mut at);
        assert_eq!(
            at,
            bytes.len(),
            "{name}: IDL layout must consume every byte"
        );
        out.push((name.to_string(), v));
    }
    out
}

/// `#[event]` structs and `AmmError` variants, read from the program source.
fn source_counts() -> (usize, usize) {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../programs/sharded-amm/src"
    );
    let events = std::fs::read_to_string(format!("{dir}/events.rs")).unwrap();
    let errors = std::fs::read_to_string(format!("{dir}/errors.rs")).unwrap();
    let n_events = events.matches("#[event]").count();
    let body = &errors[errors.find("pub enum AmmError").unwrap()..];
    let body = &body[body.find('{').unwrap() + 1..body.find("\n}").unwrap()];
    let n_errors = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('#'))
        .count();
    (n_events, n_errors)
}

#[test]
fn idl_is_complete_and_decodes_real_events() {
    let Some(idl) = load_idl() else { return };
    assert_eq!(idl["address"], sharded_amm::ID.to_string());
    let (n_events, n_errors) = source_counts();
    assert_eq!(idl["events"].as_array().unwrap().len(), n_events, "events");
    assert_eq!(idl["errors"].as_array().unwrap().len(), n_errors, "errors");
    assert!(n_events >= 13 && n_errors >= 54);
    // Error codes are dense from 6000 and match the program's enum.
    for (i, e) in idl["errors"].as_array().unwrap().iter().enumerate() {
        assert_eq!(e["code"], 6000 + i as u64, "{e}");
    }
    let code_of = |name: &str| {
        idl["errors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == name)
            .unwrap()["code"]
            .as_u64()
            .unwrap() as u32
    };
    // F-02: the IDL publishes the same position seeds as the ABI doc.
    let open = idl["instructions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "open_position")
        .unwrap();
    let pos = open["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "position")
        .unwrap();
    assert_eq!(
        pos["pda"]["seeds"],
        serde_json::json!([
            {"kind": "const", "value": b"lp_pos".to_vec()},
            {"kind": "account", "path": "shard"},
            {"kind": "account", "path": "owner"},
            {"kind": "arg", "path": "tick_lower"},
            {"kind": "arg", "path": "tick_upper"},
        ])
    );
    use sharded_amm::errors::AmmError;
    for (name, e) in [
        ("SlippageExceeded", AmmError::SlippageExceeded),
        ("ZeroLiquidityGap", AmmError::ZeroLiquidityGap),
        ("Unauthorized", AmmError::Unauthorized),
        ("InvalidAccountCount", AmmError::InvalidAccountCount),
    ] {
        assert_eq!(code_of(name), amm_code(e));
    }

    // Real logs from the SBF program, decoded with nothing but the IDL.
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let t = env.user(&m, 1_000_000_000);
    let arrays = env.swap_arrays(&m, 0, true);
    let i = env.ix_swap(&m, &t, 0, env.swap_args(true, 1_000_000), &arrays);
    let msg = solana_message::Message::new(&[i], Some(&t.kp.pubkey()));
    let tx = solana_transaction::Transaction::new(&[&t.kp], msg, env.svm.latest_blockhash());
    let meta = env.svm.send_transaction(tx).expect("swap");
    let evs = events(&idl, &meta.logs);
    assert_eq!(evs.len(), 1);
    let (name, v) = &evs[0];
    assert_eq!(name, "SwapExecuted");
    let st = env.shard_state(&m, 0);
    assert_eq!(v["shard"], shard_addrs(&m, 0, 0).shard.to_string());
    assert_eq!(v["user"], t.kp.pubkey().to_string());
    assert_eq!(v["a_to_b"], true);
    assert_eq!(v["amount_in"], 1_000_000u64);
    assert_eq!(v["sqrt_price_x64"], st.sqrt_price_x64.to_string());
    assert_eq!(v["tick"], st.tick_current);
    assert_eq!(
        v["amount_out"].as_u64().unwrap(),
        env.balance(&t.token_b) - 1_000_000_000
    );
}
