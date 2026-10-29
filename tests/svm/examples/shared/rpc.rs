#![allow(dead_code)]
//! JSON-RPC client shared by the validator examples (`bench_run`,
//! `rebalancer`): blocking, with retries for transport errors and for
//! "block not available yet" (-32004).

use base64::Engine;
use serde_json::{json, Value};
use solana_message::Hash;
use std::str::FromStr;
use std::time::Duration;

use super::common::Pubkey;

pub struct Rpc {
    pub url: String,
    pub agent: ureq::Agent,
}

impl Rpc {
    pub fn new(url: String) -> Self {
        Self {
            url,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .build(),
        }
    }

    /// Retries transport errors and "block not available yet" (-32004).
    pub fn call(&self, method: &str, params: Value) -> Value {
        self.try_call(method, params)
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    /// Like [`Rpc::call`], but returns a JSON-RPC error instead of
    /// panicking on it.
    pub fn try_call(&self, method: &str, params: Value) -> Result<Value, Value> {
        const ATTEMPTS: usize = 40;
        for attempt in 1..=ATTEMPTS {
            let r = self.agent.post(&self.url).send_json(json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params
            }));
            let retry = match r {
                Ok(resp) => {
                    let v: Value = resp.into_json().unwrap();
                    match v.get("error") {
                        None => return Ok(v["result"].clone()),
                        Some(e) if e["code"] == -32004 => e.clone(),
                        Some(e) => return Err(e.clone()),
                    }
                }
                Err(e) => json!(format!("{e}")),
            };
            if attempt == ATTEMPTS {
                return Err(retry);
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        unreachable!()
    }

    pub fn blockhash(&self) -> Hash {
        self.blockhash_valid_until().0
    }

    /// The latest blockhash and the last block height it can land at.
    pub fn blockhash_valid_until(&self) -> (Hash, u64) {
        let r = self.call("getLatestBlockhash", json!([{"commitment": "confirmed"}]));
        (
            Hash::from_str(r["value"]["blockhash"].as_str().unwrap()).unwrap(),
            r["value"]["lastValidBlockHeight"].as_u64().unwrap(),
        )
    }

    pub fn slot(&self) -> u64 {
        self.call("getSlot", json!([{"commitment": "processed"}]))
            .as_u64()
            .unwrap()
    }

    pub fn block_height(&self) -> u64 {
        self.call("getBlockHeight", json!([{"commitment": "confirmed"}]))
            .as_u64()
            .unwrap()
    }

    /// Reads `keys` in one request, so every account is from the same slot
    /// (returned first). A missing account is `None`.
    pub fn fetch_at(&self, keys: &[Pubkey]) -> (u64, Vec<Option<Vec<u8>>>) {
        let r = self.call(
            "getMultipleAccounts",
            json!([keys.iter().map(|k| k.to_string()).collect::<Vec<_>>(), {"encoding": "base64", "commitment": "confirmed"}]),
        );
        let data = r["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (!a.is_null()).then(|| {
                    base64::engine::general_purpose::STANDARD
                        .decode(a["data"][0].as_str().unwrap())
                        .unwrap()
                })
            })
            .collect();
        (r["context"]["slot"].as_u64().unwrap(), data)
    }

    /// Reads `keys`, all of which must exist.
    pub fn fetch(&self, keys: &[Pubkey]) -> Vec<Vec<u8>> {
        self.fetch_at(keys)
            .1
            .into_iter()
            .zip(keys)
            .map(|(d, k)| d.unwrap_or_else(|| panic!("{k} missing")))
            .collect()
    }
}
