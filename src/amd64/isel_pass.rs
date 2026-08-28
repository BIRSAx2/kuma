//! AMD64 instruction selection.

use crate::ir::*;
use crate::util::{self, InsBuffer};

use super::regs::*;

#[inline]
fn preg(r: u32) -> Ref {
    Ref::Tmp(TmpId(r))
}

#[inline]
fn slot_ref(s: i32) -> Ref {
    Ref::Slot((s as u32) & 0x1fff_ffff)
}

fn rslot(r: Ref, f: &Fn) -> i32 {
    match r {
        Ref::Tmp(id) if id.0 >= TMP0 => f.tmps[id.0 as usize].slot,
        _ => -1,
    }
}

fn noimm(r: Ref, f: &Fn) -> bool {
    let Ref::Con(id) = r else { return false };
    let c = f.cons[id.0 as usize];
    match c.typ {
        ConType::Addr => false,
        ConType::Bits => {
            let v = c.bits.i();
            v < i32::MIN as i64 || v > i32::MAX as i64
        }
        ConType::Undef => panic!("invalid constant"),
    }
}

fn intern_str(s: &str, f: &mut Fn) -> u32 {
    if let Some(pos) = f.strs.iter().position(|x| x == s) {
        pos as u32
    } else {
        let id = f.strs.len() as u32;
        f.strs.push(s.to_string());
        id
    }
}

fn float_const_ref(r: Ref, k: Cls, f: &mut Fn, apple: bool) -> Ref {
    let Ref::Con(id) = r else { return r };
    let c = f.cons[id.0 as usize];
    let bytes = c.bits.i().to_le_bytes();
    let n = crate::emit::stashbits(if k.is_wide() {
        &bytes[..8]
    } else {
        &bytes[..4]
    });
    let sym = format!("\"{}fp{}\"", if apple { "L" } else { ".L" }, n);
    let sid = intern_str(&sym, f);
    let con = Con {
        typ: ConType::Addr,
        sym: Sym {
            typ: SymType::Glo,
            id: sid,
        },
        bits: ConBits::default(),
        flt: 0,
    };
    util::newcon(&con, f)
}

#[derive(Copy, Clone)]
struct ArgUse {
    cls: Cls,
    op: Option<Op>,
    index: usize,
    phi: bool,
}

impl ArgUse {
    fn instruction(cls: Cls, op: Op, index: usize) -> Self {
        Self {
            cls,
            op: Some(op),
            index,
            phi: false,
        }
    }

    fn phi(cls: Cls) -> Self {
        Self {
            cls,
            op: None,
            index: 0,
            phi: true,
        }
    }
}

fn fixarg_ref(pr: &mut Ref, arg_use: ArgUse, f: &mut Fn, buf: &mut InsBuffer, apple: bool) {
    let ArgUse {
        cls: k,
        op,
        index: argn,
        phi,
    } = arg_use;
    let r0 = *pr;
    if let Ref::Con(_) = r0 {
        if k.base() == 1 {
            let r1 = util::newtmp("isel", k, f);
            buf.emit(Op::Load, k, r1, float_const_ref(r0, k, f, apple), Ref::R);
            *pr = r1;
            return;
        }
        if phi {
            return;
        }
        if op != Some(Op::Copy) && k == Cls::Kl && noimm(r0, f) {
            let r1 = util::newtmp("isel", Cls::Kl, f);
            buf.emit(Op::Copy, Cls::Kl, r1, r0, Ref::R);
            *pr = r1;
            return;
        }
        if apple
            && !matches!(op, Some(Op::Call))
            && !matches!(op, Some(o) if o.is_load())
            && !(matches!(op, Some(o) if o.is_store()) && argn == 1)
            && let Ref::Con(id) = r0
            && f.cons[id.0 as usize].typ == ConType::Addr
        {
            let r1 = util::newtmp("isel", Cls::Kl, f);
            buf.emit(Op::Addr, Cls::Kl, r1, r0, Ref::R);
            *pr = r1;
            return;
        }
    }

    let s = rslot(r0, f);
    if s != -1 {
        let r1 = util::newtmp("isel", Cls::Kl, f);
        buf.emit(Op::Addr, Cls::Kl, r1, slot_ref(s), Ref::R);
        *pr = r1;
    }
}

