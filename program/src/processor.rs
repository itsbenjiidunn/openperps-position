//! Instruction handlers.
//!
//! Authorization model, in one paragraph: every position is owned by a
//! `[POSITION_SEED, nft_mint]` PDA, and possession of the 1-of-1 NFT is the
//! ONLY credential; there is no allowlist, no admin, and no market-creator
//! override anywhere in this program. The OpenPerps deployment a position
//! belongs to is pinned by the market account's runtime OWNER, which the
//! Solana runtime enforces and no caller can fake.

use pinocchio::{
    account_info::AccountInfo,
    cpi::{invoke, invoke_signed},
    instruction::{AccountMeta, Instruction, Seed, Signer},
    pubkey::{create_program_address, Pubkey},
    ProgramResult,
};

use crate::error::PositionError;
use crate::instruction::PositionInstruction;
use crate::opcpi;
use crate::state::{
    market_quote_mint, token_account_amount, token_account_mint, token_account_owner,
    verify_position_mint, PORTFOLIO_SEED, POSITION_SEED,
};

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    match PositionInstruction::unpack(instruction_data)? {
        PositionInstruction::OpenPosition {
            side,
            asset_index,
            size_q,
            exec_price,
            fee_bps,
            deposit_amount,
            position_bump,
            portfolio_bump,
        } => process_open_position(
            program_id,
            accounts,
            side,
            asset_index,
            size_q,
            exec_price,
            fee_bps,
            deposit_amount,
            position_bump,
            portfolio_bump,
        ),
        PositionInstruction::ClosePosition {
            side,
            asset_index,
            size_q,
            exec_price,
            fee_bps,
            withdraw_amount,
            position_bump,
        } => process_close_position(
            program_id,
            accounts,
            side,
            asset_index,
            size_q,
            exec_price,
            fee_bps,
            withdraw_amount,
            position_bump,
        ),
    }
}

/// Signer seeds for a position PDA `[POSITION_SEED, nft_mint, bump]`.
macro_rules! position_signer {
    ($mint_key:expr, $bump_arr:expr) => {
        Signer::from(
            [
                Seed::from(POSITION_SEED),
                Seed::from($mint_key.as_ref()),
                Seed::from($bump_arr.as_ref()),
            ]
            .as_ref(),
        )
    };
}

/// Bind the call to the market's OpenPerps deployment and read its collateral
/// mint. The deployment identity is `market.owner()`, set by the runtime at
/// account creation and impossible to spoof; the supplied program account
/// must BE that owner and be executable. The collateral mint comes from the
/// market's wrapper header (discriminator + version verified).
fn bind_market(
    market: &AccountInfo,
    openperps_program: &AccountInfo,
) -> Result<([u8; 32], [u8; 32]), PositionError> {
    let openperps_key: Pubkey = *unsafe { market.owner() };
    if *openperps_program.key() != openperps_key || !openperps_program.executable() {
        return Err(PositionError::InvalidAccountData);
    }
    let collateral_mint = {
        let data = market
            .try_borrow_data()
            .map_err(|_| PositionError::InvalidAccountData)?;
        market_quote_mint(&data)?
    };
    Ok((openperps_key, collateral_mint))
}

