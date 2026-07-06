/// Instruction builders. Byte layouts and account orders mirror
/// `program/src/instruction.rs` exactly; the Rust unit tests pin the upstream
/// encodings against the exact OpenPerps revision this program targets.

import { PublicKey, SystemProgram, TransactionInstruction } from "@solana/web3.js";

import { ATA_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID } from "./pda.js";

export enum Tag {
  OpenPosition = 0,
  ClosePosition = 1,
}

function writeU32LE(buf: Uint8Array, offset: number, value: number): void {
  new DataView(buf.buffer, buf.byteOffset).setUint32(offset, value, true);
}

function writeU64LE(buf: Uint8Array, offset: number, value: bigint): void {
  new DataView(buf.buffer, buf.byteOffset).setBigUint64(offset, value, true);
}

function writeU128LE(buf: Uint8Array, offset: number, value: bigint): void {
  const dv = new DataView(buf.buffer, buf.byteOffset);
  dv.setBigUint64(offset, value & 0xffffffffffffffffn, true);
  dv.setBigUint64(offset + 8, value >> 64n, true);
}

export function encodeOpenPosition(args: {
  side: number;
  assetIndex: number;
  sizeQ: bigint;
  execPrice: bigint;
  feeBps: bigint;
  depositAmount: bigint;
  positionBump: number;
  portfolioBump: number;
}): Buffer {
  const d = new Uint8Array(1 + 1 + 4 + 16 + 8 + 8 + 16 + 1 + 1);
  d[0] = Tag.OpenPosition;
  d[1] = args.side;
  writeU32LE(d, 2, args.assetIndex);
  writeU128LE(d, 6, args.sizeQ);
  writeU64LE(d, 22, args.execPrice);
  writeU64LE(d, 30, args.feeBps);
  writeU128LE(d, 38, args.depositAmount);
  d[54] = args.positionBump;
  d[55] = args.portfolioBump;
  return Buffer.from(d);
}

export function encodeClosePosition(args: {
  side: number;
  assetIndex: number;
  sizeQ: bigint;
  execPrice: bigint;
  feeBps: bigint;
  withdrawAmount: bigint;
  positionBump: number;
}): Buffer {
  const d = new Uint8Array(1 + 1 + 4 + 16 + 8 + 8 + 16 + 1);
  d[0] = Tag.ClosePosition;
  d[1] = args.side;
  writeU32LE(d, 2, args.assetIndex);
  writeU128LE(d, 6, args.sizeQ);
  writeU64LE(d, 22, args.execPrice);
  writeU64LE(d, 30, args.feeBps);
  writeU128LE(d, 38, args.withdrawAmount);
  d[54] = args.positionBump;
  return Buffer.from(d);
}

/// The associated-token-account create instruction (idempotent), hand-built
/// so the SDK has no dependency beyond web3.js.
export function createAtaIdempotentIx(args: {
  payer: PublicKey;
  ataAddress: PublicKey;
  owner: PublicKey;
  mint: PublicKey;
  tokenProgram?: PublicKey;
}): TransactionInstruction {
  return {
    programId: ATA_PROGRAM_ID,
    keys: [
      { pubkey: args.payer, isSigner: true, isWritable: true },
      { pubkey: args.ataAddress, isSigner: false, isWritable: true },
      { pubkey: args.owner, isSigner: false, isWritable: false },
      { pubkey: args.mint, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      { pubkey: args.tokenProgram ?? TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    ],
    data: Buffer.from([1]),
  } as TransactionInstruction;
}

export function openPositionIx(args: {
  positionProgram: PublicKey;
  market: PublicKey;
  housePortfolio: PublicKey;
  portfolio: PublicKey;
  position: PublicKey;
  nftMint: PublicKey;
  holder: PublicKey;
  holderNftAccount: PublicKey;
  positionToken: PublicKey;
  vaultToken: PublicKey;
  openperpsProgram: PublicKey;
  houseCap: PublicKey;
  feeConfig: PublicKey;
  riskConfig: PublicKey;
  side: number;
  assetIndex: number;
  sizeQ: bigint;
  execPrice: bigint;
  feeBps: bigint;
  depositAmount: bigint;
  positionBump: number;
  portfolioBump: number;
}): TransactionInstruction {
  return new TransactionInstruction({
    programId: args.positionProgram,
    keys: [
      { pubkey: args.market, isSigner: false, isWritable: true },
      { pubkey: args.housePortfolio, isSigner: false, isWritable: true },
      { pubkey: args.portfolio, isSigner: false, isWritable: true },
      { pubkey: args.position, isSigner: false, isWritable: true },
      { pubkey: args.nftMint, isSigner: false, isWritable: true },
      { pubkey: args.holder, isSigner: true, isWritable: true },
      { pubkey: args.holderNftAccount, isSigner: false, isWritable: true },
      { pubkey: args.positionToken, isSigner: false, isWritable: true },
      { pubkey: args.vaultToken, isSigner: false, isWritable: true },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_2022_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      { pubkey: args.openperpsProgram, isSigner: false, isWritable: false },
      { pubkey: args.houseCap, isSigner: false, isWritable: false },
      { pubkey: args.feeConfig, isSigner: false, isWritable: false },
      { pubkey: args.riskConfig, isSigner: false, isWritable: false },
    ],
    data: encodeOpenPosition(args),
  });
}

export function closePositionIx(args: {
  positionProgram: PublicKey;
  market: PublicKey;
  housePortfolio: PublicKey;
  portfolio: PublicKey;
  position: PublicKey;
  nftMint: PublicKey;
  holder: PublicKey;
  holderNftAccount: PublicKey;
  positionToken: PublicKey;
  vaultToken: PublicKey;
  holderToken: PublicKey;
  openperpsProgram: PublicKey;
  houseCap: PublicKey;
  feeConfig: PublicKey;
  riskConfig: PublicKey;
  side: number;
  assetIndex: number;
  sizeQ: bigint;
  execPrice: bigint;
  feeBps: bigint;
  withdrawAmount: bigint;
  positionBump: number;
}): TransactionInstruction {
  return new TransactionInstruction({
    programId: args.positionProgram,
    keys: [
      { pubkey: args.market, isSigner: false, isWritable: true },
      { pubkey: args.housePortfolio, isSigner: false, isWritable: true },
      { pubkey: args.portfolio, isSigner: false, isWritable: true },
      { pubkey: args.position, isSigner: false, isWritable: true },
      { pubkey: args.nftMint, isSigner: false, isWritable: true },
      { pubkey: args.holder, isSigner: true, isWritable: true },
      { pubkey: args.holderNftAccount, isSigner: false, isWritable: true },
      { pubkey: args.positionToken, isSigner: false, isWritable: true },
      { pubkey: args.vaultToken, isSigner: false, isWritable: true },
      { pubkey: args.holderToken, isSigner: false, isWritable: true },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_2022_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: args.openperpsProgram, isSigner: false, isWritable: false },
      { pubkey: args.houseCap, isSigner: false, isWritable: false },
      { pubkey: args.feeConfig, isSigner: false, isWritable: false },
      { pubkey: args.riskConfig, isSigner: false, isWritable: false },
    ],
    data: encodeClosePosition(args),
  });
}
