//! Program-specific errors, mapped to `ProgramError::Custom(code)`.
//! Discriminants are the on-chain custom codes; existing variants must keep
//! their numbers stable.

use pinocchio::program_error::ProgramError;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionError {
    /// Instruction tag byte did not match any known instruction.
    InvalidInstruction = 0,
    /// Instruction payload was malformed or too short.
    InvalidInstructionData = 1,
    /// Account is not owned by the expected program.
    InvalidAccountOwner = 2,
    /// A required signer did not sign.
    MissingRequiredSignature = 3,
    /// Account data layout / derivation / identity was invalid.
    InvalidAccountData = 4,
    /// The market account is not an initialized OpenPerps market of the
    /// layout version this build understands.
    UninitializedMarket = 5,
    /// The position NFT mint is malformed: wrong decimals / supply / mint
    /// authority, a freeze authority, or a non-whitelisted Token-2022
    /// extension (any of which could grief the holder out of redemption).
    InvalidPositionMint = 6,
    /// The signer does not hold the position NFT (token account must hold
    /// exactly 1 of the mint and be owned by the signer).
    NotPositionHolder = 7,
}

impl From<PositionError> for ProgramError {
    fn from(e: PositionError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
