# Studimi fillestar: shkëmbim direkt pa pool

Data e studimit: 30 tetor 2026. Ky është studim i veçantë nga projekti AMM
i ndalur. Nuk u ndryshua kontrata ekzistuese; nuk u përdorën GitHub Actions,
deployment, mainnet, devnet apo fonde reale. Nuk është kontratë prodhimi.

## Çfarë u provua dhe çfarë mbetet

- 31 teste modeli kaluan. Përfshijnë 500 shkëmbime me sasi integer të
  rastësishme, të24 renditjet e4 shkëmbimeve të pavarura,112 grafe të vogla
  dhe kundërshembuj për gabime ekonomike/planifikimi.
- Në model, 8 shkëmbime me llogari të ndara ruajnë tokenët dhe kanë0 konflikte.
- Me pagues tarifash të përbashkët, treasury writable të përbashkët ose
  source token account të përbashkët, maksimumi pa konflikt është1.
- Katër grupe burimesh të përbashkëta, me2 kandidatë për grup, japin maksimum4.
- Kur dy urdhra me nonce të ndryshme kërkojnë të shpenzojnë të njëjtat fonde
  të pamjaftueshme, maksimumi njëri kalon; tjetri refuzohet pa ndryshim gjendjeje.
- Krahasimi i algoritmeve përdori96 grafe sintetike me16 kandidatë:32 seed-e
  për secilin probabilitet lidhjeje0.1,0.3,0.7. Optimumet u gjetën me një
  algoritëm ekzakt për këto raste të vogla, jo me etiketim nga heuristika.
- U ndërtuan24 transaksione reale në formatin Solana, me56 nënshkrime të
  verifikuara Ed25519. Secili përmban2 instruksione classic SPL TransferChecked.
  Ndryshimi i shumës pa rinënshkrim dhe mungesa e nënshkrimeve u refuzuan.
- Mesazhet reale konfirmojnë maksimum8/1/1 kandidatë pa konflikt për:
  llogari të ndara / payer të përbashkët / source të përbashkët me payer-a të ndarë.
  Madhësitë ishin488 bytes ose584 bytes me sponsor të tretë.
- ZERO transaksione u ekzekutuan në validator ose LiteSVM. Startup-i i
  validator-it dështoi dy herë gjatë unpack të genesis-it në Windows; LiteSVM
  mungon në native module të wheel-it Windows. Të dy proceset dolën.
- NUK është matur TPS, CU apo përshpejtim8x; nuk është provuar siguri më e
  fortë se protokollet ekzistuese ose një shpikje e re.

## Rezultatet e përzgjedhjes

Përqindja është mesatarja e raportit madhësi e përzgjedhur / optimumi i
grafit. Nuk është probabilitet sigurie, fitim tregtar ose throughput onchain.
Kohët janë matje indikative të një ekzekutimi Python në këtë Windows, përfshirë
llogaritjen e score/projection, por jo ndërtimin e grafit, nënshkrimet apo rrjetin.

| Algoritmi | Mesatarja ndaj optimumit | Rasti më i dobët | Mediana ms |
|---|---:|---:|---:|
| first_fit | 82.45% | 42.86% | 0.0126 |
| degree_greedy | 97.47% | 75.00% | 0.0626 |
| local_inhibition | 85.68% | 50.00% | 0.0982 |
| sigmoid_projected | 97.85% | 75.00% | 9.6265 |
| competitive_projected | 96.32% | 50.00% | 17.5095 |

Metoda e thjeshtë degree_greedy është pikënisja më praktike në këtë provë.
Modeli sigmoid fitoi pak në cardinality mesatare, por kushtoi shumë më tepër
kohë Python. Modeli ekologjik Lotka-Volterra nuk e tejkaloi degree_greedy në
mesatare dhe ishte më i ngadaltë. Nuk ekziston fitues universal nga ky kampion.

## Çfarë doli nga biologjia

U testuan:

1. Zgjedhje lokale e fituesit dhe inhibim i fqinjëve.
2. Dinamikë sigmoid e inhibimit lateral, me parametra eksperimentalë.
3. Dinamikë konkurruese Lotka-Volterra sipas Mooij-Kryven2024, me integrim
   numerik të kufizuar dhe pastaj kontroll diskret të detyrueshëm.

Ekuacioni ekologjik:

    dx_i/dt = x_i*(1-x_i-tau*sum_j A_ij*x_j), tau>1

x_i përfaqëson aktivitetin e kandidatit, jo tokenë financiarë. Përzgjedhja
finale kontrollon konfliktet pavarësisht score-it. Dy fqinjë me tau2 dhe
x1=x2=1/3 janë ekuilibër jobinar: prag0.25 zgjedh të dy (gabim), prag0.5
nuk zgjedh asnjërin. Kjo kundërshton përdorimin naiv të pragut biologjik.

Në një star me8 gjethe, first_fit mund të zgjedhë vetëm qendrën, ndërsa
optimumi zgjedh8 gjethe. Local inhibition gjithashtu mund të zgjedhë qendrën.
Maximal nuk do të thotë maximum. Modelet biologjike janë hipoteza për ranking,
jo mënyrë për të anashkaluar varësitë e fondeve ose account locks të Solana-s.

Lidhja biologji→independent set është njohur më herët; shiko SOURCES.md.
Ky studim nuk pretendon zbulim të një funksioni biologjik të panjohur.

## Kufijtë e sigurisë

Modeli përdor authorization-set abstrakt. Prova e nënshkrimeve është veçmas
mbi mesazhe të plota Solana; nuk implementon order intents/nonce/expiry onchain.
Nuk ka ende matcher ekonomik për urdhra arbitrarë, tarifim protokolli,
partial fills, cancellation, receipts lifecycle, oracle ose kundër-MEV proof.
Kandidatët e shkëmbimit në fixtures përputhen paraprakisht.

U kontrolluan: ruajtja e secilit token, bounds integer, minimumet e marrjes,
owner/mint/domain, replay, expiry, mungesa e autorizimit dhe rollback modeli
pas dështimit të këmbës së dytë. Asnjë prej tyre nuk zëvendëson auditin SVM.

8 kandidatë të pavarur janë kusht i nevojshëm për këtë provë paralelizmi,
jo garanci për8 punëtorë fizikë ose8x throughput. Transfertat e pavarura
tashmë mbështeten nga Solana; vlera shtesë e përzgjedhësit duhet matur kundrejt
scheduler-it ekzistues dhe baseline-ve të thjeshta.

## Riprodhimi

Nga kjo dosje, për modelin (vetëm Python standard library):

    python -X utf8 -B -m unittest discover -s tests -v
    python -X utf8 -B run_study.py

Për mesazhet/nënshkrimet, mjedisi i izoluar .venv ka requirements-protocol.txt:

    .venv\Scripts\python.exe -X utf8 -B protocol_probe.py

Dosja .venv nuk duhet dërguar në GitHub. Modeli dhe testet nuk varen prej saj.
Rezultatet raw ndodhen te results/model_results.json dhe protocol_results.json;
gabimet e runtime-it te runtime_attempts.json. Log-u i31 testeve: test_results.txt.
Specifikimi dhe provat e kushtëzuara: MATH_SPEC.md. Hapi i pambyllur:
RUNTIME_PLAN.md, me matje në një validator funksional Linux.
