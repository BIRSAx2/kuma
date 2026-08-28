//! AMD64 ABI lowering.

use crate::ir::*;
use crate::util::{self, InsBuffer};

use super::regs::*;

#[derive(Clone)]
struct AClass {
    type_idx: Option<usize>,
    inmem: i32,
    align: i32,
    size: u32,
    cls: [Cls; 2],
    refs: [Ref; 2],
}

impl Default for AClass {
    fn default() -> Self {
        Self {
            type_idx: None,
            inmem: 0,
            align: 0,
            size: 0,
            cls: [Cls::Kx; 2],
            refs: [Ref::R; 2],
        }
    }
}

#[inline]
fn preg(r: u32) -> Ref {
    Ref::Tmp(TmpId(r))
}

#[inline]
fn slot_ref(s: i32) -> Ref {
    Ref::Slot((s as u32) & 0x1fff_ffff)
}

fn all_blk_ids(f: &Fn) -> Vec<BlkId> {
    if f.rpo.is_empty() {
        (0..f.blks.len()).map(|i| BlkId(i as u32)).collect()
    } else {
        f.rpo.clone()
    }
}

fn classify(a: &mut AClass, t: &Typ, s0: u32, typs: &[Typ]) {
    for u in 0..t.nunion as usize {
        if u >= t.fields.len() {
            break;
        }
        let mut s = s0;
        for f in &t.fields[u] {
            assert!(s <= 16);
            let idx = (s / 8) as usize;
            match f.typ {
                FieldType::End => break,
                FieldType::FPad => {
                    s += f.len;
                }
                FieldType::Fs | FieldType::Fd => {
                    if a.cls[idx] == Cls::Kx {
                        a.cls[idx] = Cls::Kd;
                    }
                    s += f.len;
                }
                FieldType::Fb | FieldType::Fh | FieldType::Fw | FieldType::Fl => {
                    a.cls[idx] = Cls::Kl;
                    s += f.len;
                }
                FieldType::FTyp => {
                    let ti = f.len as usize;
                    classify(a, &typs[ti], s, typs);
                    s += typs[ti].size as u32;
                }
            }
        }
    }
}

fn typclass(a: &mut AClass, t: &Typ, typs: &[Typ]) {
    let mut sz = t.size;
    let mut al = 1u64 << t.align;
    if al < 8 {
        al = 8;
    }
    sz = (sz + al - 1) & !(al - 1);
    a.size = sz as u32;
    a.align = t.align;
    a.type_idx = None;

    if t.is_dark || sz > 16 || sz == 0 {
        a.inmem = 1;
        return;
    }

    a.cls = [Cls::Kx; 2];
    a.inmem = 0;
    classify(a, t, 0, typs);
}

fn retr(reg: &mut [Ref; 2], aret: &AClass) -> u32 {
    let retreg = [RET_GPRS, RET_FPRS];
    let mut nr = [0usize; 2];
    let mut ca = 0u32;
    let mut n = 0usize;
    while (n as u32) * 8 < aret.size {
        let k = aret.cls[n].base() as usize;
        reg[n] = preg(retreg[k][nr[k]]);
        nr[k] += 1;
        ca += 1 << (2 * k);
        n += 1;
    }
    ca
}

