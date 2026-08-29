//! AArch64 instruction selection.

use crate::codegen::emission::FPStash;
use crate::ir::builder::{self, InsBuffer};
use crate::ir::internal::*;

use super::registers::*;

/// Intern a string into the function's strs table, returning its index.
/// If the string already exists, return the existing index.
fn intern_str(s: &str, f: &mut Fn) -> u32 {
    if let Some(pos) = f.strs.iter().position(|existing| existing == s) {
        return pos as u32;
    }
    let id = f.strs.len() as u32;
    f.strs.push(s.to_string());
    id
}

/// Immediate classification for ARM64 add/sub encoding.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum Imm {
    Other,
    Plo12, // positive, fits in low 12 bits
    Phi12, // positive, fits in high 12 bits (shifted by 12)
    Plo24, // positive, fits in 24 bits (lo12 + hi12)
    Nlo12, // negative, absolute fits in low 12 bits
    Nhi12, // negative, absolute fits in high 12 bits
    Nlo24, // negative, absolute fits in 24 bits
}

/// Classify an integer constant for ARM64 immediate encoding.
fn imm_classify(c: &Con, k: Cls) -> (Imm, i64) {
    if c.typ != ConType::Bits {
        return (Imm::Other, 0);
    }
    let mut n = c.bits.i();
    if k == Cls::Kw {
        n = n as i32 as i64;
    }
    let mut base = Imm::Plo12;
    if n < 0 {
        base = Imm::Nlo12;
        n = -n;
    }
    if (n & 0x000fff) == n {
        return (base, n);
    }
    if (n & 0xfff000) == n {
        let shifted = match base {
            Imm::Plo12 => Imm::Phi12,
            _ => Imm::Nhi12,
        };
        return (shifted, n);
    }
    if (n & 0xffffff) == n {
        let composite = match base {
            Imm::Plo12 => Imm::Plo24,
            _ => Imm::Nlo24,
        };
        return (composite, n);
    }
    (Imm::Other, n)
}

/// Check if a value is a valid ARM64 logical immediate.
pub fn logimm(x: u64, k: Cls) -> bool {
    let mut x = x;
    if k == Cls::Kw {
        x = (x & 0xffffffff) | (x << 32);
    }
    if x & 1 != 0 {
        x = !x;
    }
    if x == 0 {
        return false;
    }
    if x == 0xaaaaaaaaaaaaaaaa {
        return true;
    }
    let mut n;
    n = x & 0xf;
    if 0x1111111111111111u64.wrapping_mul(n) == x {
        return (n & n.wrapping_add(n & n.wrapping_neg())) == 0;
    }
    n = x & 0xff;
    if 0x0101010101010101u64.wrapping_mul(n) == x {
        return (n & n.wrapping_add(n & n.wrapping_neg())) == 0;
    }
    n = x & 0xffff;
    if 0x0001000100010001u64.wrapping_mul(n) == x {
        return (n & n.wrapping_add(n & n.wrapping_neg())) == 0;
    }
    n = x & 0xffffffff;
    if 0x0000000100000001u64.wrapping_mul(n) == x {
        return (n & n.wrapping_add(n & n.wrapping_neg())) == 0;
    }
    n = x;
    (n & n.wrapping_add(n & n.wrapping_neg())) == 0
}

