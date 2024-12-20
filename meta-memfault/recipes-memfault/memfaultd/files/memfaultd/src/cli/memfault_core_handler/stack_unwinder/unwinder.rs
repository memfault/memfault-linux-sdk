//
// Copyright (c) Memfault, Inc.
// See License.txt for details
//! Stack unwinding logic for the Memfault core handler.
//!
//! This module provides the ability to unwind the stack of an application piped into the Memfault
//! core handler. The main goal here is to get the PC of every frame on the stack, so we can
//! later symbolicate it.

use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
};

use super::eh_frame_finder::{EhFrameFinder, EhFrameInfo};
use eyre::{eyre, Result};
use gimli::{
    read::{EhFrame, EhFrameHdr, UnwindSection},
    BaseAddresses, CfaRule, Encoding, EvaluationResult, Expression, Location, NativeEndian, Reader,
    Register, RegisterRule, UnwindContext, Value,
};
use gimli::{EndianSlice, ParsedEhFrameHdr};
use log::debug;

use crate::cli::memfault_core_handler::arch::{
    get_return_register, get_vendor, return_register_idx, set_stack_pointer, WORD_SIZE,
};

// Max stack depth to unwind. This prevents infinite loops in case of corrupted stack frames,
// missing data.
const MAX_STACK_DEPTH: usize = 64;

/// Used to unwind the stack of a crashed process.
///
/// This struct is responsible for unwinding the stack of a crashed process. It is provided
/// with the `.eh_frame_hdr` and `.eh_frame` sections of the ELF file, and it will use them
/// to unwind the stack.
pub struct Unwinder<'a, E: EhFrameFinder> {
    eh_frame_finder: &'a mut E,
}

impl<'a, E: EhFrameFinder> Unwinder<'a, E> {
    pub fn new(eh_frame_finder: &'a mut E) -> Self {
        Self { eh_frame_finder }
    }

    // Given a starting address, unwind the stack and fill the context with the PC of each frame.
    pub fn unwind_stack<R: Read + Seek>(
        &mut self,
        starting_addr: usize,
        ctx: &mut UnwindFrameContext,
        memory: &mut R,
    ) -> Result<()> {
        // Push the initial PC onto the stack
        ctx.pc_stack.push(starting_addr);

        let mut cur_return_addr = starting_addr;
        let mut first_frame = true;
        // Limit the stack depth to prevent infinite loops
        for _ in 0..MAX_STACK_DEPTH {
            let eh_frame_info = match self.eh_frame_finder.find_eh_frame(cur_return_addr) {
                Ok(eh_frame_info) => eh_frame_info,
                Err(_) => {
                    // If we can't find an eh_frame, we've reached the end of the stack, or at
                    // least the end of the stack we can unwind.
                    debug!(
                        "Failed to find eh_frame for address: {:#x}",
                        cur_return_addr
                    );
                    break;
                }
            };
            let unwind_info = eh_frame_from_sections(&eh_frame_info)?;

            // TODO: Implement a naive FDE lookup for binaries without an eh_frame_hdr
            let hdr_table = unwind_info
                .eh_frame_hdr
                .table()
                .ok_or_else(|| eyre!("No table found"))?;

            let mut unwind_ctx = UnwindContext::new();

            // Parse and set up FDE
            let fde = hdr_table.fde_for_address(
                &unwind_info.eh_frame,
                &unwind_info.bases,
                cur_return_addr as _,
                EhFrame::cie_from_offset,
            );
            let fde = match fde {
                Ok(fde) => fde,
                Err(e) => {
                    // If we can't get an FDE, we've reached the end of the stack
                    debug!("Failed to get FDE: {:?}", e);
                    break;
                }
            };
            let unwind_row = fde.unwind_info_for_address(
                &unwind_info.eh_frame,
                &unwind_info.bases,
                &mut unwind_ctx,
                cur_return_addr as _,
            )?;

            if unwind_row.register(return_register_idx()) == RegisterRule::Undefined && !first_frame
            {
                // If the return address rule is undefined, we know this function never returns,
                // so we can exit.
                //
                // Note: there is an edge case here where the first frame may never return, like a
                // rust null ptr write. For architectures that have it we will us the return
                // address directly from the coredump.
                break;
            }

            // Unwind CFA
            let cfa_rule = unwind_row.cfa();
            let cfa = parse_cfa_rule(
                cfa_rule,
                ctx,
                &unwind_info.eh_frame,
                fde.cie().encoding(),
                memory,
            )?;

            // Unwind the registers
            let result = unwind_row.registers().try_for_each(|(register, rule)| {
                parse_register_rule(
                    rule,
                    register,
                    cfa,
                    ctx,
                    memory,
                    fde.cie().encoding(),
                    &unwind_info.eh_frame,
                )
            });
            if let Err(e) = result {
                debug!("Failed to unwind registers: {:?}", e);
                break;
            }

            set_stack_pointer(&mut ctx.registers, cfa);

            cur_return_addr = get_return_register(&ctx.registers)
                .ok_or_else(|| eyre!("Failed to get return address from registers"))?;

            // A return address of 0 indicates the end of the stack
            if cur_return_addr == 0 {
                break;
            }

            // For trampoline functions, the return address is the address of the trampoline.
            // This differs from a normal function where the return address is the instruction
            // after the call
            let pc_offset = !fde.is_signal_trampoline() as usize;
            cur_return_addr -= pc_offset;

            ctx.pc_stack.push(cur_return_addr);

            first_frame = false;
        }

        Ok(())
    }
}

