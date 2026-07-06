//! No state accounts. A position is fully described by its NFT mint: the
//! owner of the OpenPerps portfolio is the `[POSITION_SEED, nft_mint]` PDA,
//! so `(nft_mint, market)` re-derives everything and there is nothing to
//! create, register, rent, or forget on this program's side.
//!
//! What lives here instead are the offset-exact raw readers this program
//! needs: SPL token account fields, the Token-2022 position-mint whitelist,
//! and the OpenPerps market wrapper header fields (collateral mint), each
//! pinned byte-for-byte against the exact upstream revision by host tests.

use crate::error::PositionError;

/// PDA seed for a position owner: `[POSITION_SEED, nft_mint]`.
pub const POSITION_SEED: &[u8] = b"position";
/// OpenPerps portfolio seed (`[PORTFOLIO_SEED, owner, market]` under the
/// OpenPerps program). Local copy, pinned to upstream by a host unit test.
pub const PORTFOLIO_SEED: &[u8] = b"portfolio";

fn read_32(buf: &[u8], off: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&buf[off..off + 32]);
    out
}

fn read_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}

// ---------- OpenPerps market wrapper header raw reads ----------
// The market account's runtime OWNER is the OpenPerps program (unfakeable);
// its wrapper header records the collateral mint. Local constants, pinned to
// the upstream layout by a host unit test.

/// Wrapper discriminator at offset 0.
pub const MARKET_DISCRIMINATOR: [u8; 8] = *b"OPMARKET";
/// Wrapper layout version at offset 8 (u32 LE) this build understands.
pub const MARKET_HEADER_VERSION: u32 = 4;
/// `quote_mint` (the collateral mint) at offset 48.
pub const MARKET_QUOTE_MINT_OFFSET: usize = 48;

/// Read the collateral mint out of an OpenPerps market account, verifying the
/// discriminator + layout version first so an account written by a different
/// layout reads as uninitialized instead of being silently mis-decoded.
pub fn market_quote_mint(data: &[u8]) -> Result<[u8; 32], PositionError> {
    if data.len() < MARKET_QUOTE_MINT_OFFSET + 32 {
        return Err(PositionError::UninitializedMarket);
    }
    if data[0..8] != MARKET_DISCRIMINATOR
        || u32::from_le_bytes(data[8..12].try_into().unwrap()) != MARKET_HEADER_VERSION
    {
        return Err(PositionError::UninitializedMarket);
    }
    Ok(read_32(data, MARKET_QUOTE_MINT_OFFSET))
}

// ---------- SPL token account raw reads ----------
// Minimal, offset-exact reads of the SPL Token layouts (shared by v1 and
// Token-2022). Callers verify the account owner program before trusting any
// field.

/// SPL TokenAccount: mint pubkey at offset 0.
pub fn token_account_mint(data: &[u8]) -> Result<[u8; 32], PositionError> {
    if data.len() < 72 {
        return Err(PositionError::InvalidAccountData);
    }
    Ok(read_32(data, 0))
}

/// SPL TokenAccount: owner pubkey at offset 32.
pub fn token_account_owner(data: &[u8]) -> Result<[u8; 32], PositionError> {
    if data.len() < 72 {
        return Err(PositionError::InvalidAccountData);
    }
    Ok(read_32(data, 32))
}

/// SPL TokenAccount: amount (u64 LE) at offset 64.
pub fn token_account_amount(data: &[u8]) -> Result<u64, PositionError> {
    if data.len() < 72 {
        return Err(PositionError::InvalidAccountData);
    }
    Ok(read_u64(data, 64))
}

// Token-2022 extension type ids (TLV `type` field).
const EXT_MINT_CLOSE_AUTHORITY: u16 = 3;
const EXT_METADATA_POINTER: u16 = 18;
/// Offset of the account-type byte in an extended Token-2022 account, and the
/// start of its TLV region.
const T22_ACCOUNT_TYPE_OFFSET: usize = 165;
const T22_TLV_OFFSET: usize = 166;
const T22_ACCOUNT_TYPE_MINT: u8 = 1;

