#!/usr/bin/env node
/// The lifeboat CLI: a holder can read and redeem their position with no
/// frontend, no keeper, and no cooperation from anyone.
///
///   opp-position inspect <market> <nftMint>    read a position from chain
///   opp-position close   <market> <nftMint>    burn-to-close: flatten, settle,
///                                              pay out, burn, reclaim rent
///
/// Env: POSITION_PROGRAM_ID (required), RPC_URL (default devnet),
///      KEYPAIR (path; required for close), FEE_BPS (default 5).
/// The OpenPerps deployment and the collateral mint are discovered from the
/// market account itself; nothing else needs configuring.

import { readFileSync } from "node:fs";
import {
  ComputeBudgetProgram,
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import { decodePortfolioSummary, fetchMarketState, portfolioPda } from "@opp-oss/sdk";

import { marketQuoteMint, positionPda, ata } from "./pda.js";
import { buildClosePosition } from "./position.js";

function env(name: string, fallback?: string): string {
  const v = process.env[name] ?? fallback;
  if (!v) {
    console.error(`missing env ${name}`);
    process.exit(1);
  }
  return v;
}

function keypair(): Keypair {
  return Keypair.fromSecretKey(
    Uint8Array.from(JSON.parse(readFileSync(env("KEYPAIR"), "utf8")) as number[]),
  );
}

async function main(): Promise<void> {
  const [cmd, marketArg, nftArg] = process.argv.slice(2);
  if (!cmd || !marketArg || !nftArg || (cmd !== "inspect" && cmd !== "close")) {
    console.error("usage: opp-position <inspect|close> <market> <nftMint>");
    process.exit(1);
  }
  const connection = new Connection(env("RPC_URL", "https://api.devnet.solana.com"), "confirmed");
  const positionProgram = new PublicKey(env("POSITION_PROGRAM_ID"));
  const market = new PublicKey(marketArg);
  const nftMint = new PublicKey(nftArg);

  const marketAcc = await connection.getAccountInfo(market);
  if (!marketAcc) {
    console.error("market account not found");
    process.exit(1);
  }
  const openperpsProgram = marketAcc.owner;
  const mint = marketQuoteMint(marketAcc.data);

  const [position] = positionPda(positionProgram, nftMint);
  const [portfolio] = portfolioPda(openperpsProgram, position, market);
  const acc = await connection.getAccountInfo(portfolio);
  if (!acc) {
    console.error("no portfolio for this position (already closed?)");
    process.exit(1);
  }
  const summary = decodePortfolioSummary(acc.data);
  const leg = summary.positions[0];
  const { markPrice } = await fetchMarketState(connection, market, leg?.assetIndex ?? 0);
  console.log(`openperps program ${openperpsProgram.toBase58()}`);
  console.log(`position ${position.toBase58()}`);
  console.log(`  capital: ${summary.capital}`);
  console.log(`  pnl:     ${summary.pnl}`);
  console.log(`  legs:    ${JSON.stringify(summary.positions.map((p) => ({ asset: p.assetIndex, side: p.side, sizeQ: String(p.sizeQ) })))}`);
  console.log(`  mark:    ${markPrice}`);
  if (cmd === "inspect") return;

  const payer = keypair();
  const feeBps = BigInt(env("FEE_BPS", "5"));
  const sizeQ = leg?.sizeQ ?? 0n;
  const flipSide = leg ? 1 - leg.side : 0;
  // Withdraw everything the flatten leaves behind: capital minus the close
  // fee (engine formula: notional * fee_bps / 10^4, notional at the mark).
  const closeFee = leg ? (sizeQ * markPrice * feeBps) / (10_000n * 1_000_000n) : 0n;
  const withdraw = summary.capital > closeFee ? summary.capital - closeFee : 0n;
  const [holderToken] = ata(payer.publicKey, mint);
  const ixs = buildClosePosition({
    positionProgram,
    openperpsProgram,
    market,
    mint,
    nftMint,
    holder: payer.publicKey,
    holderToken,
    createHolderToken: true,
    side: flipSide,
    assetIndex: leg?.assetIndex ?? 0,
    sizeQ,
    execPrice: markPrice,
    feeBps,
    withdrawAmount: withdraw,
  });
  const sig = await sendAndConfirmTransaction(
    connection,
    new Transaction().add(ComputeBudgetProgram.setComputeUnitLimit({ units: 900_000 }), ...ixs),
    [payer],
  );
  console.log(`closed: ${sig}`);
  console.log(`paid out ${withdraw} atoms to ${holderToken.toBase58()}; NFT burned, all rent reclaimed`);
}

main().catch((e) => {
  console.error(e instanceof Error ? e.message : e);
  process.exit(1);
});