/// Fix up a reference: load constants into temps, resolve stack slots.
fn fixarg(
    pr: &mut Ref,
    k: Cls,
    phi: bool,
    f: &mut Fn,
    buf: &mut InsBuffer,
    apple: bool,
    literals: &mut FPStash,
) {
    let r0 = *pr;
    match r0 {
        Ref::Con(cid) => {
            let c = f.cons[cid.0 as usize];
            if apple && c.typ == ConType::Addr && c.sym.typ == SymType::Thr {
                let r1 = builder::newtmp("isel", Cls::Kl, f);
                *pr = r1;
                if c.bits.i() != 0 {
                    let r2 = builder::newtmp("isel", Cls::Kl, f);
                    let cc = Con {
                        typ: ConType::Bits,
                        bits: ConBits::from_i64(c.bits.i()),
                        ..Con::default()
                    };
                    let r3 = builder::newcon(&cc, f);
                    buf.emit(Op::Add, Cls::Kl, r1, r2, r3);
                    let r1_inner = r2;
                    buf.emit(Op::Copy, Cls::Kl, r1_inner, Ref::Tmp(TmpId(R0)), Ref::R);
                } else {
                    buf.emit(Op::Copy, Cls::Kl, r1, Ref::Tmp(TmpId(R0)), Ref::R);
                }
                let r1_fn = builder::newtmp("isel", Cls::Kl, f);
                let r2_addr = builder::newtmp("isel", Cls::Kl, f);
                buf.emit(Op::Call, Cls::Kw, Ref::R, r1_fn, Ref::Call(33));
                buf.emit(Op::Copy, Cls::Kl, Ref::Tmp(TmpId(R0)), r2_addr, Ref::R);
                buf.emit(Op::Load, Cls::Kl, r1_fn, r2_addr, Ref::R);
                let mut cc2 = c;
                cc2.bits = ConBits::from_i64(0);
                let r3_2 = builder::newcon(&cc2, f);
                buf.emit(Op::Copy, Cls::Kl, r2_addr, r3_2, Ref::R);
                return;
            }
            if k.base() == 0 && phi {
                return;
            }
            let r1 = builder::newtmp("isel", k, f);
            if k.base() == 0 {
                buf.emit(Op::Copy, k, r1, r0, Ref::R);
            } else {
                let nbytes = if k.is_wide() { 8 } else { 4 };
                let bits_bytes = c.bits.i().to_le_bytes();
                let n = literals.stash(&bits_bytes[..nbytes]);
                let asloc = if apple { "L" } else { ".L" };
                let label = format!("\"{asloc}fp{n}\"");
                let sym_id = intern_str(&label, f);
                let addr_con = Con {
                    typ: ConType::Addr,
                    sym: Sym {
                        typ: SymType::Glo,
                        id: sym_id,
                    },
                    ..Con::default()
                };
                let r2 = builder::newtmp("isel", Cls::Kl, f);
                let addr_ref = builder::newcon(&addr_con, f);
                buf.emit(Op::Load, k, r1, r2, Ref::R);
                buf.emit(Op::Copy, Cls::Kl, r2, addr_ref, Ref::R);
            }
            *pr = r1;
        }
        Ref::Tmp(tid) => {
            let Some(slot) = f.tmps[tid.0 as usize].slot else {
                return;
            };
            let r1 = builder::newtmp("isel", Cls::Kl, f);
            buf.emit(Op::Addr, Cls::Kl, r1, Ref::Slot(slot), Ref::R);
            *pr = r1;
        }
        _ => {}
    }
}

/// Select a comparison instruction, folding negative constants to cmn.
/// Returns true if operands were swapped.
fn selcmp(
    arg: &mut [Ref; 2],
    k: Cls,
    f: &mut Fn,
    buf: &mut InsBuffer,
    apple: bool,
    literals: &mut FPStash,
) -> bool {
    if k.base() == 1 {
        let cmp_idx = buf.len();
        buf.emit(Op::Afcmp, k, Ref::R, arg[0], arg[1]);
        let mut a0 = arg[0];
        let mut a1 = arg[1];
        fixarg(&mut a0, k, false, f, buf, apple, literals);
        fixarg(&mut a1, k, false, f, buf, apple, literals);
        buf.at_mut(cmp_idx).arg[0] = a0;
        buf.at_mut(cmp_idx).arg[1] = a1;
        return false;
    }

    let swap = matches!(arg[0], Ref::Con(_));
    if swap {
        arg.swap(0, 1);
    }

    let mut fix = true;
    let mut cmp = Op::Acmp;
    let mut r = arg[1];

    if let Ref::Con(cid) = r {
        let c = f.cons[cid.0 as usize];
        let (imm_cls, n) = imm_classify(&c, k);
        match imm_cls {
            Imm::Plo12 | Imm::Phi12 => {
                fix = false;
            }
            Imm::Nlo12 | Imm::Nhi12 => {
                cmp = Op::Acmn;
                r = builder::getcon(n, f);
                fix = false;
            }
            _ => {}
        }
    }

    let cmp_idx = buf.len();
    buf.emit(cmp, k, Ref::R, arg[0], r);
    let mut a0 = arg[0];
    fixarg(&mut a0, k, false, f, buf, apple, literals);
    buf.at_mut(cmp_idx).arg[0] = a0;
    if fix {
        let mut a1 = r;
        fixarg(&mut a1, k, false, f, buf, apple, literals);
        buf.at_mut(cmp_idx).arg[1] = a1;
    }

    swap
}

