# Phần VI - Sysvar Instructions & Memory Optimization

Bài học này tập trung vào hai chủ đề liên quan chặt chẽ: **cách Solana thực thi các instruction trong một giao dịch** và **cách program on-chain có thể kiểm tra (introspect) các instruction khác trong cùng giao dịch** thông qua Sysvar Instructions. Ngoài ra, bạn sẽ học các giới hạn runtime quan trọng cùng các kỹ thuật tối ưu hóa bộ nhớ mà bất kỳ nhà phát triển Solana nào cũng cần nắm vững.

---

Kết thúc bài học, bạn sẽ:

✅ Hiểu mô hình thực thi tuần tự và nguyên tử của Solana
✅ Biết tại sao thứ tự instruction ảnh hưởng đến tính đúng đắn và bảo mật
✅ Sử dụng Sysvar Instructions để kiểm tra và áp đặt ràng buộc giữa các instruction
✅ Nắm các giới hạn runtime: CPI depth, call stack, compute budget, stack frame
✅ Áp dụng các kỹ thuật tối ưu hóa: Box, zero-copy, remaining_accounts, #[inline(never)], UncheckedAccount

---

## 1. Mô hình Thực thi Giao dịch trên Solana

### Thực thi tuần tự

Một giao dịch Solana chứa một hoặc nhiều instruction. Runtime thực thi chúng **lần lượt theo thứ tự** — instruction 0 hoàn thành rồi mới đến instruction 1, rồi 2, v.v. Mỗi instruction sau có thể đọc các thay đổi trạng thái mà instruction trước đã ghi.

```
Transaction { instructions: [Ix0, Ix1, Ix2] }

1. Ix0 chạy → thay đổi state → OK
2. Ix1 chạy → đọc state mới từ Ix0 → thay đổi state → OK
3. Ix2 chạy → đọc state mới từ Ix1 → thay đổi state → OK

→ Giao dịch thành công, tất cả thay đổi được commit.
```

### Tính nguyên tử (Atomicity)

Nếu **bất kỳ instruction nào** trong giao dịch thất bại, **toàn bộ** giao dịch bị hủy và mọi thay đổi trạng thái bị rollback. Phí giao dịch (base fee) vẫn bị tính.

Có thể hình dung như một database transaction:

```sql
BEGIN;
  INSERT INTO deposits ...;   -- Ix0
  UPDATE balances SET ...;    -- Ix1
  DELETE FROM pending ...;    -- Ix2: nếu lỗi ở đây → toàn bộ rollback
COMMIT;
```

Tính chất này rất hữu ích: bạn có thể gộp nhiều thao tác phụ thuộc nhau vào một giao dịch duy nhất, đảm bảo hoặc tất cả thành công, hoặc không có gì xảy ra.

### Trạng thái phụ thuộc thứ tự

Kết quả cuối cùng phụ thuộc vào thứ tự instruction. Ví dụ:

```rust
// Ix0: counter = 0
// Ix1: counter += 5    → counter = 5
// Ix2: counter *= 2    → counter = 10

// Nếu đảo Ix1 và Ix2:
// Ix0: counter = 0
// Ix1: counter *= 2    → counter = 0  (!)
// Ix2: counter += 5    → counter = 5
```

Cùng các instruction, nhưng thứ tự khác → kết quả khác. Đây không phải bug — đây là đặc điểm cơ bản của mô hình thực thi tuần tự.

---

## 2. Tại sao Thứ tự Instruction Quan trọng

### Tính đúng đắn (Correctness)

Nhiều thao tác có phụ thuộc logic tự nhiên. Ví dụ, trong một DEX swap, bạn phải:

1. Kiểm tra điều kiện slippage
2. Thực hiện swap

Nếu đảo ngược — swap trước, kiểm tra sau — thì kiểm tra trở nên vô nghĩa vì tiền đã chuyển rồi.

### Bảo mật (Security)

Quy tắc vàng trong program: **kiểm tra trước, thay đổi sau** (checks-effects-interactions pattern). Luôn xác minh tất cả precondition trước khi mutate bất kỳ state nào.

