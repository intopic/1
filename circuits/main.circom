pragma circom 2.1.6;

include "circomlib/circuits/poseidon.circom";
include "circomlib/circuits/comparators.circom";
include "circomlib/circuits/bitify.circom";
include "circomlib/circuits/sha256/sha256.circom";
include "@zk-email/circuits/utils/hash.circom";
include "./rsa_verify_2048.circom";
include "./claim_extractor.circom";

// Qarku kryesor zkTLS. Vërteton se:
//  1. noteri/banka me çelësin RSA-2048 (hash-i i të cilit është publik dhe në
//     listën e bardhë on-chain) ka nënshkruar mesazhin 128-bajtësh;
//  2. SHA-256 i mesazhit llogaritet brenda qarkut -> vlera është e lidhur me nënshkrimin;
//  3. value >= min_threshold, pa zbuluar value;
//  4. mesazhi është lëshuar për portofolin Solana të dhënë (anti-replay).
//
// Sinjalet publike (rendi në public.json): commitment, attested_at, claim_key_id,
// bank_public_key_hash, min_threshold, user_pubkey_hi, user_pubkey_lo.
// Asnjë e dhënë personale nuk del publike (rregulli 3).
template ZkWebProof() {
    // HYRJET PRIVATE
    signal input attestation_msg[128];   // mesazhi i nënshkruar (përmban bilancin)
    signal input user_secret_salt;       // salt për zotimin
    signal input rsa_signature[17];      // nënshkrimi RSA-2048 (17 x 121 bit)
    signal input rsa_modulus[17];        // moduli i bankës/noterit

    // HYRJET PUBLIKE
    signal input bank_public_key_hash;   // PoseidonLarge(121,17)(rsa_modulus)
    signal input min_threshold;          // p.sh. 10000
    signal input user_pubkey_hi;         // çelësi Solana, 16 bajtët e parë
    signal input user_pubkey_lo;         // 16 bajtët e fundit

    // DALJET PUBLIKE
    signal output commitment;            // Poseidon(value, salt)
    signal output attested_at;           // koha e atestimit (sekonda Unix)
    signal output claim_key_id;          // SHA-256(target_key)[0..8]

    component claim = ClaimExtractor();
    claim.msg <== attestation_msg;

    // 1. Kushti financiar: value >= min_threshold (të dyja < 2^64)
    component thr_bits = Num2Bits(64);
    thr_bits.in <== min_threshold;
    component is_ge = GreaterEqThan(64);
    is_ge.in[0] <== claim.value;
    is_ge.in[1] <== min_threshold;
    is_ge.out === 1;

    // 2. Zotimi kriptografik
    commitment <== Poseidon(2)([claim.value, user_secret_salt]);

    // 3. Integriteti: SHA-256(mesazhi) + nënshkrimi RSA-2048
    component sha = Sha256(1024);
    sha.in <== claim.bits;
    component rsa_verifier = VerifyRsa2048Pkcs1();
    rsa_verifier.hash_bits <== sha.out;
    rsa_verifier.signature <== rsa_signature;
    rsa_verifier.modulus <== rsa_modulus;

    // 4. Çelësi i bankës përputhet me hash-in publik (whitelist on-chain)
    signal key_hash <== PoseidonLarge(121, 17)(rsa_modulus);
    key_hash === bank_public_key_hash;

    // 5. Lidhja me portofolin
    claim.wallet_hi === user_pubkey_hi;
    claim.wallet_lo === user_pubkey_lo;

    attested_at <== claim.timestamp;
    claim_key_id <== claim.claim_key_id;
}

component main {public [bank_public_key_hash, min_threshold, user_pubkey_hi, user_pubkey_lo]} = ZkWebProof();