/// Check if a reference is a callable target (tmp or addr constant with offset 0).
fn callable(r: Ref, f: &Fn) -> bool {
    match r {
        Ref::Tmp(_) => true,
        Ref::Con(cid) => {
            let c = &f.cons[cid.0 as usize];
            c.typ == ConType::Addr && c.bits.i() == 0
        }
        _ => false,
    }
}

fn flag_op(condition: u16) -> Op {
    const FLAGS: [Op; N_CMP] = [
        Op::Flagieq,
        Op::Flagine,
        Op::Flagisge,
        Op::Flagisgt,
        Op::Flagisle,
        Op::Flagislt,
        Op::Flagiuge,
        Op::Flagiugt,
        Op::Flagiule,
        Op::Flagiult,
        Op::Flagfeq,
        Op::Flagfge,
        Op::Flagfgt,
        Op::Flagfle,
        Op::Flagflt,
        Op::Flagfne,
        Op::Flagfo,
        Op::Flagfuo,
    ];
    FLAGS[condition as usize]
}

fn conditional_jump(condition: u16) -> Jmp {
    const JUMPS: [Jmp; N_CMP] = [
        Jmp::Jfieq,
        Jmp::Jfine,
        Jmp::Jfisge,
        Jmp::Jfisgt,
        Jmp::Jfisle,
        Jmp::Jfislt,
        Jmp::Jfiuge,
        Jmp::Jfiugt,
        Jmp::Jfiule,
        Jmp::Jfiult,
        Jmp::Jffeq,
        Jmp::Jffge,
        Jmp::Jffgt,
        Jmp::Jffle,
        Jmp::Jfflt,
        Jmp::Jffne,
        Jmp::Jffo,
        Jmp::Jffuo,
    ];
    JUMPS[condition as usize]
}

/// Select instruction `i`, emitting lowered form into `buf`.
fn sel(i: Ins, f: &mut Fn, buf: &mut InsBuffer, apple: bool, literals: &mut FPStash) {
    if i.op.is_alloc() {
        let salloc_idx = buf.len();
        builder::salloc(i.to, i.arg[0], f, buf);
        let mut a0 = buf.at_mut(salloc_idx).arg[0];
        fixarg(&mut a0, Cls::Kl, false, f, buf, apple, literals);
        buf.at_mut(salloc_idx).arg[0] = a0;
        return;
    }

    if let Some((cc, ck)) = builder::iscmp(i.op) {
        let k = Cls::from_i8(ck as i8);
        let flag_idx = buf.len();
        buf.emit(flag_op(cc as u16), i.cls, i.to, Ref::R, Ref::R);
        let mut arg = i.arg;
        if selcmp(&mut arg, k, f, buf, apple, literals) {
            let swapped = cc_swap(cc as u16);
            buf.at_mut(flag_idx).op = flag_op(swapped);
        }
        return;
    }

    if i.op == Op::Call && callable(i.arg[0], f) {
        buf.emiti(i);
        return;
    }

    if i.op != Op::Nop {
        let ins_idx = buf.len();
        buf.emiti(i);
        let ac0 = argcls(&i, 0);
        let ac1 = argcls(&i, 1);
        let mut a0 = i.arg[0];
        let mut a1 = i.arg[1];
        fixarg(&mut a0, ac0, false, f, buf, apple, literals);
        fixarg(&mut a1, ac1, false, f, buf, apple, literals);
        buf.at_mut(ins_idx).arg[0] = a0;
        buf.at_mut(ins_idx).arg[1] = a1;
    }
}

