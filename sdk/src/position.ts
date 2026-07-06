/// Open / close builders. Pure (no network).
///
/// Opening produces ONE transaction straight from the holder's wallet: create
/// the Token-2022 1-of-1 mint (close authority + metadata pointer aimed at
/// itself), stage the collateral under the position PDA, pre-fund the PDA
/// with the portfolio rent, then the program instruction that writes the
/// metadata INTO the mint, mints the NFT, opens the PDA-owned portfolio,
/// deposits, and places the order. No prior deposit or account exists.
///
/// Closing is burn-to-close: the program flattens, settles, withdraws to the
/// holder, burns the NFT, then closes every account the position ever created
/// (staging token account, the holder's NFT account, the mint itself) and
/// sweeps the PDA's lamports, so all rent returns to the holder. Flatten
/// parameters are caller-supplied and ENGINE-verified: if they do not fully
/// close the position, the withdraw refuses (open legs) and the whole
/// transaction reverts, so a wrong client can never lose the position, only
/// fail to close it.

import { PublicKey, SystemProgram, type TransactionInstruction, type Connection } from "@solana/web3.js";
import { portfolioPda, portfolioAccountSize, houseCapPda, feeConfigPda, riskConfigPda, housePortfolioPda, VAULT_SEED } from "@opp-oss/sdk";

import {
  positionPda,
  ata,
  POSITION_MINT_SPACE,
  POSITION_MINT_RENT_SPACE,
  TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
} from "./pda.js";
import { openPositionIx, closePositionIx, createAtaIdempotentIx } from "./instructions.js";

export const SIDE_LONG = 0;
export const SIDE_SHORT = 1;

/// Rent lamports an open needs. Fetch once, reuse for every open.
export interface OpenRents {
  nftMint: bigint;
  portfolio: bigint;
}

export async function fetchOpenRents(connection: Connection): Promise<OpenRents> {
  const [nftMint, portfolio] = await Promise.all([
    connection.getMinimumBalanceForRentExemption(POSITION_MINT_RENT_SPACE),
    connection.getMinimumBalanceForRentExemption(portfolioAccountSize(1)),
  ]);
  return { nftMint: BigInt(nftMint), portfolio: BigInt(portfolio) };
}

export interface OpenPositionInput {
  positionProgram: PublicKey;
  openperpsProgram: PublicKey;
  market: PublicKey;
  /// The market's collateral mint (read it with `marketQuoteMint`).
  mint: PublicKey;
  /// Fresh keypair public key for the position NFT (caller signs with it).
  nftMint: PublicKey;
  holder: PublicKey;
  /// The holder's token account holding the collateral to deposit.
  holderToken: PublicKey;
  side: number;
  assetIndex: number;
  sizeQ: bigint;
  execPrice: bigint;
  feeBps: bigint;
  depositAmount: bigint;
  rents: OpenRents;
}

export interface OpenPositionBuild {
  position: PublicKey;
  portfolio: PublicKey;
  holderNftAccount: PublicKey;
  positionToken: PublicKey;
  /// One transaction, in order. Sign with the holder AND the nftMint keypair.
  /// Prepend a ComputeBudget limit (~600k CU): the open runs the whole engine
  /// risk stack plus the metadata write in one instruction.
  instructions: TransactionInstruction[];
}

/// Token-2022 `InitializeMintCloseAuthority` (tag 25): the position PDA may
/// close the mint once its supply returns to zero (after burn-to-close),
/// returning the mint rent to the holder.
function initializeMintCloseAuthorityIx(nftMint: PublicKey, closeAuthority: PublicKey): TransactionInstruction {
  const data = new Uint8Array(1 + 1 + 32);
  data[0] = 25;
  data[1] = 1; // COption = Some
  data.set(closeAuthority.toBytes(), 2);
  return {
    programId: TOKEN_2022_PROGRAM_ID,
    keys: [{ pubkey: nftMint, isSigner: false, isWritable: true }],
    data: Buffer.from(data),
  } as TransactionInstruction;
}

