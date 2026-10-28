# Provë e mjedisit Solana

Ky program i veçuar pranon vetëm instruksionin `ping` dhe shkruan një log `pong`. Përdoret për të provuar build SBF dhe ngarkimin/ekzekutimin lokal. Nuk është programi AMM dhe nuk mban fonde.

Workspace-i është i veçuar nga modeli Rust, për të ruajtur kontrollet e tij pa varësi. Versioni SDK është fiksuar në `solana-program = 4.1.0`.

Pas aktivizimit të mjeteve Agave:

```powershell
cargo build-sbf --manifest-path env-smoke/Cargo.toml --tools-version v1.58 --skip-tools-install --offline
```

Build-i, ngarkimi në validator dhe ekzekutimi janë prova të ndryshme. Statusi real raportohet në `../docs/toolchain.md` dhe `../STATUS.md`.