fn argsclass(
    ins: &[Ins],
    ac: &mut [AClass],
    is_par: bool,
    aret: Option<&AClass>,
    env: &mut Ref,
    typs: &[Typ],
) -> u32 {
    let mut nint = if matches!(aret, Some(a) if a.inmem != 0) {
        5
    } else {
        6
    };
    let mut nsse = 8;
    let mut varc = false;
    let mut envc = false;

    for (i, a) in ins.iter().zip(ac.iter_mut()) {
        match i.op {
            Op::Arg | Op::Par => {
                let pn = if i.cls.base() == 0 {
                    &mut nint
                } else {
                    &mut nsse
                };
                if *pn > 0 {
                    *pn -= 1;
                    a.inmem = 0;
                } else {
                    a.inmem = 2;
                }
                a.align = 3;
                a.size = 8;
                a.cls[0] = i.cls;
            }
            Op::Argc | Op::Parc => {
                let n = match i.arg[0] {
                    Ref::Typ(id) => id.0 as usize,
                    _ => panic!("aggregate arg/par missing type"),
                };
                typclass(a, &typs[n], typs);
                a.type_idx = Some(n);
                if a.inmem != 0 {
                    continue;
                }
                let mut ni = 0;
                let mut ns = 0;
                let mut m = 0usize;
                while (m as u32) * 8 < a.size {
                    if a.cls[m].base() == 0 {
                        ni += 1;
                    } else {
                        ns += 1;
                    }
                    m += 1;
                }
                if nint >= ni && nsse >= ns {
                    nint -= ni;
                    nsse -= ns;
                } else {
                    a.inmem = 1;
                }
            }
            Op::Arge | Op::Pare => {
                envc = true;
                *env = if is_par { i.to } else { i.arg[0] };
            }
            Op::Argv => {
                varc = true;
            }
            _ => {}
        }
    }

    assert!(
        !(varc && envc),
        "sysv abi does not support variadic env calls"
    );
    (((varc as u32) | (envc as u32)) << 12)
        | (((6 - nint) as u32) << 4)
        | (((8 - nsse) as u32) << 8)
}

#[inline]
fn rarg(ty: Cls, ni: &mut usize, ns: &mut usize) -> Ref {
    if ty.base() == 0 {
        let r = preg(ARG_GPRS[*ni]);
        *ni += 1;
        r
    } else {
        let r = preg(XMM0 + *ns as u32);
        *ns += 1;
        r
    }
}

fn selret(bid: BlkId, f: &mut Fn, buf: &mut InsBuffer, typs: &[Typ]) {
    let j = f.blks[bid.0 as usize].jmp.typ;
    if !j.is_ret() || j == Jmp::Ret0 {
        return;
    }

    let r0 = f.blks[bid.0 as usize].jmp.arg;
    f.blks[bid.0 as usize].jmp.typ = Jmp::Ret0;

    let ca = if j == Jmp::Retc {
        let mut aret = AClass::default();
        typclass(&mut aret, &typs[f.retty as usize], typs);
        if aret.inmem != 0 {
            buf.emit(Op::Copy, Cls::Kl, preg(RAX), f.retr, Ref::R);
            buf.emit(
                Op::Blit1,
                Cls::Kw,
                Ref::R,
                Ref::Int(typs[f.retty as usize].size as i32),
                Ref::R,
            );
            buf.emit(Op::Blit0, Cls::Kw, Ref::R, r0, f.retr);
            1
        } else {
            let mut reg = [Ref::R; 2];
            let ca = retr(&mut reg, &aret);
            if aret.size > 8 {
                let r = util::newtmp("abi", Cls::Kl, f);
                buf.emit(Op::Load, aret.cls[1], reg[1], r, Ref::R);
                buf.emit(Op::Add, Cls::Kl, r, r0, util::getcon(8, f));
            }
            buf.emit(Op::Load, aret.cls[0], reg[0], r0, Ref::R);
            ca
        }
    } else {
        let k = Cls::from_i8((j as u16 - Jmp::Retw as u16) as i8);
        if k.base() == 0 {
            buf.emit(Op::Copy, k, preg(RAX), r0, Ref::R);
            1
        } else {
            buf.emit(Op::Copy, k, preg(XMM0), r0, Ref::R);
            1 << 2
        }
    };

    f.blks[bid.0 as usize].jmp.arg = Ref::Call(ca);
}