fn fixarg_ins(buf: &mut InsBuffer, idx: usize, arg_use: ArgUse, f: &mut Fn, apple: bool) {
    if arg_use.cls == Cls::Kx {
        return;
    }
    let mut r = buf.as_slice()[idx].arg[arg_use.index];
    fixarg_ref(&mut r, arg_use, f, buf, apple);
    buf.at_mut(idx).arg[arg_use.index] = r;
}

fn cmp_swap_kind(kind: i32) -> i32 {
    match kind {
        0 => 0,
        1 => 1,
        2 => 4,
        3 => 5,
        4 => 2,
        5 => 3,
        6 => 8,
        7 => 9,
        8 => 6,
        9 => 7,
        10 => 10,
        11 => 13,
        12 => 14,
        13 => 11,
        14 => 12,
        15 => 15,
        16 => 16,
        17 => 17,
        _ => kind,
    }
}

fn flag_op(kind: i32) -> Op {
    unsafe { std::mem::transmute::<u16, Op>(Op::Flagieq as u16 + kind as u16) }
}

fn cmpswap(arg: &[Ref; 2], kind: i32) -> bool {
    match kind {
        x if x == N_CMP_I as i32 + CmpF::Cflt as i32 || x == N_CMP_I as i32 + CmpF::Cfle as i32 => {
            true
        }
        x if x == N_CMP_I as i32 + CmpF::Cfgt as i32 || x == N_CMP_I as i32 + CmpF::Cfge as i32 => {
            false
        }
        _ => matches!(arg[0], Ref::Con(_)),
    }
}

fn selcmp(arg: &mut [Ref; 2], k: Cls, swap: bool, f: &mut Fn, buf: &mut InsBuffer, apple: bool) {
    if swap {
        arg.swap(0, 1);
    }
    let cmp_idx = buf.len();
    buf.emit(Op::Xcmp, k, Ref::R, arg[1], arg[0]);
    if matches!(arg[0], Ref::Con(_)) {
        let t = util::newtmp("isel", k, f);
        buf.at_mut(cmp_idx).arg[1] = t;
        buf.emit(Op::Copy, k, t, arg[0], Ref::R);
        let copy_idx = buf.len() - 1;
        fixarg_ins(buf, copy_idx, ArgUse::instruction(k, Op::Copy, 0), f, apple);
    } else {
        fixarg_ins(buf, cmp_idx, ArgUse::instruction(k, Op::Xcmp, 1), f, apple);
    }
    fixarg_ins(buf, cmp_idx, ArgUse::instruction(k, Op::Xcmp, 0), f, apple);
}

fn seljmp(bid: BlkId, f: &mut Fn, buf: &mut InsBuffer, apple: bool) {
    let j = f.blks[bid.0 as usize].jmp.typ;
    if matches!(j, Jmp::Ret0 | Jmp::Jmp_ | Jmp::Hlt) {
        return;
    }
    assert_eq!(j, Jmp::Jnz);
    let r = f.blks[bid.0 as usize].jmp.arg;
    f.blks[bid.0 as usize].jmp.arg = Ref::R;
    if f.blks[bid.0 as usize].s1 == f.blks[bid.0 as usize].s2 {
        util::chuse(r, -1, f);
        f.blks[bid.0 as usize].jmp.typ = Jmp::Jmp_;
        f.blks[bid.0 as usize].s2 = None;
        return;
    }
    let mut arg = [r, Ref::CON_Z];
    selcmp(&mut arg, Cls::Kw, false, f, buf, apple);
    f.blks[bid.0 as usize].jmp.typ = Jmp::Jfine;
}

