<div align="center">

# OpenPerps Position

**Wallet-native, bearer-instrument perp positions for any OpenPerps market.**

Open a position straight from the wallet in one signature: no prior deposit,
no account setup, no allowlist. The position is a Token-2022 NFT whose
metadata lives inside the mint; whoever holds the NFT holds the trade, and
burning it redeems principal plus PnL with every lamport of rent returned.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Program: Pinocchio](https://img.shields.io/badge/program-pinocchio-9cf.svg)](./program)
[![Engine: Percolator v16](https://img.shields.io/badge/engine-percolator%20v16-2ee6c2.svg)](https://github.com/itsbenjiidunn/openperps-oss)
[![SDK: TypeScript](https://img.shields.io/badge/sdk-%40opp--oss%2Fposition-3178c6.svg)](./sdk)

</div>

---

## Overview

OpenPerps Position removes the account model from perp trading. On a
conventional venue a trader deposits into a protocol account through a
frontend, trades against that balance, and withdraws later; every step leans
on hosted infrastructure. Here the wallet is the account:

- **One-signature open.** A single transaction stages the collateral out of
  the holder's wallet, creates the position's isolated portfolio, places the
  order against the market's counterparty liquidity, and mints the position
  NFT to the holder. There is nothing to deposit into beforehand and nothing
  to remember afterwards; the collateral rides inside the position.
- **The NFT is the position.** Each position is owned by a
  `[b"position", nft_mint]` PDA whose only credential is a 1-of-1 Token-2022
  NFT. Its name (long or short) is written INTO the mint account with the
  TokenMetadata extension: no URI, no metadata service, nothing off-chain.
  Transfer the NFT and the position, with all its PnL, transfers with it.
- **Burn to redeem.** Burning the NFT flattens the position (engine-verified:
  the upstream withdraw refuses while legs are open, so a wrong flatten
  reverts atomically), settles PnL, pays principal plus PnL to the holder,
  closes every account the position ever created, and returns all rent.
  Redemption needs only this program and the market's OpenPerps deployment.
  No website, keeper, market creator, or LP cooperation is involved.
- **Any OpenPerps market.** The instruction binds to a deployment through the
  market account's runtime owner, which the Solana runtime sets and nobody
  can fake, and reads the collateral mint from the market's own header. There
  is no registry and no allowlist; if it is an OpenPerps market, positions
  work on it.
- **Isolated by construction.** Every position is its own engine portfolio.
  One position liquidating never touches another; a wallet's exposure is
  exactly the sum of the NFTs it chose to hold.

This program contains no market logic. The
[Percolator](https://github.com/itsbenjiidunn/openperps-oss/tree/main/crates/engine)
engine (formally verified upstream) prices, margins, and liquidates; the
[OpenPerps](https://github.com/itsbenjiidunn/openperps-oss) wrapper owns the
market accounts; this program only decides who holds a position's keys, and
the answer is: whoever holds its NFT.

Launching markets whose liquidity vests from an external lock platform is the
sibling repository, [simpleperps](https://github.com/itsbenjiidunn/simpleperps).
The two compose but neither requires the other.

## Position lifecycle

```
OpenPosition (one tx from the wallet)     ClosePosition (burn to redeem)
-------------------------------------     -------------------------------
create the T22 mint (client ixs)          verify: signer holds the NFT
verify the mint (extension whitelist)     flatten via PlaceOrder (CPI)
write metadata INTO the mint              settle PnL (CPI)
mint the 1-of-1 to the holder             withdraw principal + PnL
revoke the mint authority                    to the holder (CPI)
create the PDA-owned portfolio (CPI)      burn the NFT
deposit the staged collateral (CPI)       close staging + NFT account + mint
place the order (CPI)                     sweep PDA lamports to the holder
```

The mint is verified against an extension whitelist before anything else: it
must carry exactly `MintCloseAuthority` (the position PDA, so burn can
reclaim the mint rent) and `MetadataPointer` (aimed at the mint itself), no
freeze authority, and nothing more. A `PermanentDelegate`, `TransferHook`, or
`DefaultAccountState` extension is refused outright, so no third party can
ever seize, veto, or freeze a position NFT.

## Authorization model

| Actor | Authorized | Not authorized |
|---|---|---|
| **Holder** | open; transfer or sell the NFT; burn to redeem | anything affecting other positions |
| **Anyone** | read every position from chain data | any access to a position they do not hold |
| **No one** | | freezing or seizing a position NFT; a second unit of any position mint; signing as a position PDA outside these two instructions |

## Quickstart

```ts
import {
  buildOpenPosition, buildClosePosition, fetchOpenRents,
  marketQuoteMint, SIDE_LONG,
} from "@opp-oss/position";

// The deployment and collateral mint come from the market account itself.
const marketAcc = await connection.getAccountInfo(market);
const openperpsProgram = marketAcc.owner;
const mint = marketQuoteMint(marketAcc.data);

// 1) Open, straight from the wallet: one transaction, two signers
//    (the holder and the fresh NFT mint keypair).
const open = buildOpenPosition({
  positionProgram, openperpsProgram, market, mint,
  nftMint: nftKeypair.publicKey,
  holder: wallet.publicKey, holderToken,
  side: SIDE_LONG, assetIndex: 0,
  sizeQ: 5_000_000n, execPrice: markPrice, feeBps: 5n,
  depositAmount: 1_000_000n,
  rents: await fetchOpenRents(connection),
});

// 2) Redeem, from any client, indefinitely.
const close = buildClosePosition({
  positionProgram, openperpsProgram, market, mint,
  nftMint, holder: wallet.publicKey, holderToken, createHolderToken: true,
  side: 1 - SIDE_LONG, assetIndex: 0,
  sizeQ: 5_000_000n, execPrice: markPrice, feeBps: 5n,
  withdrawAmount: capitalAfterSettle,
});
```

Or with no code at all, the lifeboat CLI (no frontend, no configuration
beyond the program id; the deployment is discovered from the market account):

```
POSITION_PROGRAM_ID=... KEYPAIR=holder.json npx opp-position inspect <market> <nftMint>
POSITION_PROGRAM_ID=... KEYPAIR=holder.json npx opp-position close   <market> <nftMint>
```

## Repository layout

```
program/   on-chain program (Rust, Pinocchio, no_std), two instructions
           state.rs        raw readers: SPL fields, T22 mint whitelist,
                           OpenPerps market header (all pinned by host tests)
           opcpi.rs        typed re-encoders for the upstream instructions
           processor.rs    OpenPosition / ClosePosition
sdk/       @opp-oss/position (TypeScript): builders, rents, lifeboat CLI
```

## Status

Version 0.1.0, split out of the combined prototype whose end-to-end
suite passed on devnet (open, transfer-of-ownership semantics, burn-to-close
with exact payout and full rent reclaim, close-on-a-burned-program). This
standalone build binds to markets via the market account owner instead of a
launch record; host tests are green (Rust 9, SDK 4, including byte-parity
pins against the exact upstream revision), and a fresh devnet deployment +
end-to-end run of THIS binary is the explicit next step. Not audited; do not
put mainnet value behind it yet.

## License

Apache-2.0.
