# Anchor environment smoke — prepared, not yet verified

This independent Cargo/Anchor workspace tests Anchor `1.2.1`, Solana v4 feature selection, SBF compilation and local IDL generation. Its only instruction, `ping`, logs a message and accepts no accounts. This is not an AMM or a custody program. No wallet or deployment is required for these build checks.

Run in Linux/WSL with the reviewed pinned toolchain already installed. From `sharded_amm`, configure only the current shell, then enter this project:

```bash
project_root=$(pwd -P)
export CARGO_HOME="$project_root/.tools/linux/cargo"
export RUSTUP_HOME="$project_root/.tools/linux/rustup"
export RUSTUP_TOOLCHAIN=1.95.0
export PATH="$project_root/.tools/linux/node/node-v24.13.0-linux-x64/bin:$project_root/.tools/linux/anchor/1.2.1:$project_root/.tools/linux/agave/4.3.2/solana-release/bin:$CARGO_HOME/bin:$PATH"
cd env-anchor
anchor --version
anchor build --ignore-keys --tools-version v1.58 --no-idl -- --skip-tools-install
mkdir -p target/idl
anchor idl build --program-name anchor_environment_smoke --out target/idl/anchor_environment_smoke.json
```

`--ignore-keys` is intentional for this public fixture identity, which has no matching private key in the repository. It is not a deployment procedure. Do not run `anchor deploy`, `anchor test` or `anchor keys sync` as part of this build probe.

Anchor 1.2.1 exposes `--tools-version` as an Anchor build option. Arguments after `--` go to the SBF driver; its `--skip-tools-install` flag is therefore paired with `--no-idl` above. Generate the IDL separately without passing that driver-specific flag into host Cargo. A plain `anchor build --ignore-keys --tools-version v1.58` is another option once the platform-tools cache has been verified.

Expected evidence, only after actual successful execution: nonempty `target/deploy/anchor_environment_smoke.so` and generated IDL naming this program with exactly one `ping` instruction and an empty accounts list. Confirm the IDL address equals the fixture `declare_id!`. Build/IDL success does not establish validator execution, Transaction v1 support, AMM behavior or safety.

Sources: [Anchor 1.2.1 Solana v4 feature policy](https://www.anchor-lang.com/docs/updates/release-notes/1-2-1), [pinned CLI source for build options and IDL generation](https://github.com/otter-sec/anchor/blob/v1.2.1/cli/src/lib.rs).