```rust
// ❌ Nguy hiểm: thay đổi state rồi mới kiểm tra
pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
    ctx.accounts.vault.balance -= amount;
    ctx.accounts.user.balance += amount;
    require!(ctx.accounts.authority.is_signer, ErrorCode::Unauthorized);
    Ok(())
}
```

Vấn đề: nếu `require!` thất bại, toàn bộ giao dịch rollback nên state vẫn an toàn. Nhưng mô hình này dễ dẫn đến lỗi khi refactor — ai đó có thể vô tình tách logic thành instruction riêng và quên rằng kiểm tra phải đi trước.

```rust
// ✅ An toàn: kiểm tra trước, thay đổi sau
pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
    require!(ctx.accounts.authority.is_signer, ErrorCode::Unauthorized);
    require!(ctx.accounts.vault.balance >= amount, ErrorCode::InsufficientFunds);

    ctx.accounts.vault.balance -= amount;
    ctx.accounts.user.balance += amount;
    Ok(())
}
```

### Composability

Khi program được gọi qua CPI hoặc gộp cùng instruction khác, bạn cần đảm bảo:

- **Preconditions**: Các điều kiện đầu vào được thỏa mãn trước khi instruction chạy
- **Postconditions**: Trạng thái đầu ra hợp lệ sau khi instruction hoàn thành
- **Invariants**: Các bất biến được duy trì xuyên suốt

---

## 3. Sysvar Instructions: Introspect Giao dịch từ On-chain

### Instructions Sysvar là gì?

