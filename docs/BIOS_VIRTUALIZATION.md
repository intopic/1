# Aktivizimi i virtualizimit — HP ProBook 430 G4

> **Dokument historik (para implementimit).** Ruhet si kontekst. Kur ndryshon nga kodi ose nga dokumentet normative, vlejnë [numeric-spec.md](numeric-spec.md), [account-layout.md](account-layout.md), [tx-budget.md](tx-budget.md), [ADR-të](adr/README.md) dhe [STATUS.md](../STATUS.md). Indeksi: [docs/README.md](README.md).

Kontrolluar pas rinisjes më 28 tetor 2026. Modeli është HP ProBook 430 G4, procesori Intel Core i5-7200U. `VMMonitorModeExtensions=True`, `SecondLevelAddressTranslationExtensions=True`, por `VirtualizationFirmwareEnabled=False`.

WSL 3.0.3.0 funksionon. Instalimi Ubuntu arriti te regjistrimi, pastaj dështoi me `HCS_E_HYPERV_NOT_INSTALLED` dhe mesazhin që virtualizimi nuk është aktiv. Asnjë sesion Ubuntu nuk u nis; konfigurimi Linux/Anchor ende nuk është ekzekutuar.

## Hapi manual

1. Ruaj punën, fik ose rinis kompjuterin.
2. Shtyp disa herë **F10** menjëherë gjatë ndezjes për BIOS Setup. Nëse shfaqet Startup Menu, zgjidh BIOS Setup.
3. Te BIOS kërko **Advanced → System Options → Virtualization Technology (VTx)** dhe aktivizoje. Kjo rrugë është udhëzimi HP për business PCs; vendndodhja mund të ndryshojë me versionin e BIOS-it.
4. Ruaj ndryshimin me **F10 / Save Changes and Exit** dhe kthehu në Windows.
5. Kontrollo te **Task Manager → Performance → CPU → Virtualization** që të shfaqet **Enabled**.

Për këtë hap ndrysho vetëm VTx. Nëse opsioni mungon ose është i kyçur, duhet parë menyja konkrete/administrimi BIOS; nuk hamendësojmë një ndryshim tjetër firmware.

## Verifikimi pas kthimit në Windows

```powershell
Get-CimInstance Win32_Processor | Select-Object Name,VirtualizationFirmwareEnabled
wsl --list --verbose
```

Pasi firmware flag bëhet True, rifillojmë përfundimin e Ubuntu-s, setup Linux, build SBF, build Anchor/IDL dhe provën ping/pong në validator. Nëse del përsëri gabimi i hypervisor-it, duhet kontrolluar edhe gjendja e Virtual Machine Platform dhe hypervisor launch me privilegje Windows administrator; aktivizimi BIOS nuk quhet provë që çdo kusht tjetër tashmë është plotësuar.

## Burime zyrtare

- [HP — Enable Virtualization Technology in the BIOS](https://support.hp.com/us-en/document/ish_5637142-5637191-16)
- [Microsoft — WSL troubleshooting, virtualization and BIOS requirements](https://learn.microsoft.com/en-us/windows/wsl/troubleshooting)

Procesi dhe gabimet e plota janë përmbledhur në `environment-report.json`. Nuk u ndryshua BIOS dhe nuk u çaktivizua antivirusi.