/// Token-2022 `MetadataPointerExtension::Initialize` (tag 39, sub 0): the
/// metadata lives INSIDE the mint account itself. The pointer authority is
/// none (all zero), so the pointer is immutable.
function initializeMetadataPointerIx(nftMint: PublicKey): TransactionInstruction {
  const data = new Uint8Array(1 + 1 + 32 + 32);
  data[0] = 39;
  data[1] = 0; // sub-instruction: Initialize
  // bytes 2..34: authority OptionalNonZeroPubkey = none (zeros)
  data.set(nftMint.toBytes(), 34); // metadata_address = the mint itself
  return {
    programId: TOKEN_2022_PROGRAM_ID,
    keys: [{ pubkey: nftMint, isSigner: false, isWritable: true }],
    data: Buffer.from(data),
  } as TransactionInstruction;
}

/// `InitializeMint2` (tag 20) on Token-2022: decimals 0, mint authority = the
/// position PDA, no freeze authority.
function initializeNftMint2Ix(nftMint: PublicKey, mintAuthority: PublicKey): TransactionInstruction {
  const data = new Uint8Array(1 + 1 + 32 + 1);
  data[0] = 20;
  data[1] = 0; // decimals: a position NFT is indivisible
  data.set(mintAuthority.toBytes(), 2);
  data[34] = 0; // freeze authority: None (refused on-chain as well)
  return {
    programId: TOKEN_2022_PROGRAM_ID,
    keys: [{ pubkey: nftMint, isSigner: false, isWritable: true }],
    data: Buffer.from(data),
  } as TransactionInstruction;
}

/// SPL `Transfer` (tag 3), hand-built (v1 collateral staging).
function tokenTransferIx(
  source: PublicKey,
  destination: PublicKey,
  owner: PublicKey,
  amount: bigint,
): TransactionInstruction {
  const data = new Uint8Array(1 + 8);
  data[0] = 3;
  new DataView(data.buffer).setBigUint64(1, amount, true);
  return {
    programId: TOKEN_PROGRAM_ID,
    keys: [
      { pubkey: source, isSigner: false, isWritable: true },
      { pubkey: destination, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: true, isWritable: false },
    ],
    data: Buffer.from(data),
  } as TransactionInstruction;
}

export function buildOpenPosition(input: OpenPositionInput): OpenPositionBuild {
  const [position, positionBump] = positionPda(input.positionProgram, input.nftMint);
  const [portfolio, portfolioBump] = portfolioPda(input.openperpsProgram, position, input.market);
  const [holderNftAccount] = ata(input.holder, input.nftMint, TOKEN_2022_PROGRAM_ID);
  const [positionToken] = ata(position, input.mint);
  const [housePortfolio] = housePortfolioPda(input.openperpsProgram, input.market);
  const [vaultToken] = PublicKey.findProgramAddressSync(
    [VAULT_SEED, input.market.toBuffer()],
    input.openperpsProgram,
  );
  const [houseCap] = houseCapPda(input.openperpsProgram, input.market);
  const [feeConfig] = feeConfigPda(input.openperpsProgram, input.market);
  const [riskConfig] = riskConfigPda(input.openperpsProgram, input.market);

  const instructions: TransactionInstruction[] = [
    // The Token-2022 1-of-1 mint. The rent budget covers the metadata
    // reallocation the program performs when it writes into the mint.
    SystemProgram.createAccount({
      fromPubkey: input.holder,
      newAccountPubkey: input.nftMint,
      lamports: Number(input.rents.nftMint),
      space: POSITION_MINT_SPACE,
      programId: TOKEN_2022_PROGRAM_ID,
    }),
    initializeMintCloseAuthorityIx(input.nftMint, position),
    initializeMetadataPointerIx(input.nftMint),
    initializeNftMint2Ix(input.nftMint, position),
    // Where the NFT lands and where the collateral stages.
    createAtaIdempotentIx({
      payer: input.holder,
      ataAddress: holderNftAccount,
      owner: input.holder,
      mint: input.nftMint,
      tokenProgram: TOKEN_2022_PROGRAM_ID,
    }),
    createAtaIdempotentIx({
      payer: input.holder,
      ataAddress: positionToken,
      owner: position,
      mint: input.mint,
    }),
    // Stage the collateral under the position PDA and pre-fund the PDA with
    // the portfolio rent it pays inside the program instruction.
    tokenTransferIx(input.holderToken, positionToken, input.holder, input.depositAmount),
    SystemProgram.transfer({
      fromPubkey: input.holder,
      toPubkey: position,
      lamports: Number(input.rents.portfolio),
    }),
    // Write metadata into the mint, mint the NFT, open the portfolio,
    // deposit, place the order.
    openPositionIx({
      positionProgram: input.positionProgram,
      market: input.market,
      housePortfolio,
      portfolio,
      position,
      nftMint: input.nftMint,
      holder: input.holder,
      holderNftAccount,
      positionToken,
      vaultToken,
      openperpsProgram: input.openperpsProgram,
      houseCap,
      feeConfig,
      riskConfig,
      side: input.side,
      assetIndex: input.assetIndex,
      sizeQ: input.sizeQ,
      execPrice: input.execPrice,
      feeBps: input.feeBps,
      depositAmount: input.depositAmount,
      positionBump,
      portfolioBump,
    }),
  ];

  return { position, portfolio, holderNftAccount, positionToken, instructions };
}

