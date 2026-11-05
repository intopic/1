//! Kontroll i hershëm on-chain i nullifier-it, para provës së shtrenjtë.
//!
//! Nëse nullifier-i (llogaria burimore + aplikacioni) është regjistruar tashmë nga
//! një portofol tjetër, programi do ta refuzonte provën me `NullifierAlreadyUsed`.
//! Daemon-i e llogarit nullifier-in në milisekonda (Poseidon) dhe lexon PDA-në
//! `["nullifier", nullifier]` me RPC, ndaj nuk shpenzon ~8 sekonda-CPU për të.
//!
//! Programi on-chain mbetet autoriteti: nëse RPC-ja nuk përgjigjet, prova bëhet
//! normalisht (fail-open) dhe programi vendos.

use anyhow::{bail, Context, Result};
use base64::Engine;
use curve25519_dalek::edwards::CompressedEdwardsY;
use sha2::{Digest, Sha256};

/// `Pubkey::find_program_address` i Solana-s (pa SDK-në e Solana-s).
pub fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> ([u8; 32], u8) {
    for bump in (0..=255u8).rev() {
        let mut h = Sha256::new();
        for s in seeds {
            h.update(s);
        }
        h.update([bump]);
        h.update(program_id);
        h.update(b"ProgramDerivedAddress");
        let addr: [u8; 32] = h.finalize().into();
        // PDA e vlefshme = jashtë kurbës ed25519
        if CompressedEdwardsY(addr).decompress().is_none() {
            return (addr, bump);
        }
    }
    unreachable!("nuk u gjet bump")
}

pub fn nullifier_pda(program_id: &[u8; 32], nullifier: &[u8; 32]) -> [u8; 32] {
    find_program_address(&[b"nullifier", nullifier], program_id).0
}

/// Konfigurimi i kontrollit (`--rpc-url` + `--verifier-program`).
#[derive(Clone)]
pub struct Precheck {
    pub rpc_url: String,
    pub program_id: [u8; 32],
    agent: ureq::Agent,
}

/// Rezultati i kontrollit.
#[derive(Debug, PartialEq, Eq)]
pub enum NullifierState {
    /// I papërdorur, ose i përdorur nga i njëjti portofol (rifreskim).
    Free,
    /// I regjistruar nga një portofol tjetër: prova do të refuzohej on-chain.
    UsedBy([u8; 32]),
}

impl Precheck {
    pub fn new(rpc_url: &str, program: &str) -> Result<Self> {
        let program_id: [u8; 32] = bs58::decode(program)
            .into_vec()
            .ok()
            .and_then(|v| v.try_into().ok())
            .context("--verifier-program duhet të jetë adresë base58")?;
        Ok(Self {
            rpc_url: rpc_url.into(),
            program_id,
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(3))
                .build(),
        })
    }

    /// Lexon `NullifierRecord` (8 bajte diskriminues + `owner`) nga RPC.
    pub fn nullifier_state(
        &self,
        nullifier: &[u8; 32],
        wallet: &[u8; 32],
    ) -> Result<NullifierState> {
        let pda = bs58::encode(nullifier_pda(&self.program_id, nullifier)).into_string();
        let resp: serde_json::Value = self
            .agent
            .post(&self.rpc_url)
            .send_json(serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "getAccountInfo",
                "params": [pda, {"encoding": "base64", "commitment": "confirmed"}]
            }))?
            .into_json()?;
        if let Some(e) = resp.get("error") {
            bail!("RPC: {e}");
        }
        let value = &resp["result"]["value"];
        if value.is_null() {
            return Ok(NullifierState::Free);
        }
        let owner_program = value["owner"].as_str().unwrap_or_default();
        if bs58::decode(owner_program).into_vec().ok().as_deref() != Some(&self.program_id[..]) {
            bail!("llogaria e nullifier-it nuk i përket programit verifikues");
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(value["data"][0].as_str().context("mungojnë të dhënat")?)?;
        let owner: [u8; 32] = data
            .get(8..40)
            .context("NullifierRecord i shkurtër")?
            .try_into()?;
        Ok(if &owner == wallet || owner == [0u8; 32] {
            NullifierState::Free
        } else {
            NullifierState::UsedBy(owner)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pda_is_off_curve_and_deterministic() {
        let program = [7u8; 32];
        let (a, bump) = find_program_address(&[b"nullifier", &[1u8; 32]], &program);
        assert!(CompressedEdwardsY(a).decompress().is_none());
        assert_eq!(
            (a, bump),
            find_program_address(&[b"nullifier", &[1u8; 32]], &program)
        );
        assert_ne!(a, nullifier_pda(&program, &[2u8; 32]));
    }
}
