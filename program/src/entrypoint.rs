//! On-chain SBF entrypoint. Compiled only for `target_os = "solana"`.
//!
//! Hand-written panic handler instead of pinocchio's `nostd_panic_handler!`:
//! that macro puts `#[no_mangle]` on a `#[panic_handler]`, which the Rust
//! shipped in Solana platform-tools rejects on lang items. Same workaround as
//! the upstream OpenPerps program.

use crate::processor::process_instruction;
use pinocchio::{default_allocator, program_entrypoint};

program_entrypoint!(process_instruction);
default_allocator!();

/// `no_std` panic handler: report the panic location via the Solana syscall
/// and abort.
#[cfg(target_os = "solana")]
#[panic_handler]
fn handle_panic(info: &core::panic::PanicInfo<'_>) -> ! {
    if let Some(location) = info.location() {
        unsafe {
            pinocchio::syscalls::sol_panic_(
                location.file().as_ptr(),
                location.file().len() as u64,
                location.line() as u64,
                location.column() as u64,
            )
        }
    } else {
        pinocchio::log::sol_log("** PANICKED **");
        unsafe { pinocchio::syscalls::abort() }
    }
}
