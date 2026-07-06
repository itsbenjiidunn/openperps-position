//! OpenPerps Position: wallet-native, bearer-instrument perp positions for
//! ANY OpenPerps market.
//!
//! This program owns NO market risk logic. The Percolator engine (formally
//! verified upstream) prices, margins, and liquidates; the OpenPerps wrapper
//! owns the market accounts. This program only decides who holds a position's
//! keys, and the answer is: whoever holds its NFT.
//!
//! - A position is opened straight from the holder's wallet in ONE
//!   transaction: no prior deposit, no account setup, no allowlist. The
//!   collateral rides inside the position.
//! - The position is owned by a `[b"position", nft_mint]` PDA whose only
//!   credential is a 1-of-1 Token-2022 NFT with its metadata stored inside
//!   the mint account itself. The wallet that holds the NFT holds the trade.
//! - Burn-to-close flattens, settles, pays principal plus PnL to the holder,
//!   burns the NFT, closes every account the position ever created, and
//!   returns every lamport of rent. It needs nothing but this program and the
//!   market's OpenPerps deployment: no website, keeper, or market creator.
//! - The binding to the OpenPerps deployment is the market account's runtime
//!   owner, which the Solana runtime enforces and nobody can fake.
#![cfg_attr(target_os = "solana", no_std)]

pub mod error;
pub mod instruction;
pub mod opcpi;
pub mod processor;
pub mod state;

// The pinocchio entrypoint installs an allocator and panic handler that only
// make sense (and only compile) on the SBF target; host builds and unit tests
// stay on std. Mirrors the upstream OpenPerps gating.
#[cfg(all(target_os = "solana", not(feature = "no-entrypoint")))]
mod entrypoint;

pub use processor::process_instruction;
