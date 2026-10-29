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
        MARKET_SEED, POSITION_SEED, REGISTRY_SEED, SCHEMA_VERSION, SHARD_SEED, TICK_ARRAY_SEED,
        VAULT_AUTH_SEED, VAULT_SEED,
    },
    instructions::{InitializeMarketArgs, SwapArgs},
    state::{
        MarketConfig, MarketParams, Position, ReadinessParams, ShardRegistry, ShardState, TickArray,
    },
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
pub const NATIVE_MINT: Pubkey = anchor_lang::pubkey!("So11111111111111111111111111111111111111112");
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

pub fn default_readiness() -> ReadinessParams {
    ReadinessParams {
        min_liquidity: 1_000,
        min_reserve_a: 1_000,
        min_reserve_b: 1_000,
        max_activation_deviation_bps: 100,
        depth_probe_a: 1_000,
        depth_probe_b: 1_000,
        max_depth_impact_bps: 2_000,
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
        Self::with_svm(LiteSVM::new())
    }

    /// Like `new`, but with only the feature gates active on mainnet-beta
    /// (LiteSVM's snapshot), so cluster limits such as the 64 account locks
    /// per transaction apply.
    pub fn mainnet() -> Self {
        Self::with_svm(LiteSVM::new().with_mainnet_features())
    }

    pub fn with_svm(mut svm: LiteSVM) -> Self {
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
        if let (Err(f), Ok(_)) = (&r, std::env::var("AMM_LOGS")) {
            eprintln!("tx failed: {:?}\n{}", f.err, f.meta.logs.join("\n"));
        }
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
            readiness: default_readiness(),
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

impl Env {
    pub fn ix_set_paused(m: &Market, authority: &Pubkey, paused: bool) -> Instruction {
        let accounts = sharded_amm::accounts::SetPaused {
            authority: *authority,
            market: m.market,
        };
        Instruction {
            program_id: sharded_amm::ID,
            accounts: accounts.to_account_metas(None),
            data: sharded_amm::instruction::SetPaused { paused }.data(),
        }
    }

    /// Fee change with the dynamic fee off.
    pub fn ix_update_fees(
        m: &Market,
        admin: &Pubkey,
        fee_rate_ppm: u32,
        protocol_share_bps: u16,
    ) -> Instruction {
        Self::ix_update_fees_dynamic(
            m,
            admin,
            fee_rate_ppm,
            protocol_share_bps,
            sharded_amm::state::DynamicFeeParams::disabled(fee_rate_ppm),
        )
    }

    pub fn ix_update_rebalancers(m: &Market, admin: &Pubkey, keys: &[Pubkey]) -> Instruction {
        let mut k = [Pubkey::default(); sharded_amm::constants::MAX_REBALANCERS];
        k[..keys.len()].copy_from_slice(keys);
        ix(
            sharded_amm::accounts::UpdateFees {
                admin: *admin,
                market: m.market,
            },
            sharded_amm::instruction::UpdateRebalancers { keys: k },
        )
    }

    pub fn ix_apply_rebalancers(m: &Market, admin: &Pubkey) -> Instruction {
        ix(
            sharded_amm::accounts::UpdateFees {
                admin: *admin,
                market: m.market,
            },
            sharded_amm::instruction::ApplyRebalancers {},
        )
    }

    /// Lists `keys` as the market's rebalancers: propose, wait out the
    /// timelock, apply.
    pub fn set_rebalancers(&mut self, m: &Market, keys: &[Pubkey]) {
        let admin = self.admin.insecure_clone();
        self.send(
            &[Self::ix_update_rebalancers(m, &admin.pubkey(), keys)],
            &[&admin],
        )
        .expect("update_rebalancers");
        self.warp(sharded_amm::constants::FEE_TIMELOCK_SECONDS);
        self.send(&[Self::ix_apply_rebalancers(m, &admin.pubkey())], &[&admin])
            .expect("apply_rebalancers");
    }

    pub fn ix_update_fees_dynamic(
        m: &Market,
        admin: &Pubkey,
        fee_rate_ppm: u32,
        protocol_share_bps: u16,
        dynamic: sharded_amm::state::DynamicFeeParams,
    ) -> Instruction {
        let accounts = sharded_amm::accounts::UpdateFees {
            admin: *admin,
            market: m.market,
        };
        Instruction {
            program_id: sharded_amm::ID,
            accounts: accounts.to_account_metas(None),
            data: sharded_amm::instruction::UpdateFees {
                fee_rate_ppm,
                protocol_share_bps,
                dynamic,
            }
            .data(),
        }
    }

    /// Overwrites the market status in place. Test-only shortcut until
    /// finalize (ENG-04) can open a market through the program.
    pub fn force_market_status(&mut self, m: &Market, status: sharded_amm::state::MarketStatus) {
        use anchor_lang::AccountSerialize;
        let mut cfg = self.market_state(m);
        cfg.status = status;
        let mut acc = self.raw(&m.market).unwrap();
        let mut data = Vec::new();
        cfg.try_serialize(&mut data).unwrap();
        acc.data = data;
        self.svm.set_account(m.market, acc).unwrap();
    }
}

// ---------------------------------------------------------------------------
// ENG-04/06 helpers: token accounts, liquidity, market opening and swaps.

use solana_instruction::AccountMeta;

pub const Q64: u128 = 1 << 64;

/// Raw 165-byte initialized SPL token account.
pub fn token_account_data(mint: &Pubkey, owner: &Pubkey, amount: u64) -> Vec<u8> {
    let mut d = vec![0u8; 165];
    d[0..32].copy_from_slice(mint.as_ref());
    d[32..64].copy_from_slice(owner.as_ref());
    d[64..72].copy_from_slice(&amount.to_le_bytes());
    d[108] = 1; // AccountState::Initialized
    d
}

pub fn ix(accounts: impl ToAccountMetas, data: impl InstructionData) -> Instruction {
    Instruction {
        program_id: sharded_amm::ID,
        accounts: accounts.to_account_metas(None),
        data: data.data(),
    }
}

pub fn with_remaining(mut i: Instruction, keys: &[Pubkey]) -> Instruction {
    i.accounts
        .extend(keys.iter().map(|k| AccountMeta::new(*k, false)));
    i
}

pub fn tick_array_pda(shard: &Pubkey, start: i32) -> Pubkey {
    pda(&[TICK_ARRAY_SEED, shard.as_ref(), &start.to_le_bytes()])
}

pub fn position_pda(shard: &Pubkey, owner: &Pubkey, lower: i32, upper: i32) -> Pubkey {
    pda(&[
        POSITION_SEED,
        shard.as_ref(),
        owner.as_ref(),
        &lower.to_le_bytes(),
        &upper.to_le_bytes(),
    ])
}

pub fn array_start(tick: i32, spacing: u16) -> i32 {
    sharded_amm::state::array_start_for(tick, spacing)
}

pub struct Lp {
    pub kp: Keypair,
    pub token_a: Pubkey,
    pub token_b: Pubkey,
}

impl Env {
    /// A classic SPL token account. For the native mint (WSOL) it is a real
    /// native account: `is_native = Some(rent)` and lamports = rent + amount,
    /// as the token program requires to move wrapped SOL.
    pub fn token_account(&mut self, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Pubkey {
        let key = Pubkey::new_unique();
        let mut data = token_account_data(mint, owner, amount);
        let rent = self.svm.minimum_balance_for_rent_exemption(165);
        let mut lamports = rent;
        if *mint == NATIVE_MINT {
            data[109..113].copy_from_slice(&1u32.to_le_bytes());
            data[113..121].copy_from_slice(&rent.to_le_bytes());
            lamports += amount;
        }
        self.svm
            .set_account(
                key,
                Account {
                    lamports,
                    data,
                    owner: TOKEN_PROGRAM,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        key
    }

    pub fn balance(&self, key: &Pubkey) -> u64 {
        token_account_fields(&self.raw(key).expect("token account").data).2
    }

    pub fn user(&mut self, m: &Market, amount: u64) -> Lp {
        let kp = Keypair::new();
        self.svm.airdrop(&kp.pubkey(), 50_000_000_000).unwrap();
        let token_a = self.token_account(&m.mint_a, &kp.pubkey(), amount);
        let token_b = self.token_account(&m.mint_b, &kp.pubkey(), amount);
        Lp {
            kp,
            token_a,
            token_b,
        }
    }

    pub fn tick_array(&self, key: &Pubkey) -> TickArray {
        let acc = self.raw(key).expect("tick array exists");
        *bytemuck_from(&acc.data[8..])
    }

    pub fn position(&self, key: &Pubkey) -> Position {
        self.account(key)
    }

    pub fn now(&self) -> i64 {
        self.svm
            .get_sysvar::<anchor_lang::prelude::Clock>()
            .unix_timestamp
    }

    pub fn warp(&mut self, seconds: i64) {
        let mut c = self.svm.get_sysvar::<anchor_lang::prelude::Clock>();
        c.unix_timestamp += seconds;
        c.slot += 1;
        self.svm.set_sysvar(&c);
    }

    pub fn ix_initialize_price(
        m: &Market,
        authority: &Pubkey,
        id: u8,
        sqrt_price_x64: u128,
    ) -> Instruction {
        ix(
            sharded_amm::accounts::InitializePrice {
                authority: *authority,
                market: m.market,
                shard: shard_addrs(m, id, 0).shard,
            },
            sharded_amm::instruction::InitializePrice { sqrt_price_x64 },
        )
    }

    pub fn ix_init_tick_array(
        m: &Market,
        payer: &Pubkey,
        shard: &Pubkey,
        start: i32,
    ) -> Instruction {
        ix(
            sharded_amm::accounts::InitTickArray {
                payer: *payer,
                market: m.market,
                shard: *shard,
                tick_array: tick_array_pda(shard, start),
                system_program: SYSTEM_PROGRAM,
            },
            sharded_amm::instruction::InitTickArray {
                start_tick_index: start,
            },
        )
    }

    pub fn ix_open_position(
        m: &Market,
        owner: &Pubkey,
        shard: &Pubkey,
        lower: i32,
        upper: i32,
    ) -> Instruction {
        ix(
            sharded_amm::accounts::OpenPosition {
                owner: *owner,
                payer: *owner,
                market: m.market,
                shard: *shard,
                position: position_pda(shard, owner, lower, upper),
                system_program: SYSTEM_PROGRAM,
            },
            sharded_amm::instruction::OpenPosition {
                tick_lower: lower,
                tick_upper: upper,
            },
        )
    }

    pub fn modify_accounts(
        m: &Market,
        lp: &Lp,
        id: u8,
        lower: i32,
        upper: i32,
    ) -> sharded_amm::accounts::ModifyLiquidity {
        let a = shard_addrs(m, id, 0);
        sharded_amm::accounts::ModifyLiquidity {
            owner: lp.kp.pubkey(),
            market: m.market,
            shard: a.shard,
            position: position_pda(&a.shard, &lp.kp.pubkey(), lower, upper),
            vault_a: a.vault_a,
            vault_b: a.vault_b,
            owner_token_a: lp.token_a,
            owner_token_b: lp.token_b,
            vault_authority: a.vault_authority,
            token_program: TOKEN_PROGRAM,
        }
    }

    /// Tick arrays holding `lower` and `upper` (one key if they share an array).
    pub fn boundary_arrays(&self, m: &Market, id: u8, lower: i32, upper: i32) -> Vec<Pubkey> {
        let spacing = self.market_state(m).params.tick_spacing;
        let shard = shard_addrs(m, id, 0).shard;
        let (l, u) = (array_start(lower, spacing), array_start(upper, spacing));
        let mut v = vec![tick_array_pda(&shard, l)];
        if u != l {
            v.push(tick_array_pda(&shard, u));
        }
        v
    }

    pub fn ix_increase(
        &self,
        m: &Market,
        lp: &Lp,
        id: u8,
        lower: i32,
        upper: i32,
        liquidity: u128,
    ) -> Instruction {
        with_remaining(
            ix(
                Self::modify_accounts(m, lp, id, lower, upper),
                sharded_amm::instruction::IncreaseLiquidity {
                    liquidity,
                    amount_a_max: u64::MAX,
                    amount_b_max: u64::MAX,
                },
            ),
            &self.boundary_arrays(m, id, lower, upper),
        )
    }

    pub fn ix_decrease(
        &self,
        m: &Market,
        lp: &Lp,
        id: u8,
        lower: i32,
        upper: i32,
        liquidity: u128,
    ) -> Instruction {
        with_remaining(
            ix(
                Self::modify_accounts(m, lp, id, lower, upper),
                sharded_amm::instruction::DecreaseLiquidity {
                    liquidity,
                    amount_a_min: 0,
                    amount_b_min: 0,
                },
            ),
            &self.boundary_arrays(m, id, lower, upper),
        )
    }

    pub fn ix_collect(&self, m: &Market, lp: &Lp, id: u8, lower: i32, upper: i32) -> Instruction {
        with_remaining(
            ix(
                Self::modify_accounts(m, lp, id, lower, upper),
                sharded_amm::instruction::CollectFees {},
            ),
            &self.boundary_arrays(m, id, lower, upper),
        )
    }

    /// Creates any missing tick arrays covering `[from, to]` for one shard.
    pub fn ensure_arrays(&mut self, m: &Market, id: u8, from: i32, to: i32) {
        let spacing = self.market_state(m).params.tick_spacing;
        let span = sharded_amm::state::array_span(spacing);
        let shard = shard_addrs(m, id, 0).shard;
        let payer = self.keeper.insecure_clone();
        let mut start = array_start(from, spacing);
        while start <= to {
            if self.raw(&tick_array_pda(&shard, start)).is_none() {
                self.send(
                    &[Self::ix_init_tick_array(m, &payer.pubkey(), &shard, start)],
                    &[&payer],
                )
                .expect("init tick array");
            }
            start += span;
        }
    }

    /// Opens (if needed) and funds a position.
    pub fn add_liquidity(
        &mut self,
        m: &Market,
        lp: &Lp,
        id: u8,
        lower: i32,
        upper: i32,
        liquidity: u128,
    ) -> Result<u64, TransactionError> {
        self.ensure_arrays(m, id, lower, upper);
        let shard = shard_addrs(m, id, 0).shard;
        let mut ixs = Vec::new();
        if self
            .raw(&position_pda(&shard, &lp.kp.pubkey(), lower, upper))
            .is_none()
        {
            ixs.push(Self::ix_open_position(
                m,
                &lp.kp.pubkey(),
                &shard,
                lower,
                upper,
            ));
        }
        ixs.push(self.ix_increase(m, lp, id, lower, upper, liquidity));
        let kp = lp.kp.insecure_clone();
        let r = self.send(&ixs, &[&kp]);
        // A prepared shard needs a passing depth probe before it can open or
        // activate (rule 4); re-run it after every deposit, as a keeper
        // would. A shard too shallow to pass simply stays unverified.
        if r.is_ok() && self.shard_state(m, id).status == sharded_amm::state::ShardStatus::Prepared
        {
            let _ = self.verify_depth(m, id);
        }
        r
    }

    /// `verify_depth` with up to three arrays each way from the current tick.
    pub fn ix_verify_depth(&self, m: &Market, id: u8) -> Instruction {
        let down = self.swap_arrays(m, id, true);
        let up = self.swap_arrays(m, id, false);
        let a = shard_addrs(m, id, 0);
        let i = ix(
            sharded_amm::accounts::VerifyDepth {
                market: m.market,
                shard: a.shard,
            },
            sharded_amm::instruction::VerifyDepth {
                arrays_a_to_b: down.len() as u8,
            },
        );
        let metas: Vec<Pubkey> = down.into_iter().chain(up).collect();
        let mut i = with_remaining(i, &metas);
        for meta in i.accounts.iter_mut().skip(2) {
            meta.is_writable = false;
        }
        i
    }

    pub fn verify_depth(&mut self, m: &Market, id: u8) -> Result<u64, TransactionError> {
        let i = self.ix_verify_depth(m, id);
        let keeper = self.keeper.insecure_clone();
        self.send(&[i], &[&keeper])
    }

    pub fn ix_open_market(&self, m: &Market, authority: &Pubkey, ids: &[u8]) -> Instruction {
        let mut keys = Vec::new();
        for &id in ids {
            let a = shard_addrs(m, id, 0);
            keys.extend([a.shard, a.vault_a, a.vault_b]);
        }
        let mut i = ix(
            sharded_amm::accounts::OpenMarket {
                authority: *authority,
                market: m.market,
                registry: m.registry,
            },
            sharded_amm::instruction::OpenMarket {},
        );
        for (n, k) in keys.iter().enumerate() {
            // Shards are writable, vaults read-only.
            i.accounts.push(if n % 3 == 0 {
                AccountMeta::new(*k, false)
            } else {
                AccountMeta::new_readonly(*k, false)
            });
        }
        i
    }

    pub fn ix_cancel_setup(m: &Market, admin: &Pubkey) -> Instruction {
        ix(
            sharded_amm::accounts::CancelSetup {
                admin: *admin,
                market: m.market,
            },
            sharded_amm::instruction::CancelSetup {},
        )
    }

    pub fn ix_activate(m: &Market, keeper: &Pubkey, id: u8, reference: u8) -> Instruction {
        let a = shard_addrs(m, id, 0);
        ix(
            sharded_amm::accounts::ActivateShard {
                keeper: *keeper,
                market: m.market,
                registry: m.registry,
                shard: a.shard,
                vault_a: a.vault_a,
                vault_b: a.vault_b,
                reference_shard: shard_addrs(m, reference, 0).shard,
            },
            sharded_amm::instruction::ActivateShard {},
        )
    }

    pub fn ix_apply_fees(m: &Market, admin: &Pubkey) -> Instruction {
        ix(
            sharded_amm::accounts::UpdateFees {
                admin: *admin,
                market: m.market,
            },
            sharded_amm::instruction::ApplyFees {},
        )
    }

    pub fn ix_cancel_fee_change(m: &Market, admin: &Pubkey) -> Instruction {
        ix(
            sharded_amm::accounts::UpdateFees {
                admin: *admin,
                market: m.market,
            },
            sharded_amm::instruction::CancelFeeChange {},
        )
    }

    pub fn ix_collect_protocol(
        m: &Market,
        revenue: &Pubkey,
        id: u8,
        dest_a: Pubkey,
        dest_b: Pubkey,
    ) -> Instruction {
        let a = shard_addrs(m, id, 0);
        ix(
            sharded_amm::accounts::CollectProtocolFees {
                revenue_authority: *revenue,
                market: m.market,
                shard: a.shard,
                vault_a: a.vault_a,
                vault_b: a.vault_b,
                destination_a: dest_a,
                destination_b: dest_b,
                vault_authority: a.vault_authority,
                token_program: TOKEN_PROGRAM,
            },
            sharded_amm::instruction::CollectProtocolFees {},
        )
    }

    /// Tick arrays for a swap from the current tick, in direction, up to 3,
    /// only those that exist.
    pub fn swap_arrays(&self, m: &Market, id: u8, a_to_b: bool) -> Vec<Pubkey> {
        let spacing = self.market_state(m).params.tick_spacing;
        let span = sharded_amm::state::array_span(spacing);
        let st = self.shard_state(m, id);
        let shard = shard_addrs(m, id, 0).shard;
        let mut start = array_start(st.tick_current, spacing);
        let mut v = Vec::new();
        for _ in 0..3 {
            let k = tick_array_pda(&shard, start);
            if self.raw(&k).is_none() {
                break;
            }
            v.push(k);
            start += if a_to_b { -span } else { span };
        }
        v
    }

    pub fn swap_args(&self, a_to_b: bool, amount_in: u64) -> SwapArgs {
        SwapArgs {
            a_to_b,
            amount_in,
            min_amount_out: 0,
            sqrt_price_limit_x64: 0,
            max_rounding_dust: u64::MAX,
            deadline: self.now() + 60,
        }
    }

    pub fn ix_swap(
        &self,
        m: &Market,
        user: &Lp,
        id: u8,
        args: SwapArgs,
        arrays: &[Pubkey],
    ) -> Instruction {
        let a = shard_addrs(m, id, 0);
        let (tin, tout) = if args.a_to_b {
            (user.token_a, user.token_b)
        } else {
            (user.token_b, user.token_a)
        };
        with_remaining(
            ix(
                sharded_amm::accounts::SwapExactIn {
                    user: user.kp.pubkey(),
                    market: m.market,
                    registry: m.registry,
                    shard: a.shard,
                    vault_a: a.vault_a,
                    vault_b: a.vault_b,
                    user_token_in: tin,
                    user_token_out: tout,
                    vault_authority: a.vault_authority,
                    token_program: TOKEN_PROGRAM,
                },
                sharded_amm::instruction::SwapExactIn { args },
            ),
            arrays,
        )
    }

    pub fn swap(
        &mut self,
        m: &Market,
        user: &Lp,
        id: u8,
        a_to_b: bool,
        amount: u64,
    ) -> Result<u64, TransactionError> {
        let arrays = self.swap_arrays(m, id, a_to_b);
        let i = self.ix_swap(m, user, id, self.swap_args(a_to_b, amount), &arrays);
        let kp = user.kp.insecure_clone();
        self.send(&[i], &[&kp])
    }

    /// Prepares shards 0..8, sets price 1.0 (tick 0), funds a ±1200-tick
    /// position in each and opens the market. Returns the LP.
    pub fn open_seeded_market(&mut self) -> (Market, Lp) {
        let m = self.market();
        let lp = self.seed_initial_shards(&m, 1_000_000_000_000);
        let keeper = self.keeper.insecure_clone();
        let i = self.ix_open_market(&m, &keeper.pubkey(), &(0..8).collect::<Vec<_>>());
        self.send(&[i], &[&keeper]).expect("open_market");
        (m, lp)
    }

    pub fn seed_initial_shards(&mut self, m: &Market, liquidity: u128) -> Lp {
        let keeper = self.keeper.insecure_clone();
        let lp = self.user(m, u64::MAX / 4);
        for id in 0..8 {
            self.prepare(m, &keeper, id).expect("prepare");
            self.send(
                &[Self::ix_initialize_price(m, &keeper.pubkey(), id, Q64)],
                &[&keeper],
            )
            .expect("price");
            self.add_liquidity(m, &lp, id, -1200, 1200, liquidity)
                .expect("seed liquidity");
        }
        lp
    }
}

/// Reads a zero-copy struct from account bytes after the discriminator.
pub fn bytemuck_from<T: Copy>(bytes: &[u8]) -> &T {
    assert!(bytes.len() >= std::mem::size_of::<T>());
    assert_eq!(
        bytes.as_ptr() as usize % std::mem::align_of::<T>(),
        0,
        "unaligned account data"
    );
    unsafe { &*(bytes.as_ptr() as *const T) }
}
