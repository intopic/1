pragma circom 2.1.6;

include "circomlib/circuits/poseidon.circom";
include "circomlib/circuits/comparators.circom";
include "circomlib/circuits/bitify.circom";
include "circomlib/circuits/eddsaposeidon.circom";
include "circomlib/circuits/babyjub.circom";
include "./claim_extractor.circom";

// v3: EdDSA-Poseidon over BabyJubjub authenticates all 128 message bytes.
// Range, commitment, wallet binding and per-application nullifier stay intact.
// SHA-256 of the masked HTTP response remains a signed field computed by the bridge.
//
// Sinjalet publike (rendi në public.json): commitment, attested_at, claim_key_id,
// nullifier, bank_public_key_hash, min_value, max_value, app_id,
// user_pubkey_hi, user_pubkey_lo.
// Asnjë e dhënë personale nuk del publike (rregulli 3).
template ZkWebProof() {
    // HYRJET PRIVATE
    signal input attestation_msg[128];   // mesazhi i nënshkruar (përmban bilancin)
    signal input user_secret_salt;       // salt për zotimin
    signal input notary_pubkey[2];
    signal input signature_r8[2];
    signal input signature_s;

    // HYRJET PUBLIKE
    signal input bank_public_key_hash;   // Poseidon(domain, Ax, Ay)
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

    // v3: Poseidon(domain, five BE limbs <= 31 bytes). All 128 bytes are
    // range checked by ClaimExtractor; no limb is reduced modulo the field.
    component msg_hash = Poseidon(6);
    msg_hash.inputs[0] <== 698893195705522144370390713378547228374396270131;
    component limb[5];
    for (var i = 0; i < 5; i++) {
        var len = i == 4 ? 4 : 31;
        limb[i] = BytesToNumBE(len);
        for (var j = 0; j < len; j++) {
            limb[i].in[j] <== attestation_msg[i * 31 + j];
        }
        msg_hash.inputs[i + 1] <== limb[i].out;
    }
    component key_check = BabyCheck();
    key_check.x <== notary_pubkey[0];
    key_check.y <== notary_pubkey[1];
    component r_check = BabyCheck();
    r_check.x <== signature_r8[0];
    r_check.y <== signature_r8[1];
    component signature = EdDSAPoseidonVerifier();
    signature.enabled <== 1;
    signature.Ax <== notary_pubkey[0];
    signature.Ay <== notary_pubkey[1];
    signature.R8x <== signature_r8[0];
    signature.R8y <== signature_r8[1];
    signature.S <== signature_s;
    signature.M <== msg_hash.out;

    // Domain-separated key hash; register a new whitelist with the new VK.
    signal key_hash <== Poseidon(3)([2730051545724696904865507452897579289774356019, notary_pubkey[0], notary_pubkey[1]]);
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
