import { AnchorProvider } from "@coral-xyz/anchor";
import { Keypair, PublicKey, SystemProgram, Transaction, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { strict as assert } from "assert";

// A third party tries one-lamport griefing. Some validator configurations reject
// creation below rent exemption; in that case fund the minimum empty-account rent.
export async function prefund(provider: AnchorProvider, addresses: PublicKey[]) {
  const attacker = Keypair.generate();
  await provider.sendAndConfirm(new Transaction().add(SystemProgram.transfer({
    fromPubkey: provider.wallet.publicKey, toPubkey: attacker.publicKey, lamports: LAMPORTS_PER_SOL / 2,
  })), [], { commitment: "confirmed", preflightCommitment: "confirmed" });
  const minimum = await provider.connection.getMinimumBalanceForRentExemption(0);
  for (const toPubkey of addresses) {
    const transfer = (lamports: number) => provider.sendAndConfirm(new Transaction().add(SystemProgram.transfer({
      fromPubkey: attacker.publicKey, toPubkey, lamports,
    })), [attacker], { commitment: "confirmed", preflightCommitment: "confirmed" });
    try { await transfer(1); }
    catch (e) {
      if (!/rent/i.test(String(e))) throw e;
    }
    let account = await provider.connection.getAccountInfo(toPubkey, "confirmed");
    // A successful dust transfer may still be reclaimed by runtime rent rules.
    if (!account || account.lamports < minimum) {
      await transfer(minimum);
      account = await provider.connection.getAccountInfo(toPubkey, "confirmed");
    }
    assert(account && account.lamports >= minimum && account.data.length === 0,
      `prefund did not persist for ${toPubkey}: ${JSON.stringify(account)}`);
    assert(account.owner.equals(SystemProgram.programId));
  }
}
