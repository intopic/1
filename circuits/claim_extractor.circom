pragma circom 2.1.6;

include "circomlib/circuits/bitify.circom";

// Numër nga bajte big-endian (çdo bajt duhet të jetë kontrolluar < 256 më parë).
template BytesToNumBE(len) {
    signal input in[len];
    signal output out;
    var acc = 0;
    for (var i = 0; i < len; i++) {
        acc = acc * 256 + in[i];
    }
    out <== acc;
}

// Nxjerr fushat nga mesazhi kanonik i atestimit (128 bajte).
// Formati: crates/zktls-prover/src/attestation.rs
//   0..8 magic "ZKTLSv01" | 8..16 value | 16..24 timestamp | 24..32 claim_key_id
//   32..64 wallet | 64..96 payload_digest | 96..128 nonce
template ClaimExtractor() {
    signal input msg[128];
    signal output bits[1024];      // MSB-first për Sha256(1024)
    signal output value;
    signal output timestamp;
    signal output claim_key_id;
    signal output wallet_hi;       // bajtet 32..48 si numër 128-bit
    signal output wallet_lo;       // bajtet 48..64

    // kontrolli i intervalit të bajteve + zbërthimi në bit
    component b2b[128];
    for (var i = 0; i < 128; i++) {
        b2b[i] = Num2Bits(8);
        b2b[i].in <== msg[i];
        for (var j = 0; j < 8; j++) {
            bits[i * 8 + j] <== b2b[i].out[7 - j];
        }
    }

    var magic[8] = [90, 75, 84, 76, 83, 118, 48, 49]; // "ZKTLSv01"
    for (var i = 0; i < 8; i++) {
        msg[i] === magic[i];
    }

    component v = BytesToNumBE(8);
    component t = BytesToNumBE(8);
    component c = BytesToNumBE(8);
    for (var i = 0; i < 8; i++) {
        v.in[i] <== msg[8 + i];
        t.in[i] <== msg[16 + i];
        c.in[i] <== msg[24 + i];
    }
    component hi = BytesToNumBE(16);
    component lo = BytesToNumBE(16);
    for (var i = 0; i < 16; i++) {
        hi.in[i] <== msg[32 + i];
        lo.in[i] <== msg[48 + i];
    }
    value <== v.out;
    timestamp <== t.out;
    claim_key_id <== c.out;
    wallet_hi <== hi.out;
    wallet_lo <== lo.out;
}