/// Select jumps: fuse comparison+branch into conditional jumps.
fn seljmp(bid: BlkId, f: &mut Fn, buf: &mut InsBuffer, apple: bool, literals: &mut FPStash) {
    let jmp = f.blks[bid.0 as usize].jmp.typ;
    if jmp != Jmp::Jnz {
        return;
    }

    let r = f.blks[bid.0 as usize].jmp.arg;
    f.blks[bid.0 as usize].jmp.arg = Ref::R;

    let ins = &f.blks[bid.0 as usize].ins;
    let mut ir_idx: Option<usize> = None;
    for idx in (0..ins.len()).rev() {
        if ins[idx].to == r {
            ir_idx = Some(idx);
            break;
        }
    }

    let use_count = if let Ref::Tmp(tid) = r {
        f.tmps[tid.0 as usize].nuse
    } else {
        0
    };

    if let Some(idx) = ir_idx
        && use_count == 1
        && let Some((cc, ck)) = builder::iscmp(ins[idx].op)
    {
        let k = Cls::from_i8(ck as i8);
        let mut arg = ins[idx].arg;
        let swapped = selcmp(&mut arg, k, f, buf, apple, literals);
        let final_cc = if swapped {
            cc_swap(cc as u16)
        } else {
            cc as u16
        };
        f.blks[bid.0 as usize].jmp.typ = conditional_jump(final_cc);
        f.blks[bid.0 as usize].ins[idx].op = Op::Nop;
        return;
    }

    let mut arg = [r, Ref::CON_Z];
    selcmp(&mut arg, Cls::Kw, f, buf, apple, literals);
    f.blks[bid.0 as usize].jmp.typ = conditional_jump(1); // Cine = 1
}

/// ARM64 instruction selection pass.
pub fn isel(f: &mut Fn, t: &Target, literals: &mut FPStash) {
    let apple = t.apple;

    let start = f.start;
    {
        let blk = &f.blks[start.0 as usize];
        let ins: Vec<Ins> = blk.ins.clone();

        for (al, n_align) in [(Op::Alloc4, 4i64), (Op::Alloc8, 8), (Op::Alloc16, 16)] {
            for (idx, i) in ins.iter().enumerate() {
                if i.op == al {
                    if !matches!(i.arg[0], Ref::Con(_)) {
                        break; // dynamic size: can't fast-alloc
                    }
                    let sz = f.cons[i.arg[0].val() as usize].bits.i();
                    if sz < 0 || sz >= i32::MAX as i64 - 15 {
                        panic!("invalid alloc size {sz}");
                    }
                    let sz = (sz + n_align - 1) & !(n_align - 1);
                    let sz = sz / 4;
                    if let Ref::Tmp(tid) = i.to {
                        f.tmps[tid.0 as usize].slot = Some(StackSlot::from_signed(f.slot));
                    }
                    f.slot += sz as i32;
                    f.blks[start.0 as usize].ins[idx].op = Op::Nop;
                }
            }
        }
    }

    let rpo: Vec<BlkId> = f.rpo.clone();
    for &bid in &rpo {
        let mut buf = InsBuffer::new();

        let s1 = f.blks[bid.0 as usize].s1;
        let s2 = f.blks[bid.0 as usize].s2;
        let succs: Vec<BlkId> = [s1, s2].iter().filter_map(|s| *s).collect();

        for &sb in &succs {
            let phis: Vec<Phi> = f.blks[sb.0 as usize].phi.clone();
            for phi in &phis {
                for (n, &pblk) in phi.blks.iter().enumerate() {
                    if pblk == bid {
                        let mut arg_n = phi.args[n];
                        fixarg(&mut arg_n, phi.cls, true, f, &mut buf, apple, literals);
                        f.blks[sb.0 as usize].phi.iter_mut().for_each(|p| {
                            if p.to == phi.to && n < p.args.len() {
                                p.args[n] = arg_n;
                            }
                        });
                    }
                }
            }
        }

        seljmp(bid, f, &mut buf, apple, literals);

        let ins = std::mem::take(&mut f.blks[bid.0 as usize].ins);
        for i in ins.iter().rev() {
            sel(*i, f, &mut buf, apple, literals);
        }

        f.blks[bid.0 as usize].ins = buf.finish();
    }
}
