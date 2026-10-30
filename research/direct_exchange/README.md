# Shkëmbim direkt: studim me teste dhe prova CI

Studim i izoluar, pa ndryshim në programet AMM të këtij repository.
Nuk është kontratë prodhimi dhe nuk pretendon paralelizëm ose siguri8x.

**CI kaloi:** https://github.com/intopic/1/actions/runs/40983318078 .
31 teste modeli, wire/signatures dhe shlyerja reale SPL në LiteSVM kaluan.
Tetë shkëmbimet reale përdorën12,293 CU secili. Log-et dhe rezultatet janë te
artifact-i direct-exchange-evidence. Ekzekutimi SVM është serial; matja e
paralelizmit të scheduler-it mbetet e hapur.

## Tri nivele prove

1. **Model integer:** ruajtje tokenësh, autorizim abstrakt, minimumet,
   nonce/expiry dhe rollback i modelit.31 teste dhe krahasim algoritmesh
   kundrejt optimumit ekzakt në96 grafe të vogla sintetike.
2. **Mesazhe Solana:**24 transaksione me dy TransferChecked dhe nënshkrime
   reale; derivim i read/write sets nga header-i. Tetë transaksione të ndara
   janë pa konflikt; payer ose source i përbashkët e ul maksimumin në1.
3. **SBF real në Linux LiteSVM:** ekzekuton classic SPL Token ELF të pin-uar,
   kontrollon balances, supply, rollback, replay, mungesë fondesh dhe firma.
   Kjo provë është SERIALE: nuk mat scheduler parallelism, TPS apo latency.
4. **Validator real lokal në Linux:** validator_probe.py dërgon ngarkesa me
   të njëjtat3 firma dhe584 bytes: accounts të pavarura, payer të përbashkët,
   ose source të përbashkët me payer-a të ndarë. Ka30 përsëritje nga8 tx për
   skenar (720 matje), plus48 warmup dhe kontroll të rollback. Rendi ndërmjet
   skenarëve përzihet me seed të deklaruar. Regjistrohen signature, timestamps,
   slot, status, CU, fee dhe balances. Kjo mat landing/confirmation në ngarkesë
   të ulët; nuk është peak TPS, provë worker-overlap ose garanci për8x.

Fixture-t mint/token account ngarkohen me set_account. Nuk kemi implementuar
kontratë të re për policy, order intents, cancellation ose receipts onchain.
Nënshkrimet reale autorizojnë të gjithë transaksionin atomik, jo një protokoll
të ri urdhrash të qëndrueshëm. Pa kundërpalë të financuar nuk ka shkëmbim.

## Ekzekutim lokal / Linux CI

Nga kjo dosje, me Python3.12:

```bash
python -m pip install -r requirements-protocol.txt
mkdir -p results
python -B -m unittest discover -s tests -v
python -B run_study.py
python -B protocol_probe.py
python -B runtime_probe.py
python -B ci_summary.py
# Me Agave4.3.2 të verifikuar në PATH, Linux dhe porte18899/18900 të lira:
python -B validator_probe.py
```

runtime_probe.py kërkon një solders Linux wheel me LiteSVM; import failure
është dështim i provës, jo test i kaluar ose i kapërcyer. Windows wire probe
funksionon, por LiteSVM mungonte në wheel-in Windows gjatë provës fillestare.

Workflow: `.github/workflows/direct-exchange-study.yml`. Ekzekutohet për PR
të degës research/direct-exchange-study. Artifact-i direct-exchange-evidence
ruan rezultatet e sapogjeneruara, log-et, varësitë dhe hash-et e source/ELF.
Raporti krijohet vetëm kur të tri nivelet kalojnë; rezultatet historike nuk
përdoren për t'i dhënë PASS ekzekutimit të ri. Nuk ka merge/deployment.

## Lexime

- MATH_SPEC.md: ekuacionet, supozimet dhe kundërshembujt.
- SOURCES.md: burimet biologjike dhe të sistemeve të shpërndara.
- LOCAL_STUDY_2026-10-30.md / reference_results/: snapshot i provës fillestare
  Windows; jo rezultati i CI-së së re.31 teste modeli dhe wire checks kaluan,
  por runtime-i ishte i bllokuar në atë mjedis.
- RUNTIME_PLAN.md: prova e ardhshme me validator scheduler dhe baseline të
  krahasueshëm. Ky gate nuk plotësohet nga LiteSVM seriale.

Lidhja biologji→independent set është e njohur. Suksesi i8 transaksioneve të
pavarura nuk është shpikje e re: duhet provuar vlera shtesë e algoritmit mbi
scheduler-in e Solana-s dhe heuristikat e thjeshta, me ngarkesa reale.
