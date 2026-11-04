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
//  3. min_value <= value <= max_value, pa zbuluar value
//     (bilanc >= X, ose data e lindjes <= sot − 18 vjet për moshën);
//  4. mesazhi është lëshuar për portofolin Solana të dhënë (anti-replay);
//  5. nullifier = Poseidon(account_tag, app_id): e njëjta llogari burimore jep të
//     njëjtin nullifier brenda një aplikacioni, pa u identifikuar (anti-Sybil).
//
// Sinjalet publike (rendi në public.json): commitment, attested_at, claim_key_id,
// nullifier, bank_public_key_hash, min_value, max_value, app_id,
// user_pubkey_hi, user_pubkey_lo.
// Asnjë e dhënë personale nuk del publike (rregulli 3).
template ZkWebProof() {
    // HYRJET PRIVATE
    signal input attestation_msg[128];   // mesazhi i nënshkruar (përmban bilancin)
    signal input user_secret_salt;       // salt për zotimin
    signal input rsa_signature[17];      // nënshkrimi RSA-2048 (17 x 121 bit)
    signal input rsa_modulus[17];        // moduli i bankës/noterit

    // HYRJET PUBLIKE
    signal input bank_public_key_hash;   // PoseidonLarge(121,17)(rsa_modulus)
    signal input min_value;              // p.sh. 10000 (bilanc) ose 0
    signal input max_value;              // p.sh. 2^64-1 ose 20081104 (data limit)
    signal input app_id;                 // identifikuesi i aplikacionit/kampanjës
    signal input user_pubkey_hi;         // çelësi Solana, 16 bajtët e parë
    signal input user_pubkey_lo;         // 16 bajtët e fundit

    // DALJET PUBLIKE
    signal output commitment;            // Poseidon(value, salt)
    signal output attested_at;           // koha e atestimit (sekonda Unix)
    signal output claim_key_id;          // SHA-256(target_key)[0..8]
    signal output nullifier;             // Poseidon(account_tag, app_id)

    component claim = ClaimExtractor();
    claim.msg <== attestation_msg;

    // 1. Kushti: min_value <= value <= max_value (të gjitha < 2^64)
    component min_bits = Num2Bits(64);
    min_bits.in <== min_value;
    component max_bits = Num2Bits(64);
    max_bits.in <== max_value;
    component ge_min = GreaterEqThan(64);
    ge_min.in[0] <== claim.value;
    ge_min.in[1] <== min_value;
    ge_min.out === 1;
    component le_max = LessEqThan(64);
    le_max.in[0] <== claim.value;
    le_max.in[1] <== max_value;
    le_max.out === 1;

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

    // 6. Nullifier anti-Sybil
    nullifier <== Poseidon(2)([claim.account_tag, app_id]);

    attested_at <== claim.timestamp;
    claim_key_id <== claim.claim_key_id;
}

component main {public [bank_public_key_hash, min_value, max_value, app_id, user_pubkey_hi, user_pubkey_lo]} = ZkWebProof();
