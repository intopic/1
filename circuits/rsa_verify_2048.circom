pragma circom 2.1.6;

include "circomlib/circuits/bitify.circom";
include "@zk-email/circuits/lib/rsa.circom";

// MOD-06 në ZK: verifikon RSASSA-PKCS1-v1_5 / SHA-256 me RSA-2048 dhe e = 65537.
//
// Numrat e mëdhenj paraqiten me 17 copa x 121 bit (jo 32 x 64): produkti i dy
// copave (242 bit) plus shuma e 17 produkteve mbetet < p_BN254 (~2^254), çka
// lejon kontrollin e bartjeve pa tejmbushje të fushës. Aritmetika modulare
// (FpMul, RSAPad, BigLessThan) vjen nga @zk-email/circuits (e audituar).
template VerifyRsa2048Pkcs1() {
    var n = 121;
    var k = 17;

    signal input hash_bits[256];   // dalja e Sha256: indeksi 0 = MSB i bajtit të parë
    signal input signature[k];     // nënshkrimi, copa LE
    signal input modulus[k];       // moduli N, copa LE

    // hash-i (256 bit, BE) -> 3 copa 121-bit LE, si në EmailVerifier
    var msgChunks = (256 + n) \ n;
    component pack[msgChunks];
    for (var i = 0; i < msgChunks; i++) {
        pack[i] = Bits2Num(n);
    }
    for (var i = 0; i < 256; i++) {
        pack[i \ n].in[i % n] <== hash_bits[255 - i];
    }
    for (var i = 256; i < n * msgChunks; i++) {
        pack[i \ n].in[i % n] <== 0;
    }

    component rsa = RSAVerifier65537(n, k);
    for (var i = 0; i < msgChunks; i++) {
        rsa.message[i] <== pack[i].out;
    }
    for (var i = msgChunks; i < k; i++) {
        rsa.message[i] <== 0;
    }
    for (var i = 0; i < k; i++) {
        rsa.signature[i] <== signature[i];
        rsa.modulus[i] <== modulus[i];
    }
}