fn selcall(f: &mut Fn, call_seq: &[Ins], buf: &mut InsBuffer, ral: &mut Vec<Ins>, typs: &[Typ]) {
    let nargs = call_seq.len() - 1;
    let i1 = call_seq[nargs];
    let args = &call_seq[..nargs];
    let mut ac = vec![AClass::default(); nargs];
    let mut env = Ref::R;
    let mut aret = AClass::default();
    let mut has_aret = false;
    let mut ca = if !i1.arg[1].is_none() {
        has_aret = true;
        let ti = match i1.arg[1] {
            Ref::Typ(id) => id.0 as usize,
            _ => panic!("aggregate call missing type"),
        };
        typclass(&mut aret, &typs[ti], typs);
        aret.type_idx = Some(ti);
        argsclass(args, &mut ac, false, Some(&aret), &mut env, typs)
    } else {
        argsclass(args, &mut ac, false, None, &mut env, typs)
    };

    let mut stk = 0u32;
    for a in ac.iter().rev() {
        if a.inmem != 0 {
            assert!(a.align <= 4, "sysv abi requires alignments of 16 or less");
            stk += a.size;
            if a.align == 4 {
                stk += stk & 15;
            }
        }
    }
    stk += stk & 15;
    if stk != 0 {
        buf.emit(
            Op::Salloc,
            Cls::Kl,
            Ref::R,
            util::getcon(-(stk as i64), f),
            Ref::R,
        );
    }

    let mut hidden_ret = Ref::R;
    if has_aret {
        if aret.inmem != 0 {
            hidden_ret = util::newtmp("abi", Cls::Kl, f);
            buf.emit(Op::Copy, Cls::Kl, i1.to, preg(RAX), Ref::R);
            ca += 1;
        } else {
            let mut reg = [Ref::R; 2];
            if aret.size > 8 {
                let r = util::newtmp("abi", Cls::Kl, f);
                aret.refs[1] = util::newtmp("abi", aret.cls[1], f);
                buf.emit(Op::Storel, Cls::Kw, Ref::R, aret.refs[1], r);
                buf.emit(Op::Add, Cls::Kl, r, i1.to, util::getcon(8, f));
            }
            aret.refs[0] = util::newtmp("abi", aret.cls[0], f);
            buf.emit(Op::Storel, Cls::Kw, Ref::R, aret.refs[0], i1.to);
            ca += retr(&mut reg, &aret);
            if aret.size > 8 {
                buf.emit(Op::Copy, aret.cls[1], aret.refs[1], reg[1], Ref::R);
            }
            buf.emit(Op::Copy, aret.cls[0], aret.refs[0], reg[0], Ref::R);
            hidden_ret = i1.to;
        }

        let al = if aret.align >= 2 { aret.align - 2 } else { 0 };
        let op = match al {
            0 => Op::Alloc4,
            1 => Op::Alloc8,
            _ => Op::Alloc16,
        };
        ral.push(Ins {
            op,
            cls: Cls::Kl,
            to: hidden_ret,
            arg: [util::getcon(aret.size as i64, f), Ref::R],
        });
    } else if i1.cls.base() == 0 {
        buf.emit(Op::Copy, i1.cls, i1.to, preg(RAX), Ref::R);
        ca += 1;
    } else {
        buf.emit(Op::Copy, i1.cls, i1.to, preg(XMM0), Ref::R);
        ca += 1 << 2;
    }

    buf.emit(Op::Call, i1.cls, Ref::R, i1.arg[0], Ref::Call(ca));

    if !env.is_none() {
        buf.emit(Op::Copy, Cls::Kl, preg(RAX), env, Ref::R);
    } else if ((ca >> 12) & 1) != 0 {
        buf.emit(
            Op::Copy,
            Cls::Kw,
            preg(RAX),
            util::getcon(((ca >> 8) & 15) as i64, f),
            Ref::R,
        );
    }

    let mut ni = 0usize;
    let mut ns = 0usize;
    if has_aret && aret.inmem != 0 {
        buf.emit(
            Op::Copy,
            Cls::Kl,
            rarg(Cls::Kl, &mut ni, &mut ns),
            hidden_ret,
            Ref::R,
        );
    }

    for (i, a) in args.iter().zip(ac.iter()) {
        if matches!(i.op, Op::Arge | Op::Argv) || a.inmem != 0 {
            continue;
        }
        let r1 = rarg(a.cls[0], &mut ni, &mut ns);
        if i.op == Op::Argc {
            if a.size > 8 {
                let r2 = rarg(a.cls[1], &mut ni, &mut ns);
                let r = util::newtmp("abi", Cls::Kl, f);
                buf.emit(Op::Load, a.cls[1], r2, r, Ref::R);
                buf.emit(Op::Add, Cls::Kl, r, i.arg[1], util::getcon(8, f));
            }
            buf.emit(Op::Load, a.cls[0], r1, i.arg[1], Ref::R);
        } else {
            buf.emit(Op::Copy, i.cls, r1, i.arg[0], Ref::R);
        }
    }

    if stk == 0 {
        return;
    }

    let r = util::newtmp("abi", Cls::Kl, f);
    let mut off = 0u32;
    for (i, a) in args.iter().zip(ac.iter()) {
        if matches!(i.op, Op::Arge | Op::Argv) || a.inmem == 0 {
            continue;
        }
        let r1 = util::newtmp("abi", Cls::Kl, f);
        if i.op == Op::Argc {
            if a.align == 4 {
                off += off & 15;
            }
            let ti = a.type_idx.expect("aggregate stack arg missing type");
            buf.emit(
                Op::Blit1,
                Cls::Kw,
                Ref::R,
                Ref::Int(typs[ti].size as i32),
                Ref::R,
            );
            buf.emit(Op::Blit0, Cls::Kw, Ref::R, i.arg[1], r1);
        } else {
            buf.emit(Op::Storel, Cls::Kw, Ref::R, i.arg[0], r1);
        }
        buf.emit(Op::Add, Cls::Kl, r1, r, util::getcon(off as i64, f));
        off += a.size;
    }
    buf.emit(Op::Salloc, Cls::Kl, r, util::getcon(stk as i64, f), Ref::R);
}