Theo [tài liệu Solana](https://solana.com/docs/core/instructions/instruction-introspection), Instructions Sysvar là một tài khoản đặc biệt tại địa chỉ:

```
Sysvar1nstructions1111111111111111111111111
```

Nó cho phép program on-chain **đọc tất cả top-level instruction** trong giao dịch hiện tại — bao gồm program ID, danh sách tài khoản, và dữ liệu instruction.

Điểm quan trọng: **CPI inner instruction không thể truy cập** qua sysvar này. Chỉ các instruction cấp cao nhất (top-level) trong message mới được liệt kê.

### Cách truy cập

Khác với các sysvar khác (Clock, Rent, ...), Instructions Sysvar **không** được truy cập qua trait `Sysvar` tiêu chuẩn. Thay vào đó, bạn sử dụng các free function:

```rust
use solana_program::sysvar::instructions;

// Lấy index của instruction hiện tại
let current_idx = instructions::load_current_index_checked(ix_account_info)?;

// Đọc instruction tại một index tuyệt đối
let ix = instructions::load_instruction_at_checked(index, ix_account_info)?;

// Đọc instruction theo offset tương đối (-1 = trước, +1 = sau)
let prev_ix = instructions::get_instruction_relative(-1, ix_account_info)?;
```

Mỗi instruction được trả về gồm: `program_id`, `accounts` (danh sách `AccountMeta`), và `data` (byte array).

### Cấu trúc dữ liệu bên trong Sysvar

Dữ liệu sysvar được serialize theo layout nhị phân riêng:


| Offset      | Kích thước | Mô tả                                          |
| ----------- | ---------- | ---------------------------------------------- |
| 0           | 2 byte     | `num_instructions` (u16 little-endian)         |
| 2           | 2 * N byte | Byte offset cho từng instruction (u16 mỗi cái) |
| varies      | varies     | Dữ liệu instruction đã serialize               |
| 2 byte cuối | 2 byte     | Index instruction hiện tại (u16 little-endian) |


Runtime tự động cập nhật index instruction hiện tại mỗi khi bắt đầu thực thi instruction mới.

### Khi nào sử dụng?

- **Buộc instruction phải ở vị trí cụ thể**: chỉ cho phép chạy nếu là instruction đầu tiên, cuối cùng, hoặc duy nhất
- **Chống CPI giả mạo**: kiểm tra instruction được gọi trực tiếp (top-level) chứ không phải qua CPI
- **Workflow tuần tự**: buộc instruction A phải chạy trước instruction B trong cùng giao dịch
- **Chia tách instruction lớn**: khi một instruction vượt giới hạn stack/compute, chia thành 2 instruction nhỏ hơn và dùng sysvar để đảm bảo chúng luôn đi cùng nhau

---

## 4. Các Pattern Thường gặp

### Pattern 1: Buộc là instruction đầu tiên

Hữu ích khi instruction cần chạy trước bất kỳ thứ gì khác (ví dụ: khởi tạo state).

```rust
let current = instructions::load_current_index_checked(ix_sysvar)?;
require!(current == 0, MyError::MustBeFirstInstruction);
```

### Pattern 2: Buộc instruction trước đó phải là một instruction cụ thể

Đây là pattern phổ biến nhất. Ví dụ: `withdraw` chỉ hợp lệ nếu `deposit` chạy ngay trước nó.

```rust
let ix_acc = &ctx.accounts.ix_sysvar.to_account_info();
let current = instructions::load_current_index_checked(ix_acc)?;

require!(current > 0, MyError::MissingPreviousInstruction);

let prev = instructions::load_instruction_at_checked(
    (current - 1) as usize,
    ix_acc,
)?;

// Kiểm tra program_id — phải đến từ program này
require!(prev.program_id == crate::ID, MyError::WrongProgram);

// Kiểm tra discriminator — 8 byte đầu của instruction data
require!(prev.data.len() >= 8, MyError::InvalidData);
require!(&prev.data[0..8] == EXPECTED_DISCRIMINATOR, MyError::WrongInstruction);
```

Anchor sinh discriminator bằng `sha256("global:<instruction_name>")[0..8]`. Bạn tính off-chain rồi hardcode vào program.

### Pattern 3: Giới hạn tổng số instruction

Ngăn người dùng gửi giao dịch quá phức tạp:

```rust
let ix_acc = &ctx.accounts.ix_sysvar.to_account_info();
let mut count: usize = 0;
while instructions::load_instruction_at_checked(count, ix_acc).is_ok() {
    count += 1;
    if count > 64 { break; }
}
require!(count <= 3, MyError::TooManyInstructions);
```

### Pattern 4: Instruction duy nhất (chống sandwich)

Đảm bảo instruction của bạn là instruction duy nhất trong giao dịch — ngăn kẻ tấn công đính kèm instruction trước/sau:

```rust
let ix_acc = &ctx.accounts.ix_sysvar.to_account_info();
let current = instructions::load_current_index_checked(ix_acc)?;
require!(current == 0, MyError::MustBeOnly);

// Không có instruction nào sau
require!(
    instructions::load_instruction_at_checked(1, ix_acc).is_err(),
    MyError::MustBeOnly
);
```

### Ví dụ hoàn chỉnh: Workflow Approve → Execute

Dưới đây là ví dụ một program yêu cầu `approve` phải chạy ngay trước `execute`:

```rust
use anchor_lang::prelude::*;
use solana_program::sysvar::instructions as ix_sysvar;

declare_id!("YourProgramId1111111111111111111111111111");

const APPROVE_DISCRIMINATOR: [u8; 8] = [0x61, 0x70, 0x70, 0x72, 0x6f, 0x76, 0x65, 0x00];
// ↑ Tính off-chain bằng: sha256("global:approve")[0..8]

#[program]
pub mod sequential_flow {
    use super::*;

    pub fn approve(_ctx: Context<Approve>) -> Result<()> {
        msg!("Approved!");
        Ok(())
    }

    pub fn execute(ctx: Context<Execute>) -> Result<()> {
        let ix_acc = &ctx.accounts.ix_sysvar.to_account_info();
        let current = ix_sysvar::load_current_index_checked(ix_acc)?;

        require!(current > 0, SequentialError::NoApproval);

        let prev = ix_sysvar::load_instruction_at_checked(
            (current - 1) as usize,
            ix_acc,
        )?;

        require!(prev.program_id == crate::ID, SequentialError::WrongProgram);
        require!(prev.data.len() >= 8, SequentialError::InvalidData);
        require!(
            prev.data[0..8] == APPROVE_DISCRIMINATOR,
            SequentialError::NoApproval
        );

        msg!("Executed after approval!");
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Approve<'info> {
    pub authority: Signer<'info>,
}

#[derive(Accounts)]
pub struct Execute<'info> {
    pub authority: Signer<'info>,

    /// CHECK: Instructions sysvar — chỉ cần đúng địa chỉ
    #[account(address = solana_program::sysvar::instructions::ID)]
    pub ix_sysvar: UncheckedAccount<'info>,
}

#[error_code]
pub enum SequentialError {
    #[msg("Approve instruction must precede Execute")]
    NoApproval,
    #[msg("Previous instruction is from wrong program")]
    WrongProgram,
    #[msg("Previous instruction data is invalid")]
    InvalidData,
}
```

Phía client, giao dịch phải gộp cả hai instruction:

```typescript
const tx = new Transaction()
  .add(await program.methods.approve().accounts({ authority: wallet.publicKey }).instruction())
  .add(
    await program.methods
      .execute()
      .accounts({
        authority: wallet.publicKey,
        ixSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
      })
      .instruction()
  );

await sendAndConfirmTransaction(connection, tx, [wallet]);
```

---

## 5. Giới hạn Runtime

Khi thiết kế giao dịch phức tạp với nhiều instruction, bạn cần biết các giới hạn sau. Nguồn chính từ [Solana Docs - Limitations](https://solana.com/docs/programs/limitations) và [Fees/Compute Budget](https://solana.com/docs/core/fees).

### CPI Depth — Lỗi `CallDepth`

Solana giới hạn độ sâu CPI ở **4 cấp**:

```
Top-level instruction
 → Program A gọi CPI tới Program B    (depth 1)
   → B gọi CPI tới Program C          (depth 2)
     → C gọi CPI tới Program D        (depth 3)
       → D gọi CPI tới Program E      (depth 4)
         → E gọi CPI → ❌ CallDepth error
```

### Call Stack Depth — Lỗi `CallDepthExceeded`

Bên trong một program, call stack (gọi hàm lồng nhau) giới hạn ở **64 frame**. Đệ quy sâu hoặc chuỗi hàm quá dài sẽ gây lỗi `CallDepthExceeded`.

### Compute Budget


| Giới hạn                       | Giá trị              | Nguồn                                    |
| ------------------------------ | -------------------- | ---------------------------------------- |
| CU mặc định / instruction      | 200.000              | `DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT` |
| CU tối đa / giao dịch          | 1.400.000            | `MAX_COMPUTE_UNIT_LIMIT`                 |
| Instruction tối đa / giao dịch | 64 (top-level + CPI) | `MAX_INSTRUCTION_TRACE_LENGTH`           |


Bạn có thể yêu cầu thêm CU bằng `ComputeBudgetInstruction::set_compute_unit_limit()`, nhưng không thể vượt 1.400.000 cho toàn bộ giao dịch.

### Stack Frame — Lỗi `Access violation in stack frame`

SBF runtime cấp **4 KiB stack** cho mỗi instruction handler. Anchor deserialize toàn bộ `Accounts` struct lên stack. Nếu struct quá lớn (nhiều tài khoản, tài khoản chứa dữ liệu lớn), bạn sẽ gặp:

```
Program failed to complete: Access violation in stack frame 7 
at address 0x200007fd8 of size 8 by instruction #4170
```

Đây thực chất là **stack overflow** nhưng trong runtime SBF của Solana.

---

## 6. Kỹ thuật Tối ưu hóa Bộ nhớ

### 6.1 `Box<Account<'info, T>>` — Chuyển tài khoản lên heap

Cách đơn giản nhất: wrap tài khoản lớn bằng `Box` để Anchor allocate trên heap thay vì stack.

```rust
#[derive(Accounts)]
pub struct HeavyInstruction<'info> {
    #[account(mut)]
    pub big_account: Box<Account<'info, BigData>>,
    // ...
}
```

Đánh đổi: tốn thêm compute unit cho heap allocation. Chỉ dùng khi tài khoản thực sự lớn.

### 6.2 Zero-copy — Đọc trực tiếp từ bộ nhớ tài khoản

Khi tài khoản rất lớn (vd: mảng 2000 phần tử), thậm chí `Box` cũng không đủ vì deserialization tốn quá nhiều compute. Zero-copy giải quyết bằng cách **trỏ trực tiếp** vào vùng nhớ tài khoản — không copy, không deserialize.

```rust
#[account(zero_copy)]
#[repr(C)]
pub struct OrderBook {
    pub authority: Pubkey,
    pub orders: [Order; 500],
}
```

Trong `Accounts` struct, dùng `AccountLoader` thay vì `Account`:

```rust
#[derive(Accounts)]
pub struct ProcessOrder<'info> {
    #[account(mut)]
    pub order_book: AccountLoader<'info, OrderBook>,
}
```

Truy cập dữ liệu:

```rust
pub fn process(ctx: Context<ProcessOrder>) -> Result<()> {
    let mut book = ctx.accounts.order_book.load_mut()?;
    book.orders[0].price = 100;
    Ok(())
}
```

**Khi nào dùng zero-copy:**

- Dữ liệu có kích thước cố định, cấu trúc ổn định (mảng lớn, order book, history buffer)
- Tài khoản quá lớn để deserialize

**Khi nào KHÔNG dùng:**

- Dữ liệu động (`Vec`, `String`, Option có kích thước biến đổi)
- Struct có thể thay đổi layout trong tương lai
- Code đơn giản quan trọng hơn hiệu năng

### 6.3 `#[inline(never)]` — Giảm kích thước stack frame

Khi một hàm chứa nhiều biến cục bộ lớn, trình biên dịch dự trữ stack cho tất cả chúng **cùng lúc**, dù chúng được dùng tại các thời điểm khác nhau.

```rust
fn heavy_handler(ctx: Context<Heavy>) -> Result<()> {
    let data_a: [u8; 10_000] = [...];
    let data_b: [u8; 10_000] = [...];
    let data_c: [u8; 10_000] = [...];
    // stack frame ≈ 30KB → ❌ overflow
    process_a(&data_a);
    process_b(&data_b);
    process_c(&data_c);
    Ok(())
}
```

Tách thành các hàm con với `#[inline(never)]` để mỗi hàm con chỉ chiếm stack riêng:

```rust
fn heavy_handler(ctx: Context<Heavy>) -> Result<()> {
    step_a();
    step_b();
    step_c();
    Ok(())
}

#[inline(never)]
fn step_a() {
    let data: [u8; 10_000] = [...];
    process_a(&data);
    // data bị drop khi step_a return → giải phóng stack
}

#[inline(never)]
fn step_b() {
    let data: [u8; 10_000] = [...];
    process_b(&data);
}

#[inline(never)]
fn step_c() {
    let data: [u8; 10_000] = [...];
    process_c(&data);
}
```

Tại runtime, stack chỉ chứa một sub-frame tại mỗi thời điểm:

```
heavy_handler frame (~nhỏ)
  → step_a frame (10KB) → return, giải phóng
  → step_b frame (10KB) → return, giải phóng
  → step_c frame (10KB) → return, giải phóng
```

Peak stack usage: ~10KB thay vì ~30KB.

### 6.4 `remaining_accounts` — Tài khoản động trên heap

Mọi tài khoản khai báo trong `Accounts` struct đều được deserialize lên stack. Nếu số lượng tài khoản **biến động** (ví dụ: gửi SOL cho N người nhận, N không cố định), thay vì khai báo tất cả trong struct, hãy dùng `remaining_accounts`:

```rust
#[derive(Accounts)]
pub struct MultiSend<'info> {
    #[account(mut)]
    pub sender: Signer<'info>,
    pub system_program: Program<'info, System>,
    // Danh sách người nhận không khai báo ở đây
}

pub fn multi_send(ctx: Context<MultiSend>, amount_per_recipient: u64) -> Result<()> {
    let recipients = &ctx.remaining_accounts;

    require!(!recipients.is_empty(), MyError::NoRecipients);
    require!(recipients.len() <= 10, MyError::TooManyRecipients);

    for recipient in recipients.iter() {
        require!(recipient.is_writable, MyError::NotWritable);

        let ix = solana_program::system_instruction::transfer(
            ctx.accounts.sender.key,
            recipient.key,
            amount_per_recipient,
        );
        solana_program::program::invoke(
            &ix,
            &[
                ctx.accounts.sender.to_account_info(),
                recipient.clone(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
    }

    Ok(())
}
```

Client truyền remaining accounts như sau:

```typescript
const recipients = [addr1, addr2, addr3].map((pk) => ({
  pubkey: pk,
  isWritable: true,
  isSigner: false,
}));

await program.methods
  .multiSend(new BN(1_000_000))
  .accounts({
    sender: wallet.publicKey,
    systemProgram: SystemProgram.programId,
  })
  .remainingAccounts(recipients)
  .rpc();
```

`remaining_accounts` nằm trên heap → không tốn stack space cho dù có bao nhiêu tài khoản.

### 6.5 `UncheckedAccount` thay cho `AccountInfo`

Nếu bạn không cần validate một tài khoản (có thể nó được kiểm tra bởi instruction khác trong cùng giao dịch), hãy dùng `UncheckedAccount<'info>` thay vì `AccountInfo<'info>`. Nó thể hiện rõ ý định "không kiểm tra" trong code, và Anchor cũng yêu cầu annotation `/// CHECK:` để developer phải giải thích lý do — giúp reviewer dễ đánh giá bảo mật.

---

## 7. Gỡ lỗi

### Log instruction index

```rust
let current = instructions::load_current_index_checked(ix_acc)?;
msg!("Current instruction index: {}", current);
```

### Simulate trước khi gửi

```typescript
const simulation = await connection.simulateTransaction(tx);
if (simulation.value.err) {
  console.error("Error:", simulation.value.err);
  console.log("Logs:", simulation.value.logs);
}
```

### Đọc transaction logs

Logs từ Solana Explorer hoặc `getTransaction` thường có dạng:

```
Program YourProgram invoke [1]
Program log: Current instruction index: 0
Program log: Previous instruction verified
Program YourProgram consumed 25000 of 200000 compute units
Program YourProgram success
```

Số trong `invoke [N]` cho biết CPI depth. `[1]` = top-level, `[2]` = CPI cấp 1, v.v.

---

## 8. Bài tập

Trong bài tập này, bạn sẽ triển khai một **hệ thống phê duyệt tuần tự (sequential approval system)** yêu cầu hai instruction theo thứ tự, sau đó tối ưu hóa nó với zero-copy để xử lý dữ liệu lớn.

### Phần 1: Phê duyệt Tuần tự

Xây dựng một hệ thống mà lệnh `execute` chỉ có thể chạy nếu lệnh `approve` đã chạy ngay trước nó trong cùng một giao dịch.

1. Tạo một instruction `approve` để ghi lại sự phê duyệt
2. Tạo một instruction `execute` để:
  - Kiểm tra xem instruction trước đó có phải là `approve` không
  - Xác minh xem nó có đến từ cùng một program không
  - Chỉ thực thi nếu sự phê duyệt là hợp lệ

### Phần 2: Tối ưu hóa với Zero-Copy

Bây giờ tạo một phiên bản xử lý dữ liệu lớn một cách hiệu quả bằng cách sử dụng zero-copy.

Yêu cầu:

- Thêm một struct `LargeApprovalData` với mảng gồm 512 giá trị `u64`
- So sánh `Account<T>` thông thường vs `AccountLoader<T>`
- Đo lường xem phương pháp nào tránh được tràn ngăn xếp. Quan sát cách một `Account<T>` thông thường quy mô lớn gặp lỗi giới hạn ngăn xếp BPF tại thời điểm build, trong khi một `AccountLoader<T>` zero-copy có thể xử lý cùng một kích thước dữ liệu một cách an toàn.

### Phần 3: Remaining Accounts

Xây dựng instruction `multi_send` cho phép gửi SOL đến nhiều địa chỉ cùng lúc mà không cần hardcode danh sách người nhận vào `Accounts` struct.

Yêu cầu:

- Danh sách người nhận được truyền vào qua `remaining_accounts` thay vì khai báo trong struct — đây là cách tránh tốn stack space cho các account có số lượng biến động
- Áp đặt giới hạn: ít nhất 1 người nhận, tối đa 10 người nhận
- Kiểm tra từng account trong `remaining_accounts` phải có `is_writable = true`
- Viết test bao gồm các trường hợp: không có người nhận, quá nhiều người nhận, account không writable, và trường hợp thành công với 3 người nhận

