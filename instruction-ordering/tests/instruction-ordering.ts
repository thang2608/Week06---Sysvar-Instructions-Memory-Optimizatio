import * as anchor from "@coral-xyz/anchor";
import { expect } from "chai";
import { SYSVAR_INSTRUCTIONS_PUBKEY,PublicKey } from "@solana/web3.js";

const BN = (anchor as any).BN || (anchor as any).default?.BN;

describe("instruction_ordering", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.InstructionOrdering;

  // ---------------- Part 1: Instruction Ordering ----------------

  it("fails to execute without approval", async () => {
    try {
      await program.methods
        .execute(new BN(1000))
        .accounts({
          authority: provider.wallet.publicKey,
          instructions: SYSVAR_INSTRUCTIONS_PUBKEY,
        })
        .rpc();

      expect.fail("Should have failed");
    } catch (err: any) {
      // TODO: Verify it failed with your custom error
      console.log(err);
      expect(err.message).to.include("MustApproveFirst");
    }
  });

  it("succeeds with approval in same transaction", async () => {
    // TODO:
    // - Create approve instruction:
    const approveIx = await program.methods
      .approve()
      .accounts({ authority: provider.wallet.publicKey })
      .instruction();
    
    // - Create execute instruction:
  const executeIx = await program.methods
    .execute(new BN(1000))
    .accounts({
      authority: provider.wallet.publicKey,
      instructions: SYSVAR_INSTRUCTIONS_PUBKEY,
    })
    .instruction();
    //
    // - Combine them in a single transaction in the correct order:
    const tx = new anchor.web3.Transaction().add(approveIx).add(executeIx);
    const signature = await provider.sendAndConfirm(tx);
    console.log("Signature: ", signature)
  });

  it("fails with wrong order (execute before approve)", async () => {
    // TODO:
    // - Build execute instruction first, then approve instruction
    // - Add them to a Transaction in the wrong order
    // - Send the transaction and assert that it fails with MustApproveFirst
    try{
    const approveIx = await program.methods
      .approve()
      .accounts({ authority: provider.wallet.publicKey })
      .instruction();
    
    const executeIx = await program.methods
    .execute(new BN(1000))
    .accounts({
      authority: provider.wallet.publicKey,
      instructions: SYSVAR_INSTRUCTIONS_PUBKEY,
    })
    .instruction();
    //
    const tx = new anchor.web3.Transaction().add(executeIx).add(approveIx);
    const signature = await provider.sendAndConfirm(tx);
    expect.fail("Should fail with MustApproveFirst error")
    }
    catch(err: any){
      console.log(err);
      expect(err.message).to.include("MustApproveFirst");
    }
  });

  // ---------------- Part 2: Regular Account<T> vs Zero-Copy ----------------

  it("initializes and uses large approval data with regular Account<T>", async () => {
    // TODO:
    // - Derive a PDA for the "regular" account:
    const [regularPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("approval_regular"), provider.wallet.publicKey.toBuffer()],
      program.programId
    );
    //
    // - Call `initializeLargeApprovalRegular` with that PDA.
    // - Then call `processLargeApprovalRegular` to write a timestamp.
    // - Fetch the account with `getAccountInfo` and assert that:
    //     * accountInfo is not null
    //     * data length is > 8 (so you know it's storing something non-trivial)
    await program.methods.initializeLargeApprovalRegular().accounts({
      approvalData: regularPda,
      authority: provider.wallet.publicKey,
      systemProgram:anchor.web3.SystemProgram.programId
    }).rpc();
    await program.methods.processLargeApprovalRegular().accounts({
      approvalData: regularPda,
      authority: provider.wallet.publicKey
    }).rpc();
    const accountInfo = await provider.connection.getAccountInfo(regularPda);
    expect(accountInfo).to.not.be.null;
    expect(accountInfo !.data.length).to.be.greaterThan(8);
  });

  it("initializes and uses large approval data with zero-copy AccountLoader<T>", async () => {
    const [zcPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("approval_zero_copy"), provider.wallet.publicKey.toBuffer()],
      program.programId
    );

    await program.methods
      .initializeLargeApprovalZeroCopy()
      .accounts({
        approvalData: zcPda,
        authority: provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    await program.methods
      .processLargeApprovalZeroCopy()
      .accounts({
        approvalData: zcPda,
        authority: provider.wallet.publicKey,
      })
      .rpc();

    const accountInfo = await provider.connection.getAccountInfo(zcPda);
    expect(accountInfo).to.not.be.null;
    expect(accountInfo!.data.length).to.be.greaterThan(4096);
  });
});