fn selpar(f: &mut Fn, ins: &[Ins], typs: &[Typ]) -> (Vec<Ins>, u32) {
    let mut ac = vec![AClass::default(); ins.len()];
    let mut aret = AClass::default();
    let mut env = Ref::R;
    let fa = if f.retty >= 0 {
        typclass(&mut aret, &typs[f.retty as usize], typs);
        argsclass(ins, &mut ac, true, Some(&aret), &mut env, typs)
    } else {
        argsclass(ins, &mut ac, true, None, &mut env, typs)
    };
    f.reg = argregs(Ref::Call(fa), None);

    let mut buf = InsBuffer::new();
    let mut ni = 0usize;
    let mut ns = 0usize;

    for (i, a) in ins.iter().zip(ac.iter_mut()) {
        if i.op != Op::Parc || a.inmem != 0 {
            continue;
        }
        if a.size > 8 {
            let r = util::newtmp("abi", Cls::Kl, f);
            a.refs[1] = util::newtmp("abi", Cls::Kl, f);
            buf.emit(Op::Storel, Cls::Kw, Ref::R, a.refs[1], r);
            buf.emit(Op::Add, Cls::Kl, r, i.to, util::getcon(8, f));
        }
        a.refs[0] = util::newtmp("abi", Cls::Kl, f);
        buf.emit(Op::Storel, Cls::Kw, Ref::R, a.refs[0], i.to);
        let al = if a.align >= 2 { a.align - 2 } else { 0 };
        let op = match al {
            0 => Op::Alloc4,
            1 => Op::Alloc8,
            _ => Op::Alloc16,
        };
        buf.emit(op, Cls::Kl, i.to, util::getcon(a.size as i64, f), Ref::R);
    }

    if f.retty >= 0 && aret.inmem != 0 {
        let r = util::newtmp("abi", Cls::Kl, f);
        buf.emit(
            Op::Copy,
            Cls::Kl,
            r,
            rarg(Cls::Kl, &mut ni, &mut ns),
            Ref::R,
        );
        f.retr = r;
    }

    let mut s = 4i32;
    for (i, a) in ins.iter().zip(ac.iter()) {
        match a.inmem {
            1 => {
                assert!(a.align <= 4, "sysv abi requires alignments of 16 or less");
                if a.align == 4 {
                    s = (s + 3) & !3;
                }
                if let Ref::Tmp(tid) = i.to {
                    f.tmps[tid.0 as usize].slot = -s;
                }
                s += (a.size / 4) as i32;
                continue;
            }
            2 => {
                buf.emit(Op::Load, i.cls, i.to, slot_ref(-s), Ref::R);
                s += 2;
                continue;
            }
            _ => {}
        }
        if i.op == Op::Pare {
            continue;
        }
        let r = rarg(a.cls[0], &mut ni, &mut ns);
        if i.op == Op::Parc {
            buf.emit(Op::Copy, a.cls[0], a.refs[0], r, Ref::R);
            if a.size > 8 {
                let r2 = rarg(a.cls[1], &mut ni, &mut ns);
                buf.emit(Op::Copy, a.cls[1], a.refs[1], r2, Ref::R);
            }
        } else {
            buf.emit(Op::Copy, i.cls, i.to, r, Ref::R);
        }
    }

    if !env.is_none() {
        buf.emit(Op::Copy, Cls::Kl, env, preg(RAX), Ref::R);
    }

    (buf.finish(), fa | (((s * 4) as u32) << 12))
}

