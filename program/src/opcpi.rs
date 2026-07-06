//! CPI layer: byte-exact encoders for the OpenPerps instructions this program
//! invokes, plus the SPL Token / Token-2022 / System calls the position
//! lifecycle needs.
//!
//! SECURITY INVARIANT: no handler ever forwards caller-supplied instruction
//! BYTES into a CPI. Every payload is (re-)encoded here from typed fields.
//! The upstream tag values are pinned by a host unit test against the exact
//! `openperps-program` revision in [dev-dependencies], so drift cannot ship.

use pinocchio::{
    account_info::AccountInfo,
    cpi::{invoke, invoke_signed},
    instruction::{AccountMeta, Instruction, Signer},
    pubkey::Pubkey,
    ProgramResult,
};

/// SPL Token program (v1, `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`).
pub const TOKEN_PROGRAM_ID: Pubkey = [
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133,
    237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
];

/// Solana System program (the all-zero pubkey).
pub const SYSTEM_PROGRAM_ID: Pubkey = [0u8; 32];

/// SPL Token-2022 program (`TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`).
/// Position NFTs live here so their metadata is stored INSIDE the mint account
/// (TokenMetadata extension, fully on-chain, no URI hosting) and the mint
/// itself is closeable at burn (MintCloseAuthority extension), so redemption
/// returns every lamport of rent to the holder.
pub const TOKEN_2022_PROGRAM_ID: Pubkey = [
    6, 221, 246, 225, 238, 117, 143, 222, 24, 66, 93, 188, 228, 108, 205, 218, 182, 26, 252, 77,
    131, 185, 13, 39, 254, 189, 249, 40, 216, 161, 139, 252,
];

/// OpenPerps instruction tags this program re-encodes. Defined locally so the
/// SBF artifact is self-contained; a host unit test pins every value against
/// the exact upstream revision in [dev-dependencies], so drift cannot ship.
mod op {
    pub const INIT_PORTFOLIO: u8 = 1;
    pub const DEPOSIT: u8 = 2;
    pub const WITHDRAW: u8 = 7;
    pub const PLACE_ORDER: u8 = 14;
    pub const SETTLE_PNL: u8 = 20;
}

/// `spl_token_metadata_interface:initialize_account`, first 8 bytes of its
/// SHA-256 (the interface's wire convention).
const METADATA_INITIALIZE_DISCRIMINATOR: [u8; 8] = [210, 225, 30, 162, 88, 184, 77, 141];

// ---------- System program ----------

/// `System::Transfer` FROM a system-owned PDA, signed via seeds. Used to
/// sweep the position PDA's leftover lamports back to the holder at close.
pub fn system_transfer_signed(
    from: &AccountInfo,
    to: &AccountInfo,
    lamports: u64,
    signer_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    let mut data = [0u8; 4 + 8];
    data[0..4].copy_from_slice(&2u32.to_le_bytes());
    data[4..12].copy_from_slice(&lamports.to_le_bytes());
    let accounts = [
        AccountMeta::new(from.key(), true, true),
        AccountMeta::new(to.key(), true, false),
    ];
    let ix = Instruction {
        program_id: &SYSTEM_PROGRAM_ID,
        data: &data,
        accounts: &accounts,
    };
    invoke_signed::<2>(&ix, &[from, to], signer_seeds)
}

// ---------- SPL Token (v1 and Token-2022 share the base instruction set) ----------

/// `Token::MintTo(1)` signed by the mint authority PDA. Mints the single
/// position unit to the holder.
pub fn token_mint_to_one(
    token_program: &Pubkey,
    mint: &AccountInfo,
    destination: &AccountInfo,
    authority: &AccountInfo,
    signer_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    let mut data = [0u8; 1 + 8];
    data[0] = 7;
    data[1..9].copy_from_slice(&1u64.to_le_bytes());
    let accounts = [
        AccountMeta::new(mint.key(), true, false),
        AccountMeta::new(destination.key(), true, false),
        AccountMeta::readonly_signer(authority.key()),
    ];
    let ix = Instruction {
        program_id: token_program,
        data: &data,
        accounts: &accounts,
    };
    invoke_signed::<3>(&ix, &[mint, destination, authority], signer_seeds)
}