/// Verify a Token-2022 mint is a well-formed, un-minted position NFT mint:
///
/// - base fields: initialized, 0 decimals, 0 supply, mint authority ==
///   `expected_authority` (the position PDA), NO freeze authority (a
///   freezable position NFT could be griefed into un-redeemability);
/// - extension TLV: EXACTLY the whitelisted set. `MintCloseAuthority` must be
///   present and equal the position PDA (otherwise the burn-to-close rent
///   reclaim would fail forever), `MetadataPointer` must be present and point
///   at the mint itself (self-contained on-chain metadata). Any other
///   extension is refused outright: a `PermanentDelegate` could seize the
///   NFT, a `TransferHook` could veto transfers, `DefaultAccountState` could
///   freeze holders. A whitelist cannot be surprised by new extensions.
pub fn verify_position_mint(
    data: &[u8],
    mint_key: &[u8; 32],
    expected_authority: &[u8; 32],
) -> Result<(), PositionError> {
    if data.len() < T22_TLV_OFFSET {
        return Err(PositionError::InvalidPositionMint);
    }
    let authority_tag = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let supply = read_u64(data, 36);
    let decimals = data[44];
    let initialized = data[45];
    let freeze_tag = u32::from_le_bytes(data[46..50].try_into().unwrap());
    if initialized != 1
        || decimals != 0
        || supply != 0
        || authority_tag != 1
        || &read_32(data, 4) != expected_authority
        || freeze_tag != 0
        || data[T22_ACCOUNT_TYPE_OFFSET] != T22_ACCOUNT_TYPE_MINT
    {
        return Err(PositionError::InvalidPositionMint);
    }

    // Walk the TLV entries: (type u16 LE, length u16 LE, data).
    let mut off = T22_TLV_OFFSET;
    let mut close_authority_ok = false;
    let mut metadata_pointer_ok = false;
    while off + 4 <= data.len() {
        let ext_type = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
        let ext_len = u16::from_le_bytes(data[off + 2..off + 4].try_into().unwrap()) as usize;
        if ext_type == 0 {
            break; // padding / uninitialized tail
        }
        let body = off + 4;
        if body + ext_len > data.len() {
            return Err(PositionError::InvalidPositionMint);
        }
        match ext_type {
            EXT_MINT_CLOSE_AUTHORITY => {
                // OptionalNonZeroPubkey: 32 bytes, zero = none.
                if ext_len != 32 || &read_32(data, body) != expected_authority {
                    return Err(PositionError::InvalidPositionMint);
                }
                close_authority_ok = true;
            }
            EXT_METADATA_POINTER => {
                // authority (32) + metadata_address (32); the pointer must aim
                // at the mint itself so wallets read the embedded metadata.
                if ext_len != 64 || &read_32(data, body + 32) != mint_key {
                    return Err(PositionError::InvalidPositionMint);
                }
                metadata_pointer_ok = true;
            }
            _ => return Err(PositionError::InvalidPositionMint),
        }
        off = body + ext_len;
    }
    if !close_authority_ok || !metadata_pointer_ok {
        return Err(PositionError::InvalidPositionMint);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portfolio_seed_pinned_to_upstream() {
        assert_eq!(PORTFOLIO_SEED, openperps_program::state::PORTFOLIO_SEED);
    }

    /// The local market-header constants must equal the pinned upstream's,
    /// byte for byte: serialize a real upstream header and read it back with
    /// the local offsets.
    #[test]
    fn market_header_reads_pinned_to_upstream() {
        use bytemuck::Zeroable;
        let mut h = openperps_program::state::OpenPerpsMarketHeader::zeroed();
        h.discriminator = openperps_program::state::MARKET_DISCRIMINATOR;
        h.version = openperps_program::state::MARKET_HEADER_VERSION;
        h.quote_mint = [7u8; 32];
        assert!(h.is_initialized());
        let bytes = bytemuck::bytes_of(&h);
        assert_eq!(market_quote_mint(bytes).unwrap(), [7u8; 32]);
        assert_eq!(MARKET_DISCRIMINATOR, openperps_program::state::MARKET_DISCRIMINATOR);
        assert_eq!(MARKET_HEADER_VERSION, openperps_program::state::MARKET_HEADER_VERSION);

        // Wrong version reads as uninitialized, never mis-decoded.
        let mut stale = h;
        stale.version = MARKET_HEADER_VERSION + 1;
        assert_eq!(
            market_quote_mint(bytemuck::bytes_of(&stale)),
            Err(PositionError::UninitializedMarket)
        );
        // Garbage rejected.
        assert!(market_quote_mint(&[0u8; 200]).is_err());
        assert!(market_quote_mint(&[0u8; 10]).is_err());
    }

    /// Build a well-formed Token-2022 position mint buffer: base mint fields,
    /// account-type byte, then MintCloseAuthority + MetadataPointer TLV.
    fn t22_mint(auth: &[u8; 32], mint_key: &[u8; 32]) -> Vec<u8> {
        let mut m = vec![0u8; 270];
        m[0..4].copy_from_slice(&1u32.to_le_bytes()); // mint authority = Some
        m[4..36].copy_from_slice(auth);
        m[45] = 1; // initialized
        m[165] = 1; // AccountType::Mint
        let mut o = 166;
        // MintCloseAuthority(3): 32 bytes.
        m[o..o + 2].copy_from_slice(&3u16.to_le_bytes());
        m[o + 2..o + 4].copy_from_slice(&32u16.to_le_bytes());
        m[o + 4..o + 36].copy_from_slice(auth);
        o += 36;
        // MetadataPointer(18): authority(32) + metadata_address(32) == mint.
        m[o..o + 2].copy_from_slice(&18u16.to_le_bytes());
        m[o + 2..o + 4].copy_from_slice(&64u16.to_le_bytes());
        m[o + 36..o + 68].copy_from_slice(mint_key);
        m
    }

    #[test]
    fn position_mint_checks() {
        let auth = [7u8; 32];
        let mint_key = [9u8; 32];
        let mint = t22_mint(&auth, &mint_key);
        assert!(verify_position_mint(&mint, &mint_key, &auth).is_ok());

        // Non-zero supply refused.
        let mut bad = mint.clone();
        bad[36] = 1;
        assert!(verify_position_mint(&bad, &mint_key, &auth).is_err());
        // Wrong mint authority refused.
        assert!(verify_position_mint(&mint, &mint_key, &[8u8; 32]).is_err());
        // Freeze authority refused.
        let mut frozen = mint.clone();
        frozen[46..50].copy_from_slice(&1u32.to_le_bytes());
        assert!(verify_position_mint(&frozen, &mint_key, &auth).is_err());
        // Non-zero decimals refused.
        let mut dec = mint.clone();
        dec[44] = 6;
        assert!(verify_position_mint(&dec, &mint_key, &auth).is_err());
        // Plain 82-byte v1 mint refused (no extensions at all).
        let mut v1 = vec![0u8; 82];
        v1[0..4].copy_from_slice(&1u32.to_le_bytes());
        v1[4..36].copy_from_slice(&auth);
        v1[45] = 1;
        assert!(verify_position_mint(&v1, &mint_key, &auth).is_err());
        // Wrong close authority refused.
        let mut wrong_close = mint.clone();
        wrong_close[170..202].copy_from_slice(&[8u8; 32]);
        assert!(verify_position_mint(&wrong_close, &mint_key, &auth).is_err());
        // Metadata pointer aimed elsewhere refused.
        let mut wrong_ptr = mint.clone();
        wrong_ptr[238..270].copy_from_slice(&[8u8; 32]);
        assert!(verify_position_mint(&wrong_ptr, &mint_key, &auth).is_err());
        // A non-whitelisted extension (PermanentDelegate = 12) refused.
        let mut delegated = t22_mint(&auth, &mint_key);
        delegated.extend_from_slice(&12u16.to_le_bytes());
        delegated.extend_from_slice(&32u16.to_le_bytes());
        delegated.extend_from_slice(&[6u8; 32]);
        assert!(verify_position_mint(&delegated, &mint_key, &auth).is_err());
        // Missing MintCloseAuthority refused (burn could never reclaim rent).
        let mut no_close = vec![0u8; 270];
        no_close[0..4].copy_from_slice(&1u32.to_le_bytes());
        no_close[4..36].copy_from_slice(&auth);
        no_close[45] = 1;
        no_close[165] = 1;
        no_close[166..168].copy_from_slice(&18u16.to_le_bytes());
        no_close[168..170].copy_from_slice(&64u16.to_le_bytes());
        no_close[202..234].copy_from_slice(&mint_key);
        assert!(verify_position_mint(&no_close, &mint_key, &auth).is_err());
    }

    #[test]
    fn token_account_reads() {
        let mut acc = vec![0u8; 165];
        acc[0..32].copy_from_slice(&[9u8; 32]);
        acc[32..64].copy_from_slice(&[10u8; 32]);
        acc[64..72].copy_from_slice(&777u64.to_le_bytes());
        assert_eq!(token_account_mint(&acc).unwrap(), [9u8; 32]);
        assert_eq!(token_account_owner(&acc).unwrap(), [10u8; 32]);
        assert_eq!(token_account_amount(&acc).unwrap(), 777);
    }
}