fn expand_vastart_seq(f: &mut Fn, fa: u32, ap: Ref) -> Vec<Ins> {
    let gp = (((fa >> 4) & 15) * 8) as i64;
    let fp = (48 + ((fa >> 8) & 15) * 16) as i64;
    let sp = (fa >> 12) as i64;
    let mut buf = InsBuffer::new();

    let r0 = util::newtmp("abi", Cls::Kl, f);
    let r1 = util::newtmp("abi", Cls::Kl, f);
    buf.emit(Op::Storel, Cls::Kw, Ref::R, r1, r0);
    buf.emit(Op::Add, Cls::Kl, r1, preg(RBP), util::getcon(-176, f));
    buf.emit(Op::Add, Cls::Kl, r0, ap, util::getcon(16, f));

    let r0 = util::newtmp("abi", Cls::Kl, f);
    let r1 = util::newtmp("abi", Cls::Kl, f);
    buf.emit(Op::Storel, Cls::Kw, Ref::R, r1, r0);
    buf.emit(Op::Add, Cls::Kl, r1, preg(RBP), util::getcon(sp, f));
    buf.emit(Op::Add, Cls::Kl, r0, ap, util::getcon(8, f));

    let r0 = util::newtmp("abi", Cls::Kl, f);
    buf.emit(Op::Storew, Cls::Kw, Ref::R, util::getcon(fp, f), r0);
    buf.emit(Op::Add, Cls::Kl, r0, ap, util::getcon(4, f));
    buf.emit(Op::Storew, Cls::Kw, Ref::R, util::getcon(gp, f), ap);

    buf.finish()
}

fn patch_succ_phis(f: &mut Fn, succ: Option<BlkId>, from: BlkId, to: BlkId) {
    let Some(succ) = succ else { return };
    for phi in &mut f.blks[succ.0 as usize].phi {
        for blk in &mut phi.blks {
            if *blk == from {
                *blk = to;
            }
        }
    }
}