/// `Token::SetAuthority(MintTokens -> None)` signed by the current authority
/// PDA. Freezes the position supply at exactly 1 forever.
pub fn token_revoke_mint_authority(
    token_program: &Pubkey,
    mint: &AccountInfo,
    authority: &AccountInfo,
    signer_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    // tag(6) | authority_type(0 = MintTokens) | COption tag(0 = None)
    let data = [6u8, 0, 0];
    let accounts = [
        AccountMeta::new(mint.key(), true, false),
        AccountMeta::readonly_signer(authority.key()),
    ];
    let ix = Instruction {
        program_id: token_program,
        data: &data,
        accounts: &accounts,
    };
    invoke_signed::<2>(&ix, &[mint, authority], signer_seeds)
}

/// `Token::Burn(1)` with the holder (an outer-tx signer) as authority. The
/// position NFT is destroyed in the same transaction that pays the holder.
pub fn token_burn_one(
    token_program: &Pubkey,
    account: &AccountInfo,
    mint: &AccountInfo,
    authority: &AccountInfo,
) -> ProgramResult {
    let mut data = [0u8; 1 + 8];
    data[0] = 8;
    data[1..9].copy_from_slice(&1u64.to_le_bytes());
    let accounts = [
        AccountMeta::new(account.key(), true, false),
        AccountMeta::new(mint.key(), true, false),
        AccountMeta::readonly_signer(authority.key()),
    ];
    let ix = Instruction {
        program_id: token_program,
        data: &data,
        accounts: &accounts,
    };
    invoke::<3>(&ix, &[account, mint, authority])
}

/// `Token::Transfer` signed by a PDA (drains a PDA-owned token account before
/// closing it, so a dust donation can never wedge a close).
pub fn token_transfer_signed(
    token_program: &Pubkey,
    source: &AccountInfo,
    destination: &AccountInfo,
    authority: &AccountInfo,
    amount: u64,
    signer_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    let mut data = [0u8; 1 + 8];
    data[0] = 3;
    data[1..9].copy_from_slice(&amount.to_le_bytes());
    let accounts = [
        AccountMeta::new(source.key(), true, false),
        AccountMeta::new(destination.key(), true, false),
        AccountMeta::readonly_signer(authority.key()),
    ];
    let ix = Instruction {
        program_id: token_program,
        data: &data,
        accounts: &accounts,
    };
    invoke_signed::<3>(&ix, &[source, destination, authority], signer_seeds)
}

/// `Token::CloseAccount` (tag 9): rent flows to `destination`. Works for
/// token accounts (authority = owner) and, on Token-2022, for mints whose
/// MintCloseAuthority is the signing PDA and whose supply is zero.
pub fn token_close_account(
    token_program: &Pubkey,
    account: &AccountInfo,
    destination: &AccountInfo,
    authority: &AccountInfo,
    signer_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    let data = [9u8];
    let accounts = [
        AccountMeta::new(account.key(), true, false),
        AccountMeta::new(destination.key(), true, false),
        AccountMeta::readonly_signer(authority.key()),
    ];
    let ix = Instruction {
        program_id: token_program,
        data: &data,
        accounts: &accounts,
    };
    invoke_signed::<3>(&ix, &[account, destination, authority], signer_seeds)
}

/// Token-2022 `TokenMetadataInitialize`: writes name/symbol/uri INTO the mint
/// account (TokenMetadata extension). The mint must carry a MetadataPointer
/// aimed at itself and enough pre-funded lamports for the reallocation; both
/// are verified/arranged before this is invoked. Signed by the position PDA
/// as mint authority; the update authority is the same PDA, which only ever
/// signs inside this program, so the metadata is immutable in practice.
pub fn token_metadata_initialize(
    mint: &AccountInfo,
    update_authority: &AccountInfo,
    mint_authority: &AccountInfo,
    name: &str,
    symbol: &str,
    signer_seeds: &[Signer<'_, '_>],
) -> ProgramResult {
    // discriminator(8) | borsh name | borsh symbol | borsh uri("")
    let name_b = name.as_bytes();
    let symbol_b = symbol.as_bytes();
    let mut data = [0u8; 8 + 4 + 64 + 4 + 16 + 4];
    let mut o = 0usize;
    data[o..o + 8].copy_from_slice(&METADATA_INITIALIZE_DISCRIMINATOR);
    o += 8;
    data[o..o + 4].copy_from_slice(&(name_b.len() as u32).to_le_bytes());
    o += 4;
    data[o..o + name_b.len()].copy_from_slice(name_b);
    o += name_b.len();
    data[o..o + 4].copy_from_slice(&(symbol_b.len() as u32).to_le_bytes());
    o += 4;
    data[o..o + symbol_b.len()].copy_from_slice(symbol_b);
    o += symbol_b.len();
    data[o..o + 4].copy_from_slice(&0u32.to_le_bytes()); // uri = ""
    o += 4;
    let accounts = [
        AccountMeta::new(mint.key(), true, false),
        AccountMeta::readonly(update_authority.key()),
        AccountMeta::readonly(mint.key()),
        AccountMeta::readonly_signer(mint_authority.key()),
    ];
    let ix = Instruction {
        program_id: &TOKEN_2022_PROGRAM_ID,
        data: &data[..o],
        accounts: &accounts,
    };
    invoke_signed::<4>(&ix, &[mint, update_authority, mint, mint_authority], signer_seeds)
}

