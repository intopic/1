# Toolchain lokal — build SBF i verifikuar, runtime/Anchor të bllokuara

Versionet e synuara janë Anchor CLI `1.2.1`, Agave/Solana CLI `4.3.2` dhe platform-tools `1.58` me Rust SBF `1.95.0`/LLVM `22`. URL-të dhe SHA256 e artefakteve janë në [toolchain.versions.json](../toolchain.versions.json). Prania e executable-ve ose kalimi i `--version` nuk provon ende build SBF apo ekzekutim programi; rezultatet konkrete raportohen në [STATUS.md](../STATUS.md).

Gjendja më 28 tetor 2026: hash-et e Anchor dhe Agave u verifikuan. Agave CLI funksionon dhe build-i SBF i `env-smoke` kaloi. Anchor raportoi `1.2.1` një herë; ekzekutimet pasuese u bllokuan nga antivirusi Windows si virus ose software potencialisht i padëshiruar. Validator-i native dështoi përpara hapjes së RPC-së gjatë unpack të genesis-it (`Access is denied`, edhe jashtë sandbox-it). Stack-u i plotë native nuk është pranuar si funksional. Nuk çaktivizohet antivirusi dhe nuk shtohen përjashtime apo mekanizma anashkalimi. Evidenca është në [environment-report.json](environment-report.json).

Driver-i i paketës Agave 4.3.2 raporton `cargo-build-sbf 4.4.0`, me default tools `1.57`. Build-i ynë përdor shprehimisht `--tools-version v1.58 --skip-tools-install` dhe cache-in SBF ekzistues. Ky dallim regjistrohet për riprodhim, jo normalizohet duke ndryshuar versionet e raportuara.

[Solana](https://solana.com/docs/intro/installation) dhe [Anchor](https://www.anchor-lang.com/docs/installation) dokumentojnë WSL si rrugën e Windows. Ky workspace po provon fallback native sepse release-t zyrtare publikojnë executable/archive Windows: [Anchor 1.2.1](https://github.com/otter-sec/anchor/releases/expanded_assets/v1.2.1), [Agave 4.3.2](https://github.com/anza-xyz/agave/releases/expanded_assets/v4.3.2) dhe [platform-tools 1.58](https://github.com/anza-xyz/platform-tools/releases/tag/v1.58). Kjo nuk nënkupton që çdo rrjedhë Anchor/test-validator mbështetet plotësisht native.

## Aktivizimi

Pas shkarkimit, kontrollit të hash-eve dhe nxjerrjes së artefakteve nga instalimi lokal, nga dosja `sharded_amm`:

```powershell
. ./scripts/activate-toolchain.ps1
solana --version
cargo build-sbf --help
. ./scripts/build-smoke.ps1
```

Skripti aktivizon vetëm Agave në `PATH` të procesit aktual. `-IncludeAnchor` refuzohet ndërsa manifesti shënon bllokimin antivirus. Nuk instalon asgjë, nuk shkruan registry/PATH të përdoruesit apo makinës dhe nuk ndryshon cluster-in. Mbyllja e terminalit heq aktivizimin. Kontrolloni `Get-Command solana, cargo-build-sbf` për origjinën reale të komandave. Veçmas është krijuar një identitet prove nën `.tools`, pa përdorur wallet-in e përdoruesit.

Për build real duhet manifesti i një programi Solana me target SBF; kerneli `crates/amm-model` është model host dhe nuk dëshmon këtë:

```powershell
cargo build-sbf --manifest-path env-smoke/Cargo.toml --tools-version v1.58 --skip-tools-install --offline
```

`--help` provon vetëm nisjen e build driver-it. Një build SBF, një build Anchor/IDL dhe një test ekzekutimi janë kontrolle të veçanta. Mos e shënoni toolchain të verifikuar për një fazë që nuk është ekzekutuar.

`scripts/smoke-local.ps1` nis validator-in me ledger të ri nën `.tools`, e kontrollon dhe e ndal në fund. `--log` shmang krijimin e symlink të log-ut që kërkonte privilegjin Windows 1314. Prova ndalet aktualisht te gabimi genesis; klienti Node nuk është ekzekutuar kundër një RPC-je funksionale. Klienti është i kufizuar te `http://127.0.0.1:18899` dhe përdor transaksion legacy, jo v1. Validator-i lokal përfshin faucet prove; ky konfigurim nuk është shërbim për t'u ekspozuar publikisht.

## Rruga WSL

Instalimi i `wsl --install --distribution Ubuntu --no-launch --web-download` raportoi WSL/Ubuntu të instaluara, por kërkoi restart Windows. Regjistri `Component Based Servicing/RebootPending` është true. Instaluesi dha edhe tekst shtesë për elevation; prandaj boot/regjistrimi i shpërndarjes mbeten për t'u kontrolluar pas restart-it. Kompjuteri nuk u rinis automatikisht.

**Përditësim pas restart-it:** WSL 3.0.3.0 tani funksionon, por lista nuk ka shpërndarje të regjistruar. Përfundimi i instalimit Ubuntu dështoi me `HCS_E_HYPERV_NOT_INSTALLED`. Kontrolli CPU tregon mbështetje VM/SLAT dhe `VirtualizationFirmwareEnabled=False`. Hapi i radhës është aktivizimi VTx në BIOS sipas [udhëzimit HP](BIOS_VIRTUALIZATION.md), jo thjesht një restart tjetër. Komandat Linux më poshtë presin zgjidhjen e kësaj pengese.

Pas ruajtjes së punës dhe rinisjes, hap Ubuntu dhe krijo përdoruesin Linux nëse kërkohet. Pastaj, në terminalin Ubuntu:

```bash
cd /mnt/c/Users/Perdorues/Desktop/1/sharded_amm
bash scripts/setup-wsl.sh
bash scripts/check-wsl.sh
```

Nëse mungojnë paketat bazë Linux, setup shfaq komandën e saktë apt dhe ndalet; nuk ekzekuton sudo automatikisht. Skriptet ruajnë versionet dhe mjetet Linux nën `.tools/linux`, pa ndryshuar cluster-in ose wallet-in. Krijohet vetëm një lidhje në slot-in bosh të cache-it SBF të përdoruesit Linux; një slot ekzistues ruhet dhe shkakton ndalim për rishikim.

Skripti i kontrollit provon versionet e mjeteve, build-in SBF dhe ping/pong lokal. Një projekt Anchor/IDL dhe transaksioni v1 kërkojnë prova të veçanta. Skriptet janë përgatitur; nuk quhen të testuara në WSL derisa Ubuntu të ekzekutohet realisht.

## Përputhshmëria e Anchor

[Anchor 1.2.1](https://www.anchor-lang.com/docs/updates/release-notes/1-2-1) ofron Solana v4 me opt-in. Programi i ardhshëm duhet të përdorë versione ekzakte dhe të zgjedhë feature-n në mënyrë konsistente, p.sh.:

```toml
anchor-lang = { version = "=1.2.1", default-features = false, features = ["solana-v4"] }
```

E njëjta zgjedhje kërkohet për `anchor-spl`/`anchor-client` kur përdoren; modulet SPL të nevojshme duhen shtuar shprehimisht. Mos përzieni feature-t `solana-v3` dhe `solana-v4`. Rust i host-it dhe Rust i platform-tools SBF janë dy compiler-a të ndryshëm; kontrollojini veçmas gjatë diagnostikimit.
