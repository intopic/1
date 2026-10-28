//! Shared LiteSVM harness for the sharded-amm program.
//!
//! Loads the SBF artifact built by `cargo build-sbf` (path overridable with
//! `SHARDED_AMM_SO`). Mints are written directly as SPL Token accounts so the
//! tests control decimals and authorities without extra CPIs.
#![allow(dead_code)]

use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use litesvm::LiteSVM;
use sharded_amm::{
    constants::{
        MARKET_SEED, REGISTRY_SEED, SCHEMA_VERSION, SHARD_SEED, VAULT_AUTH_SEED, VAULT_SEED,
    },
    instructions::InitializeMarketArgs,
    state::{MarketConfig, MarketParams, ShardRegistry, ShardState},
};
use solana_account::Account;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

pub use anchor_lang::prelude::Pubkey;
pub use solana_instruction::error::InstructionError;

pub const TOKEN_PROGRAM: Pubkey = anchor_spl::token::ID;
pub const TOKEN_2022_PROGRAM: Pubkey = anchor_spl::token_2022::ID;
pub const SYSTEM_PROGRAM: Pubkey = anchor_lang::system_program::ID;
pub const RENT_SYSVAR: Pubkey = anchor_lang::pubkey!("SysvarRent111111111111111111111111111111111");

/// Anchor's numeric codes for account-constraint failures used below.
pub mod anchor_codes {
    pub const CONSTRAINT_HAS_ONE: u32 = 2001;
    pub const CONSTRAINT_SEEDS: u32 = 2006;
    pub const ACCOUNT_OWNED_BY_WRONG_PROGRAM: u32 = 3007;
    pub const INVALID_PROGRAM_ID: u32 = 3008;
    pub const ACCOUNT_NOT_SIGNER: u32 = 3010;
}

pub fn amm_code(e: sharded_amm::errors::AmmError) -> u32 {
    6000 + e as u32
}

pub fn program_so() -> String {
    std::env::var("SHARDED_AMM_SO").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/deploy/sharded_amm.so"
        )
        .to_string()
    })
}

pub struct Env {
    pub svm: LiteSVM,
    pub admin: Keypair,
    pub keeper: Keypair,
    pub revenue: Keypair,
    pub pauser: Keypair,
}

pub struct Market {
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub market_id: u64,
    pub market: Pubkey,
    pub registry: Pubkey,
}

pub struct ShardAddrs {
    pub shard: Pubkey,
    pub vault_authority: Pubkey,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
}

pub fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &sharded_amm::ID).0
}

pub fn market_pda(mint_a: &Pubkey, mint_b: &Pubkey, market_id: u64) -> Pubkey {
    pda(&[
        MARKET_SEED,
        &SCHEMA_VERSION.to_le_bytes(),
        mint_a.as_ref(),
        mint_b.as_ref(),
        &market_id.to_le_bytes(),
    ])
}

pub fn shard_addrs(market: &Market, id: u8, generation: u64) -> ShardAddrs {
    let shard = pda(&[
        SHARD_SEED,
        market.market.as_ref(),
        &[id],
        &generation.to_le_bytes(),
    ]);
    ShardAddrs {
        shard,
        vault_authority: pda(&[VAULT_AUTH_SEED, shard.as_ref()]),
        vault_a: pda(&[VAULT_SEED, shard.as_ref(), market.mint_a.as_ref()]),
        vault_b: pda(&[VAULT_SEED, shard.as_ref(), market.mint_b.as_ref()]),
    }
}

pub fn default_params() -> MarketParams {
    MarketParams {
        fee_rate_ppm: 3_000,
        protocol_share_bps: 2_000,
        tick_spacing: 60,
    }
}

/// Raw 82-byte SPL Token mint (classic layout).
pub fn mint_data(decimals: u8, authority: &Pubkey, freeze: Option<&Pubkey>) -> Vec<u8> {
    let mut d = Vec::with_capacity(82);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(authority.as_ref());
    d.extend_from_slice(&0u64.to_le_bytes()); // supply
    d.push(decimals);
    d.push(1); // is_initialized
    match freeze {
        Some(f) => {
            d.extend_from_slice(&1u32.to_le_bytes());
            d.extend_from_slice(f.as_ref());
        }
        None => d.extend_from_slice(&[0u8; 36]),
    }
    d
}