fn assign_fast_allocs(f: &mut Fn) {
    let bid = f.start;
    for ins in &mut f.blks[bid.0 as usize].ins {
        let (al, n) = match ins.op {
            Op::Alloc4 => (4i64, 4i64),
            Op::Alloc8 => (8, 8),
            Op::Alloc16 => (16, 16),
            _ => continue,
        };
        let Ref::Con(cid) = ins.arg[0] else { break };
        let sz = f.cons[cid.0 as usize].bits.i();
        assert!(
            sz >= 0 && sz < (i32::MAX - 15) as i64,
            "invalid alloc size {sz}"
        );
        let sz = ((sz + n - 1) & -n) / 4;
        assert!(sz <= i32::MAX as i64 - f.slot as i64, "alloc too large");
        if let Ref::Tmp(tid) = ins.to {
            f.tmps[tid.0 as usize].slot = f.slot;
        }
        f.slot += sz as i32;
        let _ = al;
        ins.op = Op::Nop;
    }
}

fn sel(mut i: Ins, f: &mut Fn, buf: &mut InsBuffer, apple: bool) {
    if let Ref::Tmp(tid) = i.to
        && tid.0 >= TMP0
        && !util::isreg(i.to)
        && !util::isreg(i.arg[0])
        && !util::isreg(i.arg[1])
        && f.tmps[tid.0 as usize].nuse == 0
    {
        util::chuse(i.arg[0], -1, f);
        util::chuse(i.arg[1], -1, f);
        return;
    }

    let k = i.cls;
    match i.op {
        Op::Div | Op::Rem | Op::Udiv | Op::Urem if k.base() == 0 => {
            let (dst, live) = if matches!(i.op, Op::Div | Op::Udiv) {
                (preg(RAX), preg(RDX))
            } else {
                (preg(RDX), preg(RAX))
            };
            buf.emit(Op::Copy, k, i.to, dst, Ref::R);
            buf.emit(Op::Copy, k, Ref::R, live, Ref::R);
            let rhs = if matches!(i.arg[1], Ref::Con(_)) {
                util::newtmp("isel", k, f)
            } else {
                i.arg[1]
            };
            assert!(rslot(rhs, f) == -1, "unlikely argument in division");
            if matches!(i.op, Op::Div | Op::Rem) {
                buf.emit(Op::Xidiv, k, Ref::R, rhs, Ref::R);
                buf.emit(Op::Sign, k, preg(RDX), preg(RAX), Ref::R);
            } else {
                buf.emit(Op::Xdiv, k, Ref::R, rhs, Ref::R);
                buf.emit(Op::Copy, k, preg(RDX), Ref::CON_Z, Ref::R);
            }
            buf.emit(Op::Copy, k, preg(RAX), i.arg[0], Ref::R);
            let idx = buf.len() - 1;
            fixarg_ins(buf, idx, ArgUse::instruction(k, Op::Copy, 0), f, apple);
            if matches!(i.arg[1], Ref::Con(_)) {
                buf.emit(Op::Copy, k, rhs, i.arg[1], Ref::R);
                let idx = buf.len() - 1;
                fixarg_ins(buf, idx, ArgUse::instruction(k, Op::Copy, 0), f, apple);
            }
        }
        Op::Sar | Op::Shr | Op::Shl if !matches!(i.arg[1], Ref::Con(_)) => {
            let rhs = i.arg[1];
            assert!(rslot(rhs, f) == -1, "unlikely argument in shift");
            i.arg[1] = preg(RCX);
            buf.emit(Op::Copy, Cls::Kw, Ref::R, preg(RCX), Ref::R);
            buf.emiti(i);
            let op_idx = buf.len() - 1;
            buf.emit(Op::Copy, Cls::Kw, preg(RCX), rhs, Ref::R);
            fixarg_ins(
                buf,
                op_idx,
                ArgUse::instruction(crate::ir::argcls(&i, 0), i.op, 0),
                f,
                apple,
            );
        }
        Op::Uwtof => {
            let r0 = util::newtmp("utof", Cls::Kl, f);
            buf.emit(Op::Sltof, k, i.to, r0, Ref::R);
            buf.emit(Op::Extuw, Cls::Kl, r0, i.arg[0], Ref::R);
            let idx = buf.len() - 1;
            fixarg_ins(
                buf,
                idx,
                ArgUse::instruction(Cls::Kl, Op::Extuw, 0),
                f,
                apple,
            );
        }
        Op::Ultof => {
            let kc = if k == Cls::Ks { Cls::Kw } else { Cls::Kl };
            let sh = if k == Cls::Ks { 23 } else { 52 };
            let mut tmp = [Ref::R; 7];
            for t in tmp.iter_mut().take(4) {
                *t = util::newtmp("utof", Cls::Kl, f);
            }
            for t in tmp.iter_mut().skip(4) {
                *t = util::newtmp("utof", kc, f);
            }
            let r0 = util::newtmp("utof", k, f);
            buf.emit(Op::Cast, k, i.to, tmp[6], Ref::R);
            buf.emit(Op::Add, kc, tmp[6], tmp[4], tmp[5]);
            buf.emit(Op::Shl, kc, tmp[5], tmp[1], util::getcon(sh, f));
            buf.emit(Op::Cast, kc, tmp[4], r0, Ref::R);
            buf.emit(Op::Sltof, k, r0, tmp[3], Ref::R);
            buf.emit(Op::Or, Cls::Kl, tmp[3], tmp[0], tmp[2]);
            buf.emit(Op::Shr, Cls::Kl, tmp[2], i.arg[0], tmp[1]);
            buf.emit(Op::Shr, Cls::Kl, tmp[1], i.arg[0], util::getcon(63, f));
            let idx = buf.len() - 1;
            fixarg_ins(buf, idx, ArgUse::instruction(Cls::Kl, Op::Shr, 0), f, apple);
            buf.emit(Op::And, Cls::Kl, tmp[0], i.arg[0], util::getcon(1, f));
            let idx = buf.len() - 1;
            fixarg_ins(buf, idx, ArgUse::instruction(Cls::Kl, Op::And, 0), f, apple);
        }
        Op::Stoui | Op::Dtoui => {
            let (mut op, kc, addc) = if i.op == Op::Stoui {
                (Op::Stosi, Cls::Ks, util::getcon(0xdf000000u32 as i64, f))
            } else {
                (
                    Op::Dtosi,
                    Cls::Kd,
                    util::getcon(0xc3e0000000000000u64 as i64, f),
                )
            };
            if k == Cls::Kw {
                let r0 = util::newtmp("ftou", Cls::Kl, f);
                buf.emit(Op::Copy, Cls::Kw, i.to, r0, Ref::R);
                i.cls = Cls::Kl;
                i.to = r0;
                op = if i.op == Op::Stoui {
                    Op::Stosi
                } else {
                    Op::Dtosi
                };
                buf.emiti(Ins {
                    op,
                    cls: i.cls,
                    to: i.to,
                    arg: i.arg,
                });
                let idx = buf.len() - 1;
                fixarg_ins(buf, idx, ArgUse::instruction(kc, op, 0), f, apple);
                return;
            }
            let r0 = util::newtmp("ftou", kc, f);
            let mut tmp = [Ref::R; 4];
            for t in &mut tmp {
                *t = util::newtmp("ftou", Cls::Kl, f);
            }
            buf.emit(Op::Or, Cls::Kl, i.to, tmp[0], tmp[3]);
            buf.emit(Op::And, Cls::Kl, tmp[3], tmp[2], tmp[1]);
            buf.emit(op, Cls::Kl, tmp[2], r0, Ref::R);
            buf.emit(Op::Add, kc, r0, addc, i.arg[0]);
            let idx = buf.len() - 1;
            fixarg_ins(buf, idx, ArgUse::instruction(kc, Op::Add, 0), f, apple);
            fixarg_ins(buf, idx, ArgUse::instruction(kc, Op::Add, 1), f, apple);
            buf.emit(Op::Sar, Cls::Kl, tmp[1], tmp[0], util::getcon(63, f));
            buf.emit(op, Cls::Kl, tmp[0], i.arg[0], Ref::R);
            let idx = buf.len() - 1;
            fixarg_ins(buf, idx, ArgUse::instruction(Cls::Kl, op, 0), f, apple);
        }
        Op::Nop => {}
        Op::Stored | Op::Stores => {
            if matches!(i.arg[0], Ref::Con(_)) {
                i.op = if i.op == Op::Stored {
                    Op::Storel
                } else {
                    Op::Storew
                };
            }
            buf.emiti(i);
            let idx = buf.len() - 1;
            fixarg_ins(
                buf,
                idx,
                ArgUse::instruction(crate::ir::argcls(&i, 0), i.op, 0),
                f,
                apple,
            );
            fixarg_ins(
                buf,
                idx,
                ArgUse::instruction(crate::ir::argcls(&i, 1), i.op, 1),
                f,
                apple,
            );
        }
        Op::Alloc4 | Op::Alloc8 | Op::Alloc16 => {
            util::salloc(i.to, i.arg[0], f, buf);
        }
        _ => {
            if let Some((mut kind, kc)) = util::iscmp(i.op) {
                let cmp_cls = Cls::from_i8(kc as i8);
                match kind {
                    x if x == N_CMP_I as i32 + CmpF::Cfeq as i32 => {
                        let r0 = util::newtmp("isel", Cls::Kw, f);
                        let r1 = util::newtmp("isel", Cls::Kw, f);
                        buf.emit(Op::And, Cls::Kw, i.to, r0, r1);
                        buf.emit(
                            flag_op(N_CMP_I as i32 + CmpF::Cfo as i32),
                            i.cls,
                            r1,
                            Ref::R,
                            Ref::R,
                        );
                        i.to = r0;
                    }
                    x if x == N_CMP_I as i32 + CmpF::Cfne as i32 => {
                        let r0 = util::newtmp("isel", Cls::Kw, f);
                        let r1 = util::newtmp("isel", Cls::Kw, f);
                        buf.emit(Op::Or, Cls::Kw, i.to, r0, r1);
                        buf.emit(
                            flag_op(N_CMP_I as i32 + CmpF::Cfuo as i32),
                            i.cls,
                            r1,
                            Ref::R,
                            Ref::R,
                        );
                        i.to = r0;
                    }
                    _ => {}
                }
                let swap = cmpswap(&i.arg, kind);
                if swap {
                    kind = cmp_swap_kind(kind);
                }
                buf.emit(flag_op(kind), i.cls, i.to, Ref::R, Ref::R);
                selcmp(&mut i.arg, cmp_cls, swap, f, buf, apple);
                return;
            }

            buf.emiti(i);
            let idx = buf.len() - 1;
            fixarg_ins(
                buf,
                idx,
                ArgUse::instruction(crate::ir::argcls(&i, 0), i.op, 0),
                f,
                apple,
            );
            fixarg_ins(
                buf,
                idx,
                ArgUse::instruction(crate::ir::argcls(&i, 1), i.op, 1),
                f,
                apple,
            );
        }
    }
}

