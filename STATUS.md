# Gjendja aktuale

Përditësuar: 28 tetor 2026.

**Vendimi aktual i produktit:** nisje me **8 shard-e**, ID 0–7, maskë `0xff`; zgjerim dinamik me kapital të verifikuar deri në **56**, ID 0–55. Përmbledhja dhe struktura e propozuar janë te `docs/OVERVIEW_RUST_ANCHOR_8_TO_56.md`. Kjo zëvendëson propozimin e mëparshëm me katër shard-e, por nuk do të thotë se kontrata/bitmask-i janë implementuar.

**Për developer-in:** `docs/DEVELOPER_HANDOFF_8_SHARDS.md` përmban specifikimin e detajuar, milestone-et, testet dhe një hartë referencash Raydium me commit të fiksuar. Licenca aktuale Orca është dalluar nga Apache 2.0; nuk është importuar kod Orca ose Raydium dhe nuk pretendohet trashëgimi auditimi/sigurie. Ky është dorëzim dokumentacioni, jo implementim i ri i kontratës.

## Kryer dhe verifikuar

- Workspace lokal Rust me crate `amm-model`, pa varësi të jashtme.
- README, plan fazash dhe skripte për kontrollin e mjedisit/validimin lokal.
- Specifikimi i arkitekturës dhe matrica PDA/R/W për veprimet e propozuara.
- Modeli i kërcënimeve me 13 rreziqe dhe matrica e pranimit G0–G4; provat e modelit shënohen si mbulim i pjesshëm, pa kaluar ende portat onchain.
- Model integer për quote exact-in dhe përditësimin e rezervave virtuale, në dy drejtime, me tarifa dhe refuzim overflow.
- Agave/Solana CLI 4.3.2 e instaluar brenda `.tools` dhe e verifikuar; aktivizohet me `scripts/activate-toolchain.ps1`, pa ndryshuar PATH global.
- Anchor 1.2.1 u shkarkua dhe hash-i u verifikua; Windows antivirus bllokoi ekzekutimin. Nuk u çaktivizua mbrojtja dhe nuk u shtuan përjashtime.
- **Build SBF kaloi** për programin e veçuar `env-smoke`, me SDK 4.1.0 dhe platform-tools 1.58. Artefakti `environment_smoke.so` ka 10,608 bajt; hash-i është te `docs/environment-report.json`.
- Validator-i native nisi executable-n, por nuk arriti RPC ready: me `--log` kaloi pengesën e log symlink 1314, pastaj dështoi gjatë genesis unpack me AccessDenied, edhe jashtë sandbox-it.
- Pas rinisjes nga përdoruesi, WSL **3.0.3.0** funksionon me version default 2. Lista nuk ka ende shpërndarje Linux të regjistruar.
- Përfundimi i instalimit Ubuntu dështoi me `HCS_E_HYPERV_NOT_INSTALLED`: virtualizimi firmware është i fikur. CPU Intel i5-7200U mbështet VM extensions dhe SLAT, por `VirtualizationFirmwareEnabled = False`. Kompjuteri është **HP ProBook 430 G4**. Kërkohet aktivizim i VTx në BIOS përpara nisjes Linux.
- Projekti i izoluar `env-anchor` është përgatitur për build Anchor dhe IDL, por nuk është ndërtuar ende.
- Skriptet e build-it, validator-it dhe klientit provues ruhen në `scripts`; klienti pranon vetëm RPC lokale dhe ende nuk ka kaluar provën end-to-end.
- Skriptet Linux `setup-wsl.sh`/`check-wsl.sh` kaluan kontrollin sintaksor `bash -n`; klienti Node kaloi `node --check` dhe skriptet PowerShell kaluan parser-in. Instalimi/ekzekutimi i këtyre skripteve në Ubuntu mbetet i paprovuar deri pas restart-it.
- `scripts/check.ps1`: kaloi formatimi, **5 teste Rust** dhe Clippy me `-D warnings`, në mënyrë offline.

Testet mbulojnë ruajtjen e input-it dhe tarifat në model, input-e të pavlefshme, overflow pa ndryshim të pjesshëm gjendjeje, fragmentimin e likuiditetit dhe quote të vjetra kundrejt ekzekutimit vijues.

## Hapi i radhës

**P1-01 është pjesërisht e kryer.** Build-i SBF kaloi; Anchor/IDL dhe runtime lokal mbeten të paverifikuara.

Duhet aktivizuar manualisht **Virtualization Technology (VTx)** në BIOS të HP-së. Udhëzimi dhe burimet zyrtare janë te `docs/BIOS_VIRTUALIZATION.md`. Rinisi e zakonshme Windows nuk mjafton për këtë cilësim. Nga ky sesion nuk ndryshojmë firmware-n.

Pas aktivizimit, kontrollojmë përsëri firmware flag dhe instalimin Ubuntu. Pas boot-it Linux ekzekutojmë `setup-wsl.sh`, `check-wsl.sh` dhe provën Anchor/IDL nga `env-anchor/README.md`. Skriptet dhe projekti janë të përgatitur, jo të verifikuar në një sesion Linux funksional. Kjo është pika e rifillimit.

Më pas implementohen helper-at e bitmap-it dhe programi me initialize market/registry/shard. Prepare dhe financimi bëhen për tetë shard-et fillestare; finalizimi hap tregun me maskë `0xff`. Zgjerimi aktivizon gradualisht shard-e të tjera deri në 56, pasi verifikohen fondet dhe të drejtat LP. Skema dinamike mbetet plan, jo veçori e kodit aktual.

## Ende të parealizuara

- Programi AMM Solana, build Anchor/IDL, ekzekutimi i programit provues në validator, transaksioni v1 real dhe deployment devnet.
- CLMM i plotë: tick crossing, fixed-point price/liquidity dhe kufij intervali.
- Pozicionet LP, kontabiliteti i detyrimeve dhe transferimet SPL.
- Router-i dhe benchmark-u i rrjetit.
- Huadhënia, oracle, interesi dhe likuidimi.
- Split/merge dinamik dhe auditimi.

Modeli aktual përdor rezerva virtuale constant-product, tarifa të ndara dhe rrumbullakim integer. Nuk është provë e aftësisë paguese të vault-eve reale dhe nuk është implementim i plotë i një intervali CLMM me kufij. Programi `env-smoke` është vetëm ping/pong pa fonde dhe pa gjendje. Asnjë matje TPS, CU ose vonese onchain nuk është bërë.

## Si riprodhohen kontrollet

```powershell
Set-Location 'C:\Users\Perdorues\Desktop\1\sharded_amm'
.\scripts\preflight.ps1
.\scripts\check.ps1
```

Dokumentet e sigurisë dhe pranimit dallojnë kontrollet që ekzistojnë në model nga ato që ende kërkojnë implementim. Kalimi i testeve lokale nuk mbyll portat e programit onchain.
