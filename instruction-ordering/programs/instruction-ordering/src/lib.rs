use anchor_lang::prelude::*;
use anchor_lang::solana_program::sysvar::instructions;
use anchor_lang::system_program;

declare_id!("9bC8eiNLim4NvMzShUgQx8pPpVv6gRAELVnCFiMBuMBv");

#[program]
pub mod instruction_ordering {
    use super::*;

    // ---------------- Part 1: Instruction Ordering ----------------

    pub fn approve(_ctx: Context<Approve>) -> Result<()> {
        // TODO: Implement approval logic (you can just log for now)
        msg!("Approval granted");
        Ok(())
    }

    pub fn execute(ctx: Context<Execute>, amount: u64) -> Result<()> {
        // TODO: Check that previous instruction was `approve`
        // - Use `instructions::load_current_index_checked` to get the current index
        // - Ensure there was at least one previous instruction
        // - Use `instructions::load_instruction_at_checked` to fetch the previous ix
        // - Verify:
        //     * previous_ix.program_id == crate::ID
        //     * first 8 bytes of previous_ix.data match the "approve" discriminator
        //       (hint: `hash(b"global:approve").to_bytes()[0..8]`)
        let ix_acc = &ctx.accounts.instructions.to_account_info();
        let current = instructions::load_current_index_checked(ix_acc)?;
        require!(current > 0, ErrorCode::MustApproveFirst);
        let prev = instructions::load_instruction_at_checked((current - 1) as usize, ix_acc)?;
        require!(prev.program_id == crate::ID, ErrorCode::MustApproveFirst);
        require!(prev.data.len() >= 8, ErrorCode::InvalidData);
        let expected_discriminator = anchor_lang::solana_program::hash::hash(b"global:approve").to_bytes();
        require!(prev.data.starts_with(&expected_discriminator[0..8]), ErrorCode::MustApproveFirst);
        msg!("Executing with amount: {}", amount);
        Ok(())
    }

    // ---------------- Part 2: Large Data – Regular vs Zero-Copy ----------------

    pub fn initialize_large_approval_regular(
        ctx: Context<InitializeLargeApprovalRegular>,
    ) -> Result<()> {
        // TODO:
        // - Initialize a "regular" large account using `Account<LargeApprovalDataRegular>`
        // - Set the authority to `ctx.accounts.authority.key()`
        // - Zero out the approval_history array
        let mut data = &mut ctx.accounts.approval_data;
        data.authority = ctx.accounts.authority.key();
        data.approval_history.fill(0);
        Ok(())
    }

    pub fn process_large_approval_regular(ctx: Context<ProcessLargeApprovalRegular>) -> Result<()> {
        // TODO:
        // - Get current timestamp from `Clock::get()?`
        // - Find the first empty slot (value == 0) in approval_history
        // - Write the timestamp there
        let current_time = Clock::get()?.unix_timestamp as u64;
        for timestamp in ctx.accounts.approval_data.approval_history.iter_mut(){
            if *timestamp == 0{
                *timestamp = current_time;
                break;
            }
        }
        Ok(())
    }

    pub fn initialize_large_approval_zero_copy(
        ctx: Context<InitializeLargeApprovalZeroCopy>,
    ) -> Result<()> {
        // TODO:
        // - Use `ctx.accounts.approval_data.load_init()?` to get a zero-copy reference
        // - Set the authority (as bytes) and zero out the 512-element approval_history array
        let mut data = ctx.accounts.approval_data.load_init()?;
        data.authority = ctx.accounts.authority.key().to_bytes();
        data.approval_history.fill(0);
        Ok(())
    }

    pub fn process_large_approval_zero_copy(
        ctx: Context<ProcessLargeApprovalZeroCopy>,
    ) -> Result<()> {
        // TODO:
        // - Similar to the regular version, but using zero-copy:
        //   `let mut data = ctx.accounts.approval_data.load_mut()?;`
        // - Use `Clock::get()?` and write the timestamp into the first empty slot
        let mut data = ctx.accounts.approval_data.load_mut()?;
        let current_time = Clock::get()?.unix_timestamp as u64;
        for timestamp in data.approval_history.iter_mut(){
            if *timestamp == 0{
                *timestamp = current_time;
                break;
            }
        }
        Ok(())
    }

    pub fn multi_send<'info>(ctx:Context<'_, '_, '_,'info,MultiSend<'info>>, amount: u64) -> Result<()> {
        let recipients = ctx.remaining_accounts;
        require!(!recipients.is_empty(), ErrorCode::NoRecipients);
        require!(recipients.len() <= 10, ErrorCode::TooManyRecipients);
        for recipient in recipients.iter(){
            require!(recipient.is_writable, ErrorCode::RecipientNotWritable);
        }
        let amount_per_recipient = amount/(recipients.len() as u64);
        for recipient in recipients.iter(){
            let cpi_program = ctx.accounts.system_program.to_account_info();
            let cpi_account = system_program::Transfer{
                from: ctx.accounts.sender.to_account_info(),
                to: recipient.clone()
            };
            let cpi_ctx = CpiContext::new(ctx.accounts.system_program.to_account_info(), cpi_account);
            system_program::transfer(cpi_ctx, amount_per_recipient)?;
        }
        Ok(())
    }
}