impl Env {
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program_from_file(sharded_amm::ID, program_so())
            .expect("load target/deploy/sharded_amm.so (run cargo build-sbf first)");
        let env = Env {
            svm,
            admin: Keypair::new(),
            keeper: Keypair::new(),
            revenue: Keypair::new(),
            pauser: Keypair::new(),
        };
        let mut env = env;
        for kp in [&env.admin, &env.keeper, &env.revenue, &env.pauser] {
            env.svm.airdrop(&kp.pubkey(), 100_000_000_000).unwrap();
        }
        env
    }

    pub fn create_mint(&mut self, decimals: u8, owner_program: Pubkey) -> Pubkey {
        let key = Pubkey::new_unique();
        let data = mint_data(decimals, &self.admin.pubkey(), None);
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                key,
                Account {
                    lamports,
                    data,
                    owner: owner_program,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        key
    }

    /// Two classic SPL mints with different decimals (9 and 6), sorted.
    pub fn sorted_mints(&mut self) -> (Pubkey, Pubkey) {
        let x = self.create_mint(9, TOKEN_PROGRAM);
        let y = self.create_mint(6, TOKEN_PROGRAM);
        if x.to_bytes() < y.to_bytes() {
            (x, y)
        } else {
            (y, x)
        }
    }

    pub fn send(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> Result<u64, TransactionError> {
        let payer = signers[0].pubkey();
        let msg = Message::new(ixs, Some(&payer));
        let tx = Transaction::new(signers, msg, self.svm.latest_blockhash());
        let r = self.svm.send_transaction(tx);
        self.svm.expire_blockhash();
        r.map(|m| m.compute_units_consumed).map_err(|f| f.err)
    }

    pub fn account<T: AccountDeserialize>(&self, key: &Pubkey) -> T {
        let acc = self.svm.get_account(key).expect("account exists");
        T::try_deserialize(&mut acc.data.as_slice()).expect("deserialize")
    }

    pub fn raw(&self, key: &Pubkey) -> Option<Account> {
        self.svm.get_account(key)
    }

    pub fn init_args(&self, market_id: u64, params: MarketParams) -> InitializeMarketArgs {
        InitializeMarketArgs {
            market_id,
            params,
            keeper: self.keeper.pubkey(),
            revenue_authority: self.revenue.pubkey(),
            pauser: self.pauser.pubkey(),
        }
    }

    pub fn ix_initialize_market(
        &self,
        admin: &Pubkey,
        mint_a: Pubkey,
        mint_b: Pubkey,
        args: InitializeMarketArgs,
    ) -> Instruction {
        let market = market_pda(&mint_a, &mint_b, args.market_id);
        let accounts = sharded_amm::accounts::InitializeMarket {
            admin: *admin,
            mint_a,
            mint_b,
            market,
            registry: pda(&[REGISTRY_SEED, market.as_ref()]),
            token_program: TOKEN_PROGRAM,
            system_program: SYSTEM_PROGRAM,
        };
        Instruction {
            program_id: sharded_amm::ID,
            accounts: accounts.to_account_metas(None),
            data: sharded_amm::instruction::InitializeMarket { args }.data(),
        }
    }

    /// Creates a market with sorted test mints and default params.
    pub fn market(&mut self) -> Market {
        let (mint_a, mint_b) = self.sorted_mints();
        self.market_with(mint_a, mint_b, 1)
    }

    pub fn market_with(&mut self, mint_a: Pubkey, mint_b: Pubkey, market_id: u64) -> Market {
        let ix = self.ix_initialize_market(
            &self.admin.pubkey(),
            mint_a,
            mint_b,
            self.init_args(market_id, default_params()),
        );
        let admin = self.admin.insecure_clone();
        self.send(&[ix], &[&admin]).expect("initialize_market");
        let market = market_pda(&mint_a, &mint_b, market_id);
        Market {
            mint_a,
            mint_b,
            market_id,
            market,
            registry: pda(&[REGISTRY_SEED, market.as_ref()]),
        }
    }

    pub fn prepare_accounts(
        &self,
        m: &Market,
        authority: &Pubkey,
        id: u8,
        generation: u64,
    ) -> sharded_amm::accounts::PrepareShard {
        let a = shard_addrs(m, id, generation);
        sharded_amm::accounts::PrepareShard {
            authority: *authority,
            payer: *authority,
            market: m.market,
            registry: m.registry,
            mint_a: m.mint_a,
            mint_b: m.mint_b,
            shard: a.shard,
            vault_authority: a.vault_authority,
            vault_a: a.vault_a,
            vault_b: a.vault_b,
            token_program: TOKEN_PROGRAM,
            system_program: SYSTEM_PROGRAM,
            rent: RENT_SYSVAR,
        }
    }

    pub fn ix_prepare(
        accounts: sharded_amm::accounts::PrepareShard,
        id: u8,
        generation: u64,
    ) -> Instruction {
        Instruction {
            program_id: sharded_amm::ID,
            accounts: accounts.to_account_metas(None),
            data: sharded_amm::instruction::PrepareShard { id, generation }.data(),
        }
    }

    pub fn prepare(
        &mut self,
        m: &Market,
        signer: &Keypair,
        id: u8,
    ) -> Result<u64, TransactionError> {
        let ix = Self::ix_prepare(self.prepare_accounts(m, &signer.pubkey(), id, 0), id, 0);
        self.send(&[ix], &[signer])
    }

    pub fn registry(&self, m: &Market) -> ShardRegistry {
        self.account(&m.registry)
    }

    pub fn market_state(&self, m: &Market) -> MarketConfig {
        self.account(&m.market)
    }

    pub fn shard_state(&self, m: &Market, id: u8) -> ShardState {
        self.account(&shard_addrs(m, id, 0).shard)
    }
}

pub fn expect_custom(r: Result<u64, TransactionError>, code: u32) {
    match r {
        Err(TransactionError::InstructionError(_, InstructionError::Custom(c))) => {
            assert_eq!(c, code, "expected custom error {code}, got {c}")
        }
        other => panic!("expected custom error {code}, got {other:?}"),
    }
}

pub fn expect_err(r: Result<u64, TransactionError>) -> TransactionError {
    r.expect_err("transaction should fail")
}

/// SPL token account fields at fixed offsets: mint, owner, amount.
pub fn token_account_fields(data: &[u8]) -> (Pubkey, Pubkey, u64) {
    let mint = Pubkey::try_from(&data[0..32]).unwrap();
    let owner = Pubkey::try_from(&data[32..64]).unwrap();
    let amount = u64::from_le_bytes(data[64..72].try_into().unwrap());
    (mint, owner, amount)
}