struct UnwindFrameInfo<'a> {
    eh_frame_hdr: ParsedEhFrameHdr<EndianSlice<'a, NativeEndian>>,
    eh_frame: EhFrame<EndianSlice<'a, NativeEndian>>,
    bases: BaseAddresses,
}

fn eh_frame_from_sections(eh_frame_info: &EhFrameInfo) -> Result<UnwindFrameInfo<'_>> {
    let compiled_base_addr = eh_frame_info.compiled_base_addr;
    let runtime_base_addr = eh_frame_info.runtime_base_addr;
    let eh_frame_hdr_addr =
        eh_frame_info.eh_frame_hdr.header.sh_addr as usize - compiled_base_addr + runtime_base_addr;
    let eh_frame_addr =
        eh_frame_info.eh_frame.header.sh_addr as usize - compiled_base_addr + runtime_base_addr;
    let bases = BaseAddresses::default()
        .set_eh_frame_hdr(eh_frame_hdr_addr as _)
        .set_eh_frame(eh_frame_addr as _);

    let eh_frame_hdr = EhFrameHdr::new(eh_frame_info.eh_frame_hdr.data.as_slice(), NativeEndian);
    let parsed_frame_header = eh_frame_hdr.parse(&bases, WORD_SIZE as u8)?;
    let mut eh_frame = EhFrame::new(&eh_frame_info.eh_frame.data, NativeEndian);
    eh_frame.set_vendor(get_vendor());

    let unwind_frame_info = UnwindFrameInfo {
        eh_frame_hdr: parsed_frame_header,
        eh_frame,
        bases,
    };

    Ok(unwind_frame_info)
}

/// Calculate the Canonical Frame Address (CFA) based on the provided rule.
///
/// This function will calculate the CFA for the upcoming frame. The rules are outlined in section
/// 6.4.1 of the DWARF standard:
/// https://dwarfstd.org/doc/DWARF5.pdf
fn parse_cfa_rule<R, S, M>(
    rule: &CfaRule<usize>,
    ctx: &UnwindFrameContext,
    eh_frame: &S,
    encoding: Encoding,
    memory: &mut M,
) -> Result<usize>
where
    R: Reader<Offset = usize>,
    S: UnwindSection<R>,
    M: Read + Seek,
{
    let cfa = match rule {
        CfaRule::RegisterAndOffset { register, offset } => ctx
            .registers
            .get(register)
            .ok_or_else(|| eyre!("Failed to get register value"))?
            .wrapping_add_signed(*offset as isize),
        CfaRule::Expression(expression) => {
            let expression = expression.get(eh_frame)?;
            evaluate_expression(expression, ctx, encoding, memory)?
        }
    };

    Ok(cfa)
}