export interface ClosePositionInput {
  positionProgram: PublicKey;
  openperpsProgram: PublicKey;
  market: PublicKey;
  mint: PublicKey;
  nftMint: PublicKey;
  holder: PublicKey;
  /// Destination for principal plus PnL. Created idempotently when
  /// `createHolderToken` is true.
  holderToken: PublicKey;
  createHolderToken?: boolean;
  /// Flatten leg: the exact opposite of the open (engine-verified; a partial
  /// flatten makes the withdraw, and therefore the whole close, revert).
  /// `sizeQ` 0 skips the flatten (already flat, e.g. liquidated).
  side: number;
  assetIndex: number;
  sizeQ: bigint;
  execPrice: bigint;
  feeBps: bigint;
  /// Full withdrawable capital after settle (read it with
  /// `decodePortfolioSummary` from @opp-oss/sdk). 0 skips the payout.
  withdrawAmount: bigint;
}

export function buildClosePosition(input: ClosePositionInput): TransactionInstruction[] {
  const [position, positionBump] = positionPda(input.positionProgram, input.nftMint);
  const [portfolio] = portfolioPda(input.openperpsProgram, position, input.market);
  const [holderNftAccount] = ata(input.holder, input.nftMint, TOKEN_2022_PROGRAM_ID);
  const [positionToken] = ata(position, input.mint);
  const [housePortfolio] = housePortfolioPda(input.openperpsProgram, input.market);
  const [vaultToken] = PublicKey.findProgramAddressSync(
    [VAULT_SEED, input.market.toBuffer()],
    input.openperpsProgram,
  );
  const [houseCap] = houseCapPda(input.openperpsProgram, input.market);
  const [feeConfig] = feeConfigPda(input.openperpsProgram, input.market);
  const [riskConfig] = riskConfigPda(input.openperpsProgram, input.market);

  const instructions: TransactionInstruction[] = [];
  if (input.createHolderToken) {
    instructions.push(
      createAtaIdempotentIx({
        payer: input.holder,
        ataAddress: input.holderToken,
        owner: input.holder,
        mint: input.mint,
      }),
    );
  }
  instructions.push(
    closePositionIx({
      positionProgram: input.positionProgram,
      market: input.market,
      housePortfolio,
      portfolio,
      position,
      nftMint: input.nftMint,
      holder: input.holder,
      holderNftAccount,
      positionToken,
      vaultToken,
      holderToken: input.holderToken,
      openperpsProgram: input.openperpsProgram,
      houseCap,
      feeConfig,
      riskConfig,
      side: input.side,
      assetIndex: input.assetIndex,
      sizeQ: input.sizeQ,
      execPrice: input.execPrice,
      feeBps: input.feeBps,
      withdrawAmount: input.withdrawAmount,
      positionBump,
    }),
  );
  return instructions;
}
