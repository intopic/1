# Modeli matematik kërkimor

Ky modul Rust pa varësi simulon një interval constant-product me rezerva virtuale. Nuk është kontratë Solana ose CLMM i plotë: nuk ka kufij intervali, kalim ticks, LP positions, vault-e reale, oracle apo likuidime. Rezervat virtuale nuk provojnë se një vault real mund të paguajë output-in.

API: `Band::new(x, y, fee_ppm)`, `quote_exact_in(direction, amount)` dhe `swap_exact_in(direction, amount)`. `Direction` është `XtoY` ose `YtoX`. Quote është fotografi e gjendjes; swap e rillogarit sipas gjendjes aktuale. Nuk pranon quote të vjetër si urdhër të garantuar. Kontrolli `min_out` mbetet detyrë e shtresës së ardhshme.

Për input bruto `a`, tarifa është `ceil(a * fee_ppm / 1_000_000)`, input neto `n = a - tarifa` dhe output `floor(y * n / (x + n))`. Rezerva hyrëse rritet me `n`; tarifa ruhet veçmas. Output-i ulet për rrumbullakim, tarifa rritet për rrumbullakim; te shumat shumë të vogla tregtia refuzohet. Ky trajtim i pluhurit numerik është model i thjeshtuar, jo garanci se kopjon saktësisht aritmetikën me liquidity fikse të një CLMM ekzistues.

Të gjitha input-et dhe rezervat kufizohen në `2^96 - 1`; tarifa është më e vogël se 100%. Shumëzimet përdorin `u128` me kontrolle. Edhe brenda kufirit, produkti i ndërmjetëm mund të tejkalojë `u128` dhe quote refuzohet. Nuk përdoret aritmetikë 256-bit. Gabimet nuk ndryshojnë gjendjen.

Testet kontrollojnë konservimin e input-it brenda modelit, ndarjen e tarifave, dy drejtimet, kufijtë, overflow, gjendjen pas gabimit, fragmentimin me të njëjtin kapital dhe ndryshimin mes quote-ve të vjetra dhe ekzekutimit vijues. Nuk matin scheduler-in Sealevel, TPS ose sigurinë financiare në rrjet.

Nga rrënja e workspace: `cargo test -p amm-model`.