/// Calculate the new value of a register based on the rule.
///
/// This function will calculate the new value of a register for the upcoming frame. The rules
/// are outlined in section 6.4.2 of the DWARF standard. Note that gimli does a lot of the heavy
/// lifting in decoding the CFA rules for us, so we only need handle a small subset.
/// https://dwarfstd.org/doc/DWARF5.pdf
fn parse_register_rule<R, S, M>(
    rule: &RegisterRule<usize>,
    register: &Register,
    cfa: usize,
    ctx: &mut UnwindFrameContext,
    memory: &mut M,
    encoding: Encoding,
    eh_frame: &S,
) -> Result<()>
where
    R: Reader<Offset = usize>,
    S: UnwindSection<R>,
    M: Read + Seek,
{
    let val = match rule {
        RegisterRule::Undefined | RegisterRule::SameValue => ctx.registers.get(register).copied(),
        RegisterRule::Offset(offset) => {
            let val_addr = cfa.wrapping_add_signed(*offset as isize);
            Some(read_word(memory, val_addr)?)
        }
        RegisterRule::ValOffset(offset) => {
            let cfa_val = read_word(memory, cfa)?;
            Some(cfa_val.wrapping_add_signed(*offset as isize))
        }
        RegisterRule::Register(reg) => ctx.registers.get(reg).copied(),
        RegisterRule::Constant(val) => Some(*val as usize),
        RegisterRule::Expression(expression) => {
            let expression = expression.get(eh_frame)?;
            let address = evaluate_expression(expression, ctx, encoding, memory)?;

            Some(read_word(memory, address)?)
        }
        RegisterRule::ValExpression(expression) => {
            let expression = expression.get(eh_frame)?;
            Some(evaluate_expression(expression, ctx, encoding, memory)?)
        }
        // Fall through case, we should never hit this
        _ => {
            return Err(eyre!("Unsupported register rule: {:?}", rule));
        }
    };

    match val {
        Some(val) => {
            ctx.registers.insert(*register, val);
            Ok(())
        }
        None => Err(eyre!("Failed to parse register rule")),
    }
}

fn evaluate_expression<R, M>(
    expression: Expression<R>,
    ctx: &UnwindFrameContext,
    encoding: Encoding,
    memory: &mut M,
) -> Result<usize>
where
    R: Reader,
    M: Read + Seek,
{
    let mut eval = expression.evaluation(encoding);
    let mut eval_result = eval.evaluate()?;
    loop {
        match eval_result {
            EvaluationResult::Complete => break,
            EvaluationResult::RequiresMemory { address, size, .. } => {
                let val = read_variable_size(memory, address as _, size as _)?;
                eval_result = eval.resume_with_memory(Value::Generic(val as _))?;
            }
            EvaluationResult::RequiresRegister { register, .. } => {
                let val = ctx.registers.get(&register).copied().ok_or_else(|| {
                    eyre!("Failed to get register value for expression evaluation")
                })?;
                eval_result = eval.resume_with_register(Value::Generic(val as _))?;
            }
            // Note: that we only expect RequiresMemory and RequiresRegister as not all
            // expression types are used in CFI entries. See section 6.4.2 of the
            // DWARF standard for more information.
            _ => return Err(eyre!("Unexpected evaluation result: {:?}", eval_result)),
        }
    }

    let location = &eval
        .as_result()
        .last()
        .ok_or(gimli::Error::PopWithEmptyStack)?
        .location;
    match location {
        // We only need to handle the address case. The other cases are not used in CFI entries,
        // as some the dwarf expressions have variable meaning depending on the context. Additionally
        // some of the rules would be circular.
        Location::Address { address } => Ok(*address as usize),
        _ => Err(eyre!(
            "Unexpected stack result location type: {:?}",
            location
        )),
    }
}

/// Read a variable sized value from the provided memory at the given address.
fn read_variable_size<R: Read + Seek>(memory: &mut R, addr: usize, size: u8) -> Result<usize> {
    let mut buf = vec![0u8; size as usize];
    memory.seek(SeekFrom::Start(addr as u64))?;
    memory.read_exact(&mut buf)?;
    Ok(match size {
        1 => {
            let buf = buf[0..1].try_into()?;
            u8::from_ne_bytes(buf) as usize
        }
        2 => {
            let buf = buf[0..2].try_into()?;
            u16::from_ne_bytes(buf) as usize
        }
        4 => {
            let buf = buf[0..4].try_into()?;
            u32::from_ne_bytes(buf) as usize
        }
        #[cfg(target_pointer_width = "64")]
        8 => {
            let buf = buf[0..8].try_into()?;
            u64::from_ne_bytes(buf) as usize
        }
        _ => return Err(eyre!("Unsupported read size: {}", size)),
    })
}

/// Read a word from the provided memory at the given address.
fn read_word<R: Read + Seek>(memory: &mut R, addr: usize) -> Result<usize> {
    read_variable_size(memory, addr, WORD_SIZE as u8)
}

#[derive(Default, Debug)]
pub struct UnwindFrameContext {
    pub registers: HashMap<Register, usize>,
    pub pc_stack: Vec<usize>,
}

#[cfg(test)]
mod test {
    use std::io::Cursor;

    use gimli::{DW_OP_breg0, DW_OP_deref_size, DW_OP_lit0, Encoding, Format, RunTimeEndian};
    use rstest::rstest;