fn lower_vaarg_block(f: &mut Fn, rpo_idx: usize, bid: BlkId, ins_idx: usize) {
    let old_blk = f.blks[bid.0 as usize].clone();
    let i = old_blk.ins[ins_idx];
    let ap = i.arg[0];
    let isint = i.cls.base() == 0;

    let loc = util::newtmp("abi", Cls::Kl, f);
    let lreg = util::newtmp("abi", Cls::Kl, f);
    let lstk = util::newtmp("abi", Cls::Kl, f);
    let nr = util::newtmp("abi", Cls::Kl, f);
    let c4 = util::getcon(4, f);
    let c8 = util::getcon(8, f);
    let c16 = util::getcon(16, f);
    let limit = util::getcon(if isint { 48 } else { 176 }, f);

    let bstk_id = BlkId(f.blks.len() as u32);
    let breg_id = BlkId((f.blks.len() + 1) as u32);
    let b0_id = BlkId((f.blks.len() + 2) as u32);

    let mut b = old_blk.clone();
    b.ins = old_blk.ins[..ins_idx].to_vec();
    let r0 = util::newtmp("abi", Cls::Kl, f);
    let r1 = util::newtmp("abi", Cls::Kw, f);
    b.ins.push(Ins {
        op: Op::Add,
        cls: Cls::Kl,
        to: r0,
        arg: [ap, if isint { Ref::CON_Z } else { c4 }],
    });
    b.ins.push(Ins {
        op: Op::Loadsw,
        cls: Cls::Kl,
        to: nr,
        arg: [r0, Ref::R],
    });
    b.ins.push(Ins {
        op: Op::Cultw,
        cls: Cls::Kw,
        to: r1,
        arg: [nr, limit],
    });
    b.jmp.typ = Jmp::Jnz;
    b.jmp.arg = r1;
    b.s1 = Some(breg_id);
    b.s2 = Some(bstk_id);

    let mut breg = Blk {
        name: format!("{}.va.reg{}", old_blk.name, breg_id.0),
        loop_depth: old_blk.loop_depth,
        ..Blk::default()
    };
    let r0a = util::newtmp("abi", Cls::Kl, f);
    let r1a = util::newtmp("abi", Cls::Kl, f);
    let r0b = util::newtmp("abi", Cls::Kw, f);
    let r1b = util::newtmp("abi", Cls::Kl, f);
    breg.ins.push(Ins {
        op: Op::Add,
        cls: Cls::Kl,
        to: r0a,
        arg: [ap, c16],
    });
    breg.ins.push(Ins {
        op: Op::Load,
        cls: Cls::Kl,
        to: r1a,
        arg: [r0a, Ref::R],
    });
    breg.ins.push(Ins {
        op: Op::Add,
        cls: Cls::Kl,
        to: lreg,
        arg: [r1a, nr],
    });
    breg.ins.push(Ins {
        op: Op::Add,
        cls: Cls::Kw,
        to: r0b,
        arg: [nr, if isint { c8 } else { c16 }],
    });
    breg.ins.push(Ins {
        op: Op::Add,
        cls: Cls::Kl,
        to: r1b,
        arg: [ap, if isint { Ref::CON_Z } else { c4 }],
    });
    breg.ins.push(Ins {
        op: Op::Storew,
        cls: Cls::Kw,
        to: Ref::R,
        arg: [r0b, r1b],
    });
    breg.jmp.typ = Jmp::Jmp_;
    breg.s1 = Some(b0_id);

    let mut bstk = Blk {
        name: format!("{}.va.stk{}", old_blk.name, bstk_id.0),
        loop_depth: old_blk.loop_depth,
        ..Blk::default()
    };
    let r0c = util::newtmp("abi", Cls::Kl, f);
    let r1c = util::newtmp("abi", Cls::Kl, f);
    bstk.ins.push(Ins {
        op: Op::Add,
        cls: Cls::Kl,
        to: r0c,
        arg: [ap, c8],
    });
    bstk.ins.push(Ins {
        op: Op::Load,
        cls: Cls::Kl,
        to: lstk,
        arg: [r0c, Ref::R],
    });
    bstk.ins.push(Ins {
        op: Op::Add,
        cls: Cls::Kl,
        to: r1c,
        arg: [lstk, c8],
    });
    bstk.ins.push(Ins {
        op: Op::Storel,
        cls: Cls::Kw,
        to: Ref::R,
        arg: [r1c, r0c],
    });
    bstk.jmp.typ = Jmp::Jmp_;
    bstk.s1 = Some(b0_id);

    let mut b0 = Blk {
        name: format!("{}.va.tail{}", old_blk.name, b0_id.0),
        loop_depth: old_blk.loop_depth,
        ..Blk::default()
    };
    b0.phi.push(Phi {
        to: loc,
        cls: Cls::Kl,
        args: vec![lstk, lreg],
        blks: vec![bstk_id, breg_id],
    });
    b0.ins.push(Ins {
        op: Op::Load,
        cls: i.cls,
        to: i.to,
        arg: [loc, Ref::R],
    });
    b0.ins.extend_from_slice(&old_blk.ins[ins_idx + 1..]);
    b0.jmp = old_blk.jmp;
    b0.s1 = old_blk.s1;
    b0.s2 = old_blk.s2;

    f.blks[bid.0 as usize] = b;
    patch_succ_phis(f, old_blk.s1, bid, b0_id);
    if old_blk.s2 != old_blk.s1 {
        patch_succ_phis(f, old_blk.s2, bid, b0_id);
    }
    f.blks.push(bstk);
    f.blks.push(breg);
    f.blks.push(b0);
    f.rpo.insert(rpo_idx + 1, bstk_id);
    f.rpo.insert(rpo_idx + 2, breg_id);
    f.rpo.insert(rpo_idx + 3, b0_id);
}

pub fn abi0(f: &mut Fn, _t: &Target) {
    for bid in all_blk_ids(f) {
        for ins in &mut f.blks[bid.0 as usize].ins {
            if ins.op.is_argbh() {
                ins.op = Op::Arg;
            }
            if ins.op.is_parbh() {
                ins.op = Op::Par;
            }
        }
        if f.blks[bid.0 as usize].jmp.typ.is_retbh() {
            f.blks[bid.0 as usize].jmp.typ = Jmp::Retw;
        }
    }
}