// ---------- OpenPerps data encoders ----------
// Layouts mirror `openperps_program::instruction::OpenPerpsInstruction::unpack`
// exactly; pinned against the upstream crate by host tests.

pub fn encode_init_portfolio(bump: u8) -> [u8; 2] {
    [op::INIT_PORTFOLIO, bump]
}

pub fn encode_deposit(amount: u128) -> [u8; 17] {
    let mut d = [0u8; 17];
    d[0] = op::DEPOSIT;
    d[1..17].copy_from_slice(&amount.to_le_bytes());
    d
}

pub fn encode_withdraw(amount: u128) -> [u8; 17] {
    let mut d = [0u8; 17];
    d[0] = op::WITHDRAW;
    d[1..17].copy_from_slice(&amount.to_le_bytes());
    d
}

pub fn encode_place_order(
    side: u8,
    asset_index: u32,
    size_q: u128,
    exec_price: u64,
    fee_bps: u64,
) -> [u8; 38] {
    let mut d = [0u8; 38];
    d[0] = op::PLACE_ORDER;
    d[1] = side;
    d[2..6].copy_from_slice(&asset_index.to_le_bytes());
    d[6..22].copy_from_slice(&size_q.to_le_bytes());
    d[22..30].copy_from_slice(&exec_price.to_le_bytes());
    d[30..38].copy_from_slice(&fee_bps.to_le_bytes());
    d
}

pub fn encode_settle_pnl() -> [u8; 1] {
    [op::SETTLE_PNL]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The locally defined program ids and tag bytes must equal the pinned
    /// upstream revision's. Runs on host only (upstream is a dev-dependency).
    #[test]
    fn local_constants_pinned_to_upstream() {
        use openperps_program::instruction::tag as up;
        assert_eq!(TOKEN_PROGRAM_ID, openperps_program::cpi::TOKEN_PROGRAM_ID);
        assert_eq!(SYSTEM_PROGRAM_ID, openperps_program::cpi::SYSTEM_PROGRAM_ID);
        assert_eq!(op::INIT_PORTFOLIO, up::INIT_PORTFOLIO);
        assert_eq!(op::DEPOSIT, up::DEPOSIT);
        assert_eq!(op::WITHDRAW, up::WITHDRAW);
        assert_eq!(op::PLACE_ORDER, up::PLACE_ORDER);
        assert_eq!(op::SETTLE_PNL, up::SETTLE_PNL);
    }

    /// Byte-for-byte parity: everything encoded here must round-trip through
    /// the pinned upstream decoder. This is the drift tripwire; if upstream
    /// ever changes a layout, this test fails before anything ships.
    #[test]
    fn encoders_roundtrip_through_upstream_unpack() {
        use openperps_program::instruction::OpenPerpsInstruction as Op;

        assert_eq!(
            Op::unpack(&encode_init_portfolio(255)).unwrap(),
            Op::InitPortfolio { bump: 255 }
        );
        assert_eq!(
            Op::unpack(&encode_deposit(123_456_789)).unwrap(),
            Op::Deposit { amount: 123_456_789 }
        );
        assert_eq!(
            Op::unpack(&encode_withdraw(42)).unwrap(),
            Op::Withdraw { amount: 42 }
        );
        assert_eq!(
            Op::unpack(&encode_place_order(1, 0, 500, 1_000_000, 5)).unwrap(),
            Op::PlaceOrder {
                side: 1,
                asset_index: 0,
                size_q: 500,
                exec_price: 1_000_000,
                fee_bps: 5,
            }
        );
        assert_eq!(Op::unpack(&encode_settle_pnl()).unwrap(), Op::SettlePnl);
    }
}