// ---------------- Part 1 Accounts ----------------

#[derive(Accounts)]
pub struct Approve<'info> {
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct Execute<'info> {
    pub authority: Signer<'info>,

    /// CHECK: Instructions sysvar
    // TODO: Add constraint to verify this is the instructions sysvar
    #[account(address = anchor_lang::solana_program::sysvar::instructions::ID)]
    pub instructions: UncheckedAccount<'info>,
}

// ---------------- Part 2: Regular Account<T> ----------------

// TODO: Adjust this length to be "large but still compiles" under BPF stack limits.
// Later, you can experiment with increasing it to see stack usage errors.
pub const REGULAR_HISTORY_LEN: usize = 128;

#[account]
pub struct LargeApprovalDataRegular {
    // TODO: Add fields:
    // - authority: Pubkey
    // - approval_history: [u64; REGULAR_HISTORY_LEN]
    pub authority: Pubkey,
    pub approval_history: [u64; REGULAR_HISTORY_LEN],

}

#[derive(Accounts)]
pub struct InitializeLargeApprovalRegular<'info> {
    #[account(
        init_if_needed,
        payer = authority,
        // TODO: Set correct space: 8 + size_of::<LargeApprovalDataRegular>()
        space = 8 + std::mem::size_of::<LargeApprovalDataRegular>(),
        // TODO: Choose PDA seeds (e.g. b"approval_regular", authority key)
        seeds = [b"approval_regular", authority.key().as_ref()],
        bump
    )]
    pub approval_data: Box<Account<'info, LargeApprovalDataRegular>>,

    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ProcessLargeApprovalRegular<'info> {
    #[account(
        mut,
        // TODO: Use the same seeds as in InitializeLargeApprovalRegular
        seeds = [b"approval_regular", authority.key().as_ref()],
        bump
    )]
    pub approval_data: Box<Account<'info, LargeApprovalDataRegular>>,

    pub authority: Signer<'info>,
}

// ---------------- Part 2: Zero-Copy AccountLoader<T> ----------------

// TODO:
// - Make this a zero-copy account: `#[account(zero_copy)]`
// - Add `#[repr(C)]` and derives needed for zero-copy (e.g. Copy, Clone, Default or bytemuck)
// - Add fields:
//     * authority: [u8; 32]
//     * approval_history: [u64; 512]   // full large array
#[account(zero_copy)]
#[repr(C)]
pub struct LargeApprovalData {
    // TODO
    pub authority: [u8; 32],
    pub approval_history: [u64; 512],
}

#[derive(Accounts)]
pub struct InitializeLargeApprovalZeroCopy<'info> {
    #[account(
        init,
        payer = authority,
        // TODO: Set correct space: 8 + size_of::<LargeApprovalData>()
        space = 8 + std::mem::size_of::<LargeApprovalData>(),
        // TODO: Choose PDA seeds (e.g. b"approval_zero_copy", authority key)
        seeds = [b"approval_zero_copy", authority.key().as_ref()],
        bump
    )]
    // TODO: Use AccountLoader<LargeApprovalData> instead of Account<...>
    pub approval_data: AccountLoader<'info,LargeApprovalData>,

    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ProcessLargeApprovalZeroCopy<'info> {
    #[account(
        mut,
        // TODO: Use the same seeds as in InitializeLargeApprovalZeroCopy
        seeds = [b"approval_zero_copy", authority.key().as_ref()],
        bump
    )]
    // TODO: Use AccountLoader<LargeApprovalData>
    pub approval_data: AccountLoader<'info, LargeApprovalData>,

    pub authority: Signer<'info>,
}

// ---------------- Part 3 Accounts ----------------
 
#[derive(Accounts)]
pub struct MultiSend<'info> {
    /// Người gửi — bị debit lamport
    #[account(mut)]
    pub sender: Signer<'info>,
 
    pub system_program: Program<'info, System>,
    // remaining_accounts
}

// ---------------- Errors ----------------

#[error_code]
pub enum ErrorCode {
    #[msg("Must approve before executing")]
    MustApproveFirst,
    #[msg("Invalid Data")]
    InvalidData,
    #[msg("Must provide at least one recipient")]
    NoRecipients,
 
    #[msg("Too many recipients, maximum is 10")]
    TooManyRecipients,
 
    #[msg("Recipient account must be writable")]
    RecipientNotWritable,
}
