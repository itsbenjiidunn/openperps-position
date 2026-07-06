/// Derivations and constants. One seed runs the whole system:
/// `[b"position", nft_mint]` is a position's owner PDA, so the NFT mint alone
/// re-derives everything about a position. Market-side PDAs come from
/// @opp-oss/sdk.

import { PublicKey } from "@solana/web3.js";

export const POSITION_SEED = new TextEncoder().encode("position");

/** SPL Token program (v1); the collateral side. */
export const TOKEN_PROGRAM_ID = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
/** SPL Token-2022 program; position NFTs live here so their metadata sits
 *  inside the mint account (no URI hosting) and the mint is closeable at burn
 *  (all rent returns to the holder). */
export const TOKEN_2022_PROGRAM_ID = new PublicKey("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
/** SPL Associated Token Account program. */
export const ATA_PROGRAM_ID = new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
/** Allocated size of a position NFT mint: base(165) + account-type(1) +
 *  MintCloseAuthority TLV(36) + MetadataPointer TLV(68). */
export const POSITION_MINT_SPACE = 270;
/** Rent budget (bytes) for the mint INCLUDING the Token-2022 metadata
 *  reallocation (TLV header + update authority + mint + name/symbol/uri). */
export const POSITION_MINT_RENT_SPACE = 460;

/** A position owner PDA, keyed solely by its position NFT mint. */
export function positionPda(
  positionProgram: PublicKey,
  nftMint: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [POSITION_SEED, nftMint.toBuffer()],
    positionProgram,
  );
}

/** Associated token account of `owner` for `mint`. The token program is part
 *  of the ATA derivation, so Token-2022 accounts pass it explicitly. Returns
 *  `[address, bump]`. */
export function ata(
  owner: PublicKey,
  mint: PublicKey,
  tokenProgram: PublicKey = TOKEN_PROGRAM_ID,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [owner.toBuffer(), tokenProgram.toBuffer(), mint.toBuffer()],
    ATA_PROGRAM_ID,
  );
}

/** Read the collateral mint out of a raw OpenPerps market account, mirroring
 *  the on-chain reader: discriminator "OPMARKET" + layout version 4 verified,
 *  `quote_mint` at offset 48. */
export function marketQuoteMint(marketData: Uint8Array): PublicKey {
  if (marketData.length < 80) throw new Error("not an OpenPerps market account (too small)");
  const disc = new TextDecoder().decode(marketData.subarray(0, 8));
  const version = new DataView(marketData.buffer, marketData.byteOffset).getUint32(8, true);
  if (disc !== "OPMARKET" || version !== 4) {
    throw new Error("not an initialized OpenPerps market of a supported layout version");
  }
  return new PublicKey(marketData.subarray(48, 80));
}