#[allow(clippy::too_many_arguments)]
fn process_open_position(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    side: u8,
    asset_index: u32,
    size_q: u128,
    exec_price: u64,
    fee_bps: u64,
    deposit_amount: u128,
    position_bump: u8,
    portfolio_bump: u8,
) -> ProgramResult {
    let [market, house_portfolio, portfolio, position, nft_mint, holder, holder_nft, position_token, vault_token, token_program, token22_program, system_program, openperps_program, house_cap, fee_config, risk_config, ..] =
        accounts
    else {
        return Err(PositionError::InvalidInstruction.into());
    };
    if !holder.is_signer() {
        return Err(PositionError::MissingRequiredSignature.into());
    }
    // A position with no collateral or no exposure is meaningless; refuse
    // early rather than mint a dead NFT.
    if deposit_amount == 0 || size_q == 0 {
        return Err(PositionError::InvalidInstructionData.into());
    }
    // Two token programs are in play: v1 for the collateral and Token-2022
    // for the position NFT. Pin both identities.
    if *token_program.key() != opcpi::TOKEN_PROGRAM_ID
        || *token22_program.key() != opcpi::TOKEN_2022_PROGRAM_ID
    {
        return Err(PositionError::InvalidAccountData.into());
    }

    let (openperps_key, collateral_mint) = bind_market(market, openperps_program)?;

    // The position owner is the `[POSITION_SEED, nft_mint]` PDA; the OpenPerps
    // portfolio must be the canonical `[PORTFOLIO_SEED, position, market]`
    // under the market's own deployment.
    let derived_position = create_program_address(
        &[POSITION_SEED, nft_mint.key().as_ref(), &[position_bump]],
        program_id,
    )
    .map_err(|_| PositionError::InvalidAccountData)?;
    if *position.key() != derived_position {
        return Err(PositionError::InvalidAccountData.into());
    }
    let derived_portfolio = create_program_address(
        &[
            PORTFOLIO_SEED,
            position.key().as_ref(),
            market.key().as_ref(),
            &[portfolio_bump],
        ],
        &openperps_key,
    )
    .map_err(|_| PositionError::InvalidAccountData)?;
    if *portfolio.key() != derived_portfolio {
        return Err(PositionError::InvalidAccountData.into());
    }

    // The NFT mint must be a fresh Token-2022 mint fully surrendered to the
    // position PDA: supply 0, decimals 0, mint authority == position PDA, no
    // freeze authority, and EXACTLY the whitelisted extensions
    // (MintCloseAuthority == position PDA, MetadataPointer == the mint
    // itself). After this instruction the supply is fixed at exactly 1.
    if unsafe { nft_mint.owner() } != &opcpi::TOKEN_2022_PROGRAM_ID {
        return Err(PositionError::InvalidAccountOwner.into());
    }
    {
        let m = nft_mint
            .try_borrow_data()
            .map_err(|_| PositionError::InvalidAccountData)?;
        verify_position_mint(&m, nft_mint.key(), position.key())?;
    }
    // The NFT lands in a Token-2022 account of that mint (usually the holder's).
    if unsafe { holder_nft.owner() } != &opcpi::TOKEN_2022_PROGRAM_ID {
        return Err(PositionError::InvalidAccountOwner.into());
    }
    {
        let a = holder_nft
            .try_borrow_data()
            .map_err(|_| PositionError::InvalidAccountData)?;
        if token_account_mint(&a)? != *nft_mint.key() {
            return Err(PositionError::InvalidAccountData.into());
        }
    }
    // The collateral source must be owned by the position PDA (pre-funded by
    // the holder in the same transaction) and hold the market's collateral
    // mint, so the upstream Deposit's SPL transfer is authorized by the PDA's
    // signature.
    if unsafe { position_token.owner() } != &opcpi::TOKEN_PROGRAM_ID {
        return Err(PositionError::InvalidAccountOwner.into());
    }
    {
        let a = position_token
            .try_borrow_data()
            .map_err(|_| PositionError::InvalidAccountData)?;
        if token_account_mint(&a)? != collateral_mint || token_account_owner(&a)? != *position.key()
        {
            return Err(PositionError::InvalidAccountData.into());
        }
    }

    let bump_arr = [position_bump];

    // 1) Write the metadata INTO the mint account (TokenMetadata extension:
    //    fully on-chain, no URI hosting), mint the single position unit to
    //    the holder, then destroy the mint authority: supply is 1, forever.
    let name: &str = if side == 1 {
        "OpenPerps Short Position"
    } else {
        "OpenPerps Long Position"
    };
    opcpi::token_metadata_initialize(
        nft_mint,
        position,
        position,
        name,
        "OPPOS",
        &[position_signer!(nft_mint.key(), bump_arr)],
    )?;
    opcpi::token_mint_to_one(
        &opcpi::TOKEN_2022_PROGRAM_ID,
        nft_mint,
        holder_nft,
        position,
        &[position_signer!(nft_mint.key(), bump_arr)],
    )?;
    opcpi::token_revoke_mint_authority(
        &opcpi::TOKEN_2022_PROGRAM_ID,
        nft_mint,
        position,
        &[position_signer!(nft_mint.key(), bump_arr)],
    )?;

    // 2) Create the PDA-owned portfolio (the PDA pays rent; the holder
    //    pre-funds it with lamports in the same transaction).
    {
        let data = opcpi::encode_init_portfolio(portfolio_bump);
        let metas = [
            AccountMeta::new(portfolio.key(), true, false),
            AccountMeta::readonly(market.key()),
            AccountMeta::new(position.key(), true, true),
            AccountMeta::readonly(system_program.key()),
        ];
        let ix = Instruction {
            program_id: &openperps_key,
            data: &data,
            accounts: &metas,
        };
        invoke_signed::<4>(
            &ix,
            &[portfolio, market, position, system_program],
            &[position_signer!(nft_mint.key(), bump_arr)],
        )?;
    }

    // 3) Deposit the collateral from the PDA-owned staging account.
    {
        let data = opcpi::encode_deposit(deposit_amount);
        let metas = [
            AccountMeta::new(market.key(), true, false),
            AccountMeta::new(portfolio.key(), true, false),
            AccountMeta::readonly_signer(position.key()),
            AccountMeta::new(position_token.key(), true, false),
            AccountMeta::new(vault_token.key(), true, false),
            AccountMeta::readonly(token_program.key()),
        ];
        let ix = Instruction {
            program_id: &openperps_key,
            data: &data,
            accounts: &metas,
        };
        invoke_signed::<6>(
            &ix,
            &[market, portfolio, position, position_token, vault_token, token_program],
            &[position_signer!(nft_mint.key(), bump_arr)],
        )?;
    }

    // 4) Place the order. The upstream handler runs the whole risk stack
    //    (margin, OI caps, fee floor, dynamic spread) against the market's
    //    counterparty liquidity.
    {
        let data = opcpi::encode_place_order(side, asset_index, size_q, exec_price, fee_bps);
        let metas = [
            AccountMeta::new(market.key(), true, false),
            AccountMeta::new(portfolio.key(), true, false),
            AccountMeta::new(house_portfolio.key(), true, false),
            AccountMeta::readonly_signer(position.key()),
            AccountMeta::readonly(house_cap.key()),
            AccountMeta::readonly(fee_config.key()),
            AccountMeta::readonly(risk_config.key()),
        ];
        let ix = Instruction {
            program_id: &openperps_key,
            data: &data,
            accounts: &metas,
        };
        invoke_signed::<7>(
            &ix,
            &[market, portfolio, house_portfolio, position, house_cap, fee_config, risk_config],
            &[position_signer!(nft_mint.key(), bump_arr)],
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_close_position(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    side: u8,
    asset_index: u32,
    size_q: u128,
    exec_price: u64,
    fee_bps: u64,
    withdraw_amount: u128,
    position_bump: u8,
) -> ProgramResult {
    let [market, house_portfolio, portfolio, position, nft_mint, holder, holder_nft, position_token, vault_token, holder_token, token_program, token22_program, openperps_program, house_cap, fee_config, risk_config, ..] =
        accounts
    else {
        return Err(PositionError::InvalidInstruction.into());
    };
    if !holder.is_signer() {
        return Err(PositionError::MissingRequiredSignature.into());
    }
    if *token_program.key() != opcpi::TOKEN_PROGRAM_ID
        || *token22_program.key() != opcpi::TOKEN_2022_PROGRAM_ID
    {
        return Err(PositionError::InvalidAccountData.into());
    }

    let (openperps_key, collateral_mint) = bind_market(market, openperps_program)?;

    let derived_position = create_program_address(
        &[POSITION_SEED, nft_mint.key().as_ref(), &[position_bump]],
        program_id,
    )
    .map_err(|_| PositionError::InvalidAccountData)?;
    if *position.key() != derived_position {
        return Err(PositionError::InvalidAccountData.into());
    }

    // Bearer check: the signer must hold exactly the 1-of-1 position NFT. This
    // is the ONLY authorization; there is no allowlist and no creator override.
    if unsafe { holder_nft.owner() } != &opcpi::TOKEN_2022_PROGRAM_ID {
        return Err(PositionError::InvalidAccountOwner.into());
    }
    {
        let a = holder_nft
            .try_borrow_data()
            .map_err(|_| PositionError::InvalidAccountData)?;
        if token_account_mint(&a)? != *nft_mint.key()
            || token_account_owner(&a)? != *holder.key()
            || token_account_amount(&a)? != 1
        {
            return Err(PositionError::NotPositionHolder.into());
        }
    }
    // The canonical collateral staging account (closed below, rent to holder).
    if unsafe { position_token.owner() } != &opcpi::TOKEN_PROGRAM_ID {
        return Err(PositionError::InvalidAccountOwner.into());
    }
    {
        let a = position_token
            .try_borrow_data()
            .map_err(|_| PositionError::InvalidAccountData)?;
        if token_account_mint(&a)? != collateral_mint || token_account_owner(&a)? != *position.key()
        {
            return Err(PositionError::InvalidAccountData.into());
        }
    }

    let bump_arr = [position_bump];

    // 1) Flatten. Correctness is enforced by the ENGINE, not trusted from the
    //    caller: if these params do not fully close the position, the
    //    Withdraw below refuses (open legs) and the whole close reverts.
    //    size_q == 0 skips (already flat, e.g. fully liquidated).
    if size_q > 0 {
        let data = opcpi::encode_place_order(side, asset_index, size_q, exec_price, fee_bps);
        let metas = [
            AccountMeta::new(market.key(), true, false),
            AccountMeta::new(portfolio.key(), true, false),
            AccountMeta::new(house_portfolio.key(), true, false),
            AccountMeta::readonly_signer(position.key()),
            AccountMeta::readonly(house_cap.key()),
            AccountMeta::readonly(fee_config.key()),
            AccountMeta::readonly(risk_config.key()),
        ];
        let ix = Instruction {
            program_id: &openperps_key,
            data: &data,
            accounts: &metas,
        };
        invoke_signed::<7>(
            &ix,
            &[market, portfolio, house_portfolio, position, house_cap, fee_config, risk_config],
            &[position_signer!(nft_mint.key(), bump_arr)],
        )?;
    }

    // 2) Realize the released PnL into withdrawable capital. Permissionless
    //    upstream; the holder's outer signature satisfies its signer slot.
    {
        let data = opcpi::encode_settle_pnl();
        let metas = [
            AccountMeta::new(market.key(), true, false),
            AccountMeta::new(portfolio.key(), true, false),
            AccountMeta::readonly_signer(holder.key()),
        ];
        let ix = Instruction {
            program_id: &openperps_key,
            data: &data,
            accounts: &metas,
        };
        invoke::<3>(&ix, &[market, portfolio, holder])?;
    }

    // 3) Pay out principal plus PnL straight to the holder's token account.
    if withdraw_amount > 0 {
        let data = opcpi::encode_withdraw(withdraw_amount);
        let metas = [
            AccountMeta::new(market.key(), true, false),
            AccountMeta::new(portfolio.key(), true, false),
            AccountMeta::readonly_signer(position.key()),
            AccountMeta::new(vault_token.key(), true, false),
            AccountMeta::new(holder_token.key(), true, false),
            AccountMeta::readonly(token_program.key()),
        ];
        let ix = Instruction {
            program_id: &openperps_key,
            data: &data,
            accounts: &metas,
        };
        invoke_signed::<6>(
            &ix,
            &[market, portfolio, position, vault_token, holder_token, token_program],
            &[position_signer!(nft_mint.key(), bump_arr)],
        )?;
    }

    // 4) Burn the position NFT. The position PDA can never sign again
    //    because no one can ever hold the NFT again.
    opcpi::token_burn_one(&opcpi::TOKEN_2022_PROGRAM_ID, holder_nft, nft_mint, holder)?;

    // 5) Rent hygiene: every account this position ever created is closed and
    //    every lamport flows back to the holder.
    //    5a) Drain any dust from the staging account first (a stranger's
    //        donation must never wedge the close), then close it.
    let staged = {
        let a = position_token
            .try_borrow_data()
            .map_err(|_| PositionError::InvalidAccountData)?;
        token_account_amount(&a)?
    };
    if staged > 0 {
        opcpi::token_transfer_signed(
            &opcpi::TOKEN_PROGRAM_ID,
            position_token,
            holder_token,
            position,
            staged,
            &[position_signer!(nft_mint.key(), bump_arr)],
        )?;
    }
    opcpi::token_close_account(
        &opcpi::TOKEN_PROGRAM_ID,
        position_token,
        holder,
        position,
        &[position_signer!(nft_mint.key(), bump_arr)],
    )?;
    //    5b) Close the holder's NFT token account (empty after the burn).
    opcpi::token_close_account(
        &opcpi::TOKEN_2022_PROGRAM_ID,
        holder_nft,
        holder,
        holder,
        &[],
    )?;
    //    5c) Close the mint itself (supply is 0 after the burn; the
    //        MintCloseAuthority verified at open is the position PDA).
    opcpi::token_close_account(
        &opcpi::TOKEN_2022_PROGRAM_ID,
        nft_mint,
        holder,
        position,
        &[position_signer!(nft_mint.key(), bump_arr)],
    )?;
    //    5d) Sweep the position PDA's leftover lamports (rent prefund surplus).
    let leftover = position.lamports();
    if leftover > 0 {
        opcpi::system_transfer_signed(
            position,
            holder,
            leftover,
            &[position_signer!(nft_mint.key(), bump_arr)],
        )?;
    }

    Ok(())
}