pub fn retregs(r: Ref, p: Option<&mut [i32; 2]>) -> u64 {
    assert!(matches!(r, Ref::Call(_)));
    let mut b = 0u64;
    let ni = (r.val() & 3) as i32;
    let nf = ((r.val() >> 2) & 3) as i32;
    if ni >= 1 {
        b |= bit(RAX);
    }
    if ni >= 2 {
        b |= bit(RDX);
    }
    if nf >= 1 {
        b |= bit(XMM0);
    }
    if nf >= 2 {
        b |= bit(XMM1);
    }
    if let Some(p) = p {
        p[0] = ni;
        p[1] = nf;
    }
    b
}

pub fn argregs(r: Ref, p: Option<&mut [i32; 2]>) -> u64 {
    assert!(matches!(r, Ref::Call(_)));
    let mut b = 0u64;
    let ni = ((r.val() >> 4) & 15) as usize;
    let nf = ((r.val() >> 8) & 15) as usize;
    let ra = ((r.val() >> 12) & 1) != 0;
    for &r in AMD64_SYSV_RSAVE.iter().take(ni) {
        b |= bit(r as u32);
    }
    for j in 0..nf {
        b |= bit(XMM0 + j as u32);
    }
    if let Some(p) = p {
        p[0] = ni as i32 + if ra { 1 } else { 0 };
        p[1] = nf as i32;
    }
    if ra {
        b |= bit(RAX);
    }
    b
}

pub fn abi1(f: &mut Fn, _t: &Target, typs: &[Typ]) {
    let start = f.start;
    let par_end = {
        let blk = &f.blks[start.0 as usize];
        let mut end = 0usize;
        while end < blk.ins.len() && blk.ins[end].op.is_par() {
            end += 1;
        }
        end
    };
    let par_ins = f.blks[start.0 as usize].ins[..par_end].to_vec();
    let rest_ins = f.blks[start.0 as usize].ins[par_end..].to_vec();
    let (mut lowered_par, fa) = selpar(f, &par_ins, typs);
    lowered_par.extend_from_slice(&rest_ins);
    f.blks[start.0 as usize].ins = lowered_par;

    let mut idx = 0usize;
    while idx < f.rpo.len() {
        let bid = f.rpo[idx];
        let mut lowered = Vec::new();
        let mut changed = false;
        let old_ins = f.blks[bid.0 as usize].ins.clone();
        for ins in old_ins {
            if ins.op == Op::Vastart {
                lowered.extend(expand_vastart_seq(f, fa, ins.arg[0]));
                changed = true;
            } else {
                lowered.push(ins);
            }
        }
        if changed {
            f.blks[bid.0 as usize].ins = lowered;
        }
        if let Some(pos) = f.blks[bid.0 as usize]
            .ins
            .iter()
            .position(|i| i.op == Op::Vaarg)
        {
            lower_vaarg_block(f, idx, bid, pos);
        }
        idx += 1;
    }

    let mut ral = Vec::new();
    let mut block_order: Vec<BlkId> = f.rpo.iter().copied().filter(|&b| b != start).collect();
    block_order.push(start);

    for bid in block_order {
        let mut buf = InsBuffer::new();
        selret(bid, f, &mut buf, typs);
        let ins = std::mem::take(&mut f.blks[bid.0 as usize].ins);
        let mut i = ins.len();
        while i > 0 {
            i -= 1;
            match ins[i].op {
                Op::Call => {
                    let mut i0 = i;
                    while i0 > 0 && ins[i0 - 1].op.is_arg() {
                        i0 -= 1;
                    }
                    selcall(f, &ins[i0..=i], &mut buf, &mut ral, typs);
                    i = i0;
                }
                Op::Arg | Op::Argc => {
                    panic!("unreachable: bare arg/argc outside call");
                }
                Op::Arge | Op::Argv | Op::Vastart | Op::Vaarg => {
                    panic!("unreachable: abi op remained after amd64 abi lowering");
                }
                _ => buf.emiti(ins[i]),
            }
        }
        if bid == start {
            for ins in ral.iter().rev() {
                buf.emiti(*ins);
            }
        }
        f.blks[bid.0 as usize].ins = buf.finish();
    }
}
