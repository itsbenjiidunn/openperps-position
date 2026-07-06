//! Instruction set: exactly two instructions, one pair. Open a position as a
//! bearer NFT straight from the holder's wallet, and burn it to redeem. Both
//! work against ANY OpenPerps market; the deployment is identified by the
//! market account's runtime owner, which nobody can fake.
//!
//! Every payload is a fixed little-endian layout, unpacked with the same
//! offset-exact style as the upstream OpenPerps program.

use crate::error::PositionError;

pub mod tag {
    pub const OPEN_POSITION: u8 = 0;
    pub const CLOSE_POSITION: u8 = 1;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionInstruction {
    /// Open a position as a bearer NFT, in one instruction: verify the fresh
    /// Token-2022 1-of-1 mint (extension whitelist), write the metadata INTO
    /// the mint, mint the NFT to the holder, fix the supply, create the
    /// PDA-owned portfolio, deposit the collateral (staged from the holder's
    /// wallet in the same transaction), place the order. After this
    /// instruction the NFT is the position; the wallet that holds it holds
    /// the trade. No prior deposit or account exists or is needed.
    ///
    /// Accounts:
    ///   0. `[writable]`         market account (an initialized OpenPerps
    ///                            market; its runtime OWNER pins the deployment)
    ///   1. `[writable]`         house portfolio PDA of that deployment
    ///   2. `[writable]`         portfolio PDA
    ///                            `[PORTFOLIO_SEED, position, market]`
    ///   3. `[writable]`         position PDA `[POSITION_SEED, nft_mint]`
    ///                            (pre-funded with the portfolio rent)
    ///   4. `[writable]`         position NFT mint (fresh Token-2022,
    ///                            authority = position PDA)
    ///   5. `[signer, writable]` holder
    ///   6. `[writable]`         holder's NFT token account (Token-2022)
    ///   7. `[writable]`         collateral staging account (owned by the
    ///                            position PDA, pre-funded, collateral mint)
    ///   8. `[writable]`         vault token account
    ///   9. `[]`                 SPL Token program (v1, collateral side)
    ///  10. `[]`                 Token-2022 program (NFT side)
    ///  11. `[]`                 system program
    ///  12. `[]`                 the OpenPerps program (== market owner)
    ///  13. `[]`                 house-cap PDA
    ///  14. `[]`                 fee-config PDA
    ///  15. `[]`                 risk-config PDA
    OpenPosition {
        side: u8,
        asset_index: u32,
        size_q: u128,
        exec_price: u64,
        fee_bps: u64,
        deposit_amount: u128,
        position_bump: u8,
        portfolio_bump: u8,
    },
    /// Burn-to-close: prove holdership of the NFT, flatten the position
    /// (engine-verified: the upstream `Withdraw` refuses while any leg is
    /// open, so a wrong flatten reverts the whole close), settle PnL, pay
    /// principal plus PnL to the holder, burn the NFT, close every account
    /// the position ever created (staging account, NFT account, the mint
    /// itself), and sweep the PDA's lamports. Every lamport of rent returns
    /// to the holder. Works with nothing but this program and the market's
    /// OpenPerps deployment: no website, keeper, or market creator required.
    ///
    /// `size_q == 0` skips the flatten (already flat, e.g. liquidated);
    /// `withdraw_amount == 0` skips the payout (nothing left).
    ///
    /// Accounts:
    ///   0. `[writable]`         market account
    ///   1. `[writable]`         house portfolio PDA
    ///   2. `[writable]`         portfolio PDA
    ///   3. `[writable]`         position PDA `[POSITION_SEED, nft_mint]`
    ///   4. `[writable]`         position NFT mint
    ///   5. `[signer, writable]` holder (receives payout + all rent)
    ///   6. `[writable]`         holder's NFT token account (burned + closed)
    ///   7. `[writable]`         collateral staging account (drained + closed)
    ///   8. `[writable]`         vault token account
    ///   9. `[writable]`         holder's collateral token account (payout)
    ///  10. `[]`                 SPL Token program (v1)
    ///  11. `[]`                 Token-2022 program
    ///  12. `[]`                 the OpenPerps program (== market owner)
    ///  13. `[]`                 house-cap PDA
    ///  14. `[]`                 fee-config PDA
    ///  15. `[]`                 risk-config PDA
    ClosePosition {
        side: u8,
        asset_index: u32,
        size_q: u128,
        exec_price: u64,
        fee_bps: u64,
        withdraw_amount: u128,
        position_bump: u8,
    },
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, PositionError> {
    data.get(offset..offset + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(PositionError::InvalidInstructionData)
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, PositionError> {
    data.get(offset..offset + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(PositionError::InvalidInstructionData)
}

fn read_u128(data: &[u8], offset: usize) -> Result<u128, PositionError> {
    data.get(offset..offset + 16)
        .and_then(|s| s.try_into().ok())
        .map(u128::from_le_bytes)
        .ok_or(PositionError::InvalidInstructionData)
}

fn read_u8(data: &[u8], offset: usize) -> Result<u8, PositionError> {
    data.get(offset)
        .copied()
        .ok_or(PositionError::InvalidInstructionData)
}

impl PositionInstruction {
    pub fn unpack(data: &[u8]) -> Result<Self, PositionError> {
        let (&tag, rest) = data
            .split_first()
            .ok_or(PositionError::InvalidInstruction)?;
        match tag {
            tag::OPEN_POSITION => Ok(Self::OpenPosition {
                side: read_u8(rest, 0)?,
                asset_index: read_u32(rest, 1)?,
                size_q: read_u128(rest, 5)?,
                exec_price: read_u64(rest, 21)?,
                fee_bps: read_u64(rest, 29)?,
                deposit_amount: read_u128(rest, 37)?,
                position_bump: read_u8(rest, 53)?,
                portfolio_bump: read_u8(rest, 54)?,
            }),
            tag::CLOSE_POSITION => Ok(Self::ClosePosition {
                side: read_u8(rest, 0)?,
                asset_index: read_u32(rest, 1)?,
                size_q: read_u128(rest, 5)?,
                exec_price: read_u64(rest, 21)?,
                fee_bps: read_u64(rest, 29)?,
                withdraw_amount: read_u128(rest, 37)?,
                position_bump: read_u8(rest, 53)?,
            }),
            _ => Err(PositionError::InvalidInstruction),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpack_open_position() {
        let mut data = vec![tag::OPEN_POSITION, 1];
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend_from_slice(&1_000_000u128.to_le_bytes());
        data.extend_from_slice(&123u64.to_le_bytes());
        data.extend_from_slice(&5u64.to_le_bytes());
        data.extend_from_slice(&777u128.to_le_bytes());
        data.push(250);
        data.push(249);
        assert_eq!(
            PositionInstruction::unpack(&data).unwrap(),
            PositionInstruction::OpenPosition {
                side: 1,
                asset_index: 3,
                size_q: 1_000_000,
                exec_price: 123,
                fee_bps: 5,
                deposit_amount: 777,
                position_bump: 250,
                portfolio_bump: 249,
            }
        );
    }

    #[test]
    fn unpack_close_position() {
        let mut data = vec![tag::CLOSE_POSITION, 0];
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&42u128.to_le_bytes());
        data.extend_from_slice(&100u64.to_le_bytes());
        data.extend_from_slice(&1u64.to_le_bytes());
        data.extend_from_slice(&55u128.to_le_bytes());
        data.push(248);
        assert_eq!(
            PositionInstruction::unpack(&data).unwrap(),
            PositionInstruction::ClosePosition {
                side: 0,
                asset_index: 0,
                size_q: 42,
                exec_price: 100,
                fee_bps: 1,
                withdraw_amount: 55,
                position_bump: 248,
            }
        );
    }

    #[test]
    fn truncated_payloads_rejected() {
        assert!(PositionInstruction::unpack(&[]).is_err());
        assert!(PositionInstruction::unpack(&[tag::OPEN_POSITION, 1, 2]).is_err());
        assert!(PositionInstruction::unpack(&[tag::CLOSE_POSITION]).is_err());
        assert!(PositionInstruction::unpack(&[99]).is_err());
    }
}