pub fn isel(f: &mut Fn, t: &Target) {
    assign_fast_allocs(f);
    let apple = t.apple;
    let rpo = f.rpo.clone();

    for &bid in &rpo {
        let mut buf = InsBuffer::new();
        let succs = [f.blks[bid.0 as usize].s1, f.blks[bid.0 as usize].s2];
        for succ in succs.into_iter().flatten() {
            for pi in 0..f.blks[succ.0 as usize].phi.len() {
                let blks = f.blks[succ.0 as usize].phi[pi].blks.clone();
                for (ai, pbid) in blks.iter().enumerate() {
                    if *pbid == bid {
                        let cls = f.blks[succ.0 as usize].phi[pi].cls;
                        let mut r = f.blks[succ.0 as usize].phi[pi].args[ai];
                        fixarg_ref(&mut r, ArgUse::phi(cls), f, &mut buf, apple);
                        f.blks[succ.0 as usize].phi[pi].args[ai] = r;
                    }
                }
            }
        }
        seljmp(bid, f, &mut buf, apple);
        let ins = f.blks[bid.0 as usize].ins.clone();
        for ins in ins.into_iter().rev() {
            sel(ins, f, &mut buf, apple);
        }
        f.blks[bid.0 as usize].ins = buf.finish();
    }
}