    use super::*;

    const PREV_REGISTER_VAL: usize = 42;

    #[rstest]
    #[case(RegisterRule::Undefined, PREV_REGISTER_VAL)]
    #[case(RegisterRule::SameValue, PREV_REGISTER_VAL)]
    #[case(RegisterRule::Register(Register(0)), PREV_REGISTER_VAL)]
    #[case(RegisterRule::Constant(0xdeadbeef), 0xdeadbeef)]
    #[case(RegisterRule::Offset(8), 0xfeedface)]
    #[case(RegisterRule::ValOffset(21), 42)]
    fn test_register_rule_parsing(
        #[case] register_rule: RegisterRule<usize>,
        #[case] expected: usize,
    ) {
        use std::io::Cursor;

        let mut ctx = UnwindFrameContext::default();
        let memory = vec![21u64, 0xfeedfaceu64];
        let memory_bytes = memory
            .into_iter()
            .flat_map(|x| x.to_ne_bytes().to_vec())
            .collect::<Vec<_>>();
        let mut memory_cursor = Cursor::new(memory_bytes);
        let cfa = 0;
        let register = Register(0);
        ctx.registers.insert(register, PREV_REGISTER_VAL);

        parse_register_rule(
            &register_rule,
            &register,
            cfa,
            &mut ctx,
            &mut memory_cursor,
            encoding(),
            &eh_frame(&[]),
        )
        .unwrap();
        assert_eq!(*ctx.registers.get(&register).unwrap(), expected);
    }

    #[rstest]
    #[case(CfaRule::RegisterAndOffset { register: Register(0), offset: 0xface }, 0xfeedface)]
    fn test_cfa_rule_parsing(#[case] cfa_rule: CfaRule<usize>, #[case] expected: usize) {
        let mut ctx = UnwindFrameContext::default();
        let mut memory = Cursor::new(vec![0u8]);
        ctx.registers.insert(Register(0), 0xfeed0000);

        let cfa = parse_cfa_rule(&cfa_rule, &ctx, &eh_frame(&[]), encoding(), &mut memory).unwrap();
        assert_eq!(cfa, expected);
    }

    #[test]
    fn test_register_access_expression() {
        // Expression that reads a value from a register
        // DW_OP_breg0 reads from register 0 and adds a signed offset
        let expression_bytes = [
            DW_OP_breg0.0,
            0x00, // SLEB128 offset of 0
        ];

        let endian = RunTimeEndian::Little;
        let expression_data = EndianSlice::new(&expression_bytes, endian);
        let expression = Expression(expression_data);
        let mut memory = Cursor::new(vec![0u8]);

        const REG_VAL: usize = 42;
        let registers = [(Register(0), REG_VAL)].iter().cloned().collect();
        let ctx = UnwindFrameContext {
            registers,
            ..Default::default()
        };

        let value = evaluate_expression(expression, &ctx, encoding(), &mut memory).unwrap();

        assert_eq!(value, REG_VAL);
    }

    #[test]
    fn test_memory_access_expression() {
        // Expression that reads a 4-byte value from memory at address 0
        let expression_bytes = [
            DW_OP_lit0.0,       // Push 0 (address)
            DW_OP_deref_size.0, // Dereference memory
            8,                  // Size of memory read (8 bytes)
        ];

        let endian = RunTimeEndian::Little;
        let expression_data = EndianSlice::new(&expression_bytes, endian);
        let expression = Expression(expression_data);
        let mut memory = Cursor::new(vec![0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        let ctx = UnwindFrameContext::default();

        let value = evaluate_expression(expression, &ctx, encoding(), &mut memory).unwrap();

        assert_eq!(value, 0x42);
    }

    #[rstest]
    #[case(1, 0x01)]
    #[case(2, 0x0101)]
    #[case(4, 0x01010101)]
    #[case(8, 0x0101010101010101)]
    fn test_variable_size_mem_read(#[case] size: u8, #[case] expected: usize) {
        let mut memory = Cursor::new(vec![1u8; 8]);
        let addr = 0;
        let val = read_variable_size(&mut memory, addr, size).unwrap();
        assert_eq!(val, expected);
    }

    fn encoding() -> Encoding {
        Encoding {
            format: Format::Dwarf32,
            version: 4,
            address_size: 8,
        }
    }

    /// Helper function to create an EhFrame object from a byte slice
    fn eh_frame(data: &'static [u8]) -> EhFrame<EndianSlice<'static, NativeEndian>> {
        let endian = NativeEndian;
        EhFrame::new(data, endian)
    }
}
