//! Constant folding and unreachable-block elimination.

use crate::analysis::control_flow as cfg;
use crate::ir::builder::*;
use crate::ir::internal::*;

/// Lattice value for a temporary.
///   - `Top`: not yet determined (matches UNDEF)
///   - `Bot`: known to be non-constant
///   - `Con(u32)`: known constant, value is a `ConId` index
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum LatVal {
    Top,
    Bot,
    Con(u32),
}

#[derive(Clone, Debug)]
struct Edge {
    dest: i32, // RPO index of destination block, or -1
    dead: bool,
}

fn iscon(c: &Con, w: bool, k: u64) -> bool {
    if c.typ != ConType::Bits {
        return false;
    }
    if w {
        c.bits.i() as u64 == k
    } else {
        (c.bits.i() as u32) == (k as u32)
    }
}

fn latval(r: Ref, val: &[LatVal]) -> LatVal {
    match r {
        Ref::Tmp(t) => val[t.0 as usize],
        Ref::Con(c) => LatVal::Con(c.0),
        _ => panic!("latval: unexpected ref {:?}", r),
    }
}

fn latmerge(v: LatVal, m: LatVal) -> LatVal {
    if m == LatVal::Top {
        v
    } else if v == LatVal::Top || v == m {
        m
    } else {
        LatVal::Bot
    }
}

fn update(t: u32, m: LatVal, val: &mut [LatVal], usewrk: &mut Vec<Use>, f: &Fn) {
    let m = latmerge(val[t as usize], m);
    if m != val[t as usize] {
        let tmp = &f.tmps[t as usize];
        for u in &tmp.uses {
            usewrk.push(*u);
        }
        val[t as usize] = m;
    }
}

fn deadedge(s: usize, d: usize, edge: &[[Edge; 2]]) -> bool {
    let e = &edge[s];
    if e[0].dest == d as i32 && !e[0].dead {
        return false;
    }
    if e[1].dest == d as i32 && !e[1].dead {
        return false;
    }
    true
}

fn visitphi(
    p: &Phi,
    n: usize,
    val: &mut [LatVal],
    usewrk: &mut Vec<Use>,
    edge: &[[Edge; 2]],
    f: &Fn,
) {
    let mut v = LatVal::Top;
    for a in 0..p.narg() {
        let pred_rpo = f.blks[p.blks[a].0 as usize].id as usize;
        if !deadedge(pred_rpo, n, edge) {
            v = latmerge(v, latval(p.args[a], val));
        }
    }
    if let Ref::Tmp(t) = p.to {
        update(t.0, v, val, usewrk, f);
    }
}

fn visitins(i: &Ins, val: &mut [LatVal], usewrk: &mut Vec<Use>, f: &mut Fn) {
    if !i.to.is_tmp() {
        return;
    }
    let v;
    if i.op.can_fold() {
        let l = latval(i.arg[0], val);
        let r = if i.arg[1].is_none() {
            LatVal::Con(1)
        } else {
            latval(i.arg[1], val)
        };
        if l == LatVal::Bot || r == LatVal::Bot {
            v = LatVal::Bot;
        } else if l == LatVal::Top || r == LatVal::Top {
            v = LatVal::Top;
        } else {
            let lc = match l {
                LatVal::Con(c) => c,
                _ => unreachable!(),
            };
            let rc = match r {
                LatVal::Con(c) => c,
                _ => unreachable!(),
            };
            v = opfold(i.op, i.cls, lc, rc, f);
        }
    } else {
        v = LatVal::Bot;
    }
    if let Ref::Tmp(t) = i.to {
        update(t.0, v, val, usewrk, f);
    }
}

fn visitjmp(
    b: &Blk,
    n: usize,
    val: &[LatVal],
    edge: &mut [[Edge; 2]],
    flowrk: &mut Vec<usize>,
    f: &Fn,
) {
    if b.jmp.typ.is_jnz() {
        let l = latval(b.jmp.arg, val);
        if l == LatVal::Bot {
            flowrk.push(n * 2 + 1);
            flowrk.push(n * 2);
        } else if let LatVal::Con(c) = l {
            if iscon(&f.cons[c as usize], false, 0) {
                debug_assert!(edge[n][0].dead);
                flowrk.push(n * 2 + 1);
            } else {
                debug_assert!(edge[n][1].dead);
                flowrk.push(n * 2);
            }
        }
    } else if b.jmp.typ.is_jmp() {
        flowrk.push(n * 2);
    } else if b.jmp.typ == Jmp::Hlt || b.jmp.typ.is_ret() {
    } else {
        panic!("visitjmp: unexpected jmp type {:?}", b.jmp.typ);
    }
}

fn initedge(s: Option<BlkId>, f: &Fn) -> Edge {
    Edge {
        dest: match s {
            Some(id) => f.blks[id.0 as usize].id as i32,
            None => -1,
        },
        dead: true,
    }
}

fn renref(r: &mut Ref, val: &[LatVal]) -> bool {
    if let Ref::Tmp(t) = *r
        && let LatVal::Con(c) = val[t.0 as usize]
    {
        *r = Ref::Con(ConId(c));
        return true;
    }
    false
}

/// Sparse Conditional Constant Propagation pass.
/// Requires: RPO, use info, pred info.
pub fn fold(f: &mut Fn) {
    let ntmp = f.tmps.len();
    let nblk = f.rpo.len();

    let mut val: Vec<LatVal> = vec![LatVal::Top; ntmp];

    let mut edge: Vec<[Edge; 2]> = Vec::with_capacity(nblk);
    for n in 0..nblk {
        let bid = f.rpo[n];
        let s1 = f.blks[bid.0 as usize].s1;
        let s2 = f.blks[bid.0 as usize].s2;
        f.blks[bid.0 as usize].visit = 0;
        edge.push([initedge(s1, f), initedge(s2, f)]);
    }

    let mut flowrk: Vec<usize> = Vec::new();
    let start_edge_idx = nblk * 2; // sentinel index for the start edge
    let start_edge = Edge {
        dest: 0, // RPO index 0 = start block
        dead: true,
    };
    edge.push([
        start_edge.clone(),
        Edge {
            dest: -1,
            dead: true,
        },
    ]);
    flowrk.push(start_edge_idx); // index into edge array, slot 0

    let mut usewrk: Vec<Use> = Vec::new();

    loop {
        if let Some(eidx) = flowrk.pop() {
            let slot = eidx & 1;
            let block_idx = eidx >> 1;
            let e = &edge[block_idx][slot];
            if e.dest == -1 || !e.dead {
                continue;
            }
            let dest = e.dest as usize;
            edge[block_idx][slot].dead = false;

            let bid = f.rpo[dest];
            let n = dest;

            let nphi = f.blks[bid.0 as usize].phi.len();
            for pi in 0..nphi {
                let p = f.blks[bid.0 as usize].phi[pi].clone();
                visitphi(&p, n, &mut val, &mut usewrk, &edge, f);
            }

            let visit = f.blks[bid.0 as usize].visit;
            if visit == 0 {
                let nins = f.blks[bid.0 as usize].ins.len();
                for ii in 0..nins {
                    let ins = f.blks[bid.0 as usize].ins[ii];
                    visitins(&ins, &mut val, &mut usewrk, f);
                }
                let b_copy = f.blks[bid.0 as usize].clone();
                visitjmp(&b_copy, n, &val, &mut edge, &mut flowrk, f);
            }
            f.blks[bid.0 as usize].visit += 1;
        } else if let Some(u) = usewrk.pop() {
            let bid = BlkId(u.bid);
            let n = f.blks[bid.0 as usize].id as usize;
            if n >= nblk {
                continue;
            }
            if f.blks[bid.0 as usize].visit == 0 {
                continue;
            }
            match u.typ {
                UseType::Phi => {
                    if let UseDetail::PhiIdx(pidx) = u.detail {
                        let p = f.blks[bid.0 as usize].phi[pidx as usize].clone();
                        visitphi(&p, n, &mut val, &mut usewrk, &edge, f);
                    }
                }
                UseType::Ins => {
                    if let UseDetail::InsIdx(iidx) = u.detail {
                        let ins = f.blks[bid.0 as usize].ins[iidx as usize];
                        visitins(&ins, &mut val, &mut usewrk, f);
                    }
                }
                UseType::Jmp => {
                    let b_copy = f.blks[bid.0 as usize].clone();
                    visitjmp(&b_copy, n, &val, &mut edge, &mut flowrk, f);
                }
                _ => panic!("fold: unexpected use type {:?}", u.typ),
            }
        } else {
            break;
        }
    }

    let mut dead_blks: Vec<bool> = vec![false; f.blks.len()];
    let mut any_dead = false;
    for n in 0..nblk {
        let bid = f.rpo[n];
        let b = &f.blks[bid.0 as usize];
        if b.visit == 0 {
            dead_blks[bid.0 as usize] = true;
            any_dead = true;
        }
    }

    if any_dead {
        for n in 0..nblk {
            let bid = f.rpo[n];
            if !dead_blks[bid.0 as usize] {
                continue;
            }
            let s1 = f.blks[bid.0 as usize].s1;
            let s2 = f.blks[bid.0 as usize].s2;
            if s1.is_some() {
                cfg::edgedel(f, bid, 0);
            }
            if s2.is_some() {
                cfg::edgedel(f, bid, 1);
            }
        }
    }

    for n in 0..nblk {
        let bid = f.rpo[n];
        if dead_blks[bid.0 as usize] {
            continue;
        }

        let bi = bid.0 as usize;

        let mut new_phis: Vec<Phi> = Vec::new();
        let phis = std::mem::take(&mut f.blks[bi].phi);
        for mut p in phis {
            if let Ref::Tmp(t) = p.to
                && val[t.0 as usize] != LatVal::Bot
            {
                continue;
            }
            for a in 0..p.narg() {
                let pred_rpo = f.blks[p.blks[a].0 as usize].id as usize;
                if !deadedge(pred_rpo, n, &edge) {
                    renref(&mut p.args[a], &val);
                }
            }
            new_phis.push(p);
        }
        f.blks[bi].phi = new_phis;

        let nins = f.blks[bi].ins.len();
        for ii in 0..nins {
            let mut ins = f.blks[bi].ins[ii];
            if renref(&mut ins.to, &val) {
                ins = Ins::default(); // Nop
            } else {
                renref(&mut ins.arg[0], &val);
                renref(&mut ins.arg[1], &val);
                if ins.op.is_store() && ins.arg[0] == Ref::UNDEF {
                    ins = Ins::default(); // Nop
                }
            }
            f.blks[bi].ins[ii] = ins;
        }

        let mut jmp = f.blks[bi].jmp;
        renref(&mut jmp.arg, &val);
        if jmp.typ.is_jnz()
            && let Ref::Con(c) = jmp.arg
        {
            if iscon(&f.cons[c.0 as usize], false, 0) {
                cfg::edgedel(f, bid, 0);
                let s2 = f.blks[bi].s2;
                f.blks[bi].s1 = s2;
                f.blks[bi].s2 = None;
            } else {
                cfg::edgedel(f, bid, 1);
            }
            jmp.typ = Jmp::Jmp_;
            jmp.arg = Ref::R;
        }
        f.blks[bi].jmp = jmp;
    }

    if any_dead {
        for n in 0..nblk {
            let bid = f.rpo[n];
            if dead_blks[bid.0 as usize] {
                f.blks[bid.0 as usize].jmp = JmpInfo::default();
                f.blks[bid.0 as usize].s1 = None;
                f.blks[bid.0 as usize].s2 = None;
                f.blks[bid.0 as usize].phi.clear();
                f.blks[bid.0 as usize].ins.clear();
            }
        }
        cfg::fillrpo(f);
    }
}

fn evaluate_integer_comparison(kind: i32, left: u64, right: u64) -> u64 {
    let left_signed = left as i64;
    let right_signed = right as i64;
    match kind {
        value if value == CmpI::Cieq as i32 => (left == right) as u64,
        value if value == CmpI::Cine as i32 => (left != right) as u64,
        value if value == CmpI::Cisge as i32 => (left_signed >= right_signed) as u64,
        value if value == CmpI::Cisgt as i32 => (left_signed > right_signed) as u64,
        value if value == CmpI::Cisle as i32 => (left_signed <= right_signed) as u64,
        value if value == CmpI::Cislt as i32 => (left_signed < right_signed) as u64,
        value if value == CmpI::Ciuge as i32 => (left >= right) as u64,
        value if value == CmpI::Ciugt as i32 => (left > right) as u64,
        value if value == CmpI::Ciule as i32 => (left <= right) as u64,
        value if value == CmpI::Ciult as i32 => (left < right) as u64,
        _ => unreachable!("comparison kind came from iscmp"),
    }
}

fn evaluate_float_comparison(kind: i32, left: f64, right: f64) -> u64 {
    match kind {
        value if value == CmpF::Cfeq as i32 => (left == right) as u64,
        value if value == CmpF::Cfge as i32 => (left >= right) as u64,
        value if value == CmpF::Cfgt as i32 => (left > right) as u64,
        value if value == CmpF::Cfle as i32 => (left <= right) as u64,
        value if value == CmpF::Cflt as i32 => (left < right) as u64,
        value if value == CmpF::Cfne as i32 => (left != right) as u64,
        value if value == CmpF::Cfo as i32 => (!left.is_nan() && !right.is_nan()) as u64,
        value if value == CmpF::Cfuo as i32 => (left.is_nan() || right.is_nan()) as u64,
        _ => unreachable!("comparison kind came from iscmp"),
    }
}

/// Fold an integer operation, or return `None` when folding is invalid.
fn foldint(op: Op, w: bool, cl: &Con, cr: &Con) -> Option<Con> {
    let mut sym = Sym::default();
    let mut typ = ConType::Bits;

    let l_u = cl.bits.i() as u64;
    let r_u = cr.bits.i() as u64;
    let l_s = cl.bits.i();
    let r_s = cr.bits.i();

    if op == Op::Add {
        if cl.typ == ConType::Addr {
            if cr.typ == ConType::Addr {
                return None;
            }
            typ = ConType::Addr;
            sym = cl.sym;
        } else if cr.typ == ConType::Addr {
            typ = ConType::Addr;
            sym = cr.sym;
        }
    } else if op == Op::Sub {
        if cl.typ == ConType::Addr {
            if cr.typ != ConType::Addr {
                typ = ConType::Addr;
                sym = cl.sym;
            } else if cl.sym != cr.sym {
                return None;
            }
        } else if cr.typ == ConType::Addr {
            return None;
        }
    } else if cl.typ == ConType::Addr || cr.typ == ConType::Addr {
        return None;
    }

    if op == Op::Div || op == Op::Rem || op == Op::Udiv || op == Op::Urem {
        if iscon(cr, w, 0) {
            return None;
        }
        if op == Op::Div || op == Op::Rem {
            let min = if w { i64::MIN as u64 } else { i32::MIN as u64 };
            if iscon(cr, w, u64::MAX) && iscon(cl, w, min) {
                return None;
            }
        }
    }

    let x: u64 = match op {
        Op::Add => l_u.wrapping_add(r_u),
        Op::Sub => l_u.wrapping_sub(r_u),
        Op::Neg => 0u64.wrapping_sub(l_u),
        Op::Div => {
            if w {
                (l_s / r_s) as u64
            } else {
                ((l_s as i32) / (r_s as i32)) as u64
            }
        }
        Op::Rem => {
            if w {
                (l_s % r_s) as u64
            } else {
                ((l_s as i32) % (r_s as i32)) as u64
            }
        }
        Op::Udiv => {
            if w {
                l_u / r_u
            } else {
                ((l_u as u32) / (r_u as u32)) as u64
            }
        }
        Op::Urem => {
            if w {
                l_u % r_u
            } else {
                ((l_u as u32) % (r_u as u32)) as u64
            }
        }
        Op::Mul => l_u.wrapping_mul(r_u),
        Op::And => l_u & r_u,
        Op::Or => l_u | r_u,
        Op::Xor => l_u ^ r_u,
        Op::Sar => {
            let shift = r_u & (31 | if w { 32 } else { 0 });
            if w {
                (l_s >> shift) as u64
            } else {
                ((l_s as i32) >> shift) as u64
            }
        }
        Op::Shr => {
            let shift = r_u & (31 | if w { 32 } else { 0 });
            if w {
                l_u >> shift
            } else {
                ((l_u as u32) >> shift) as u64
            }
        }
        Op::Shl => {
            let shift = r_u & (31 | if w { 32 } else { 0 });
            l_u.wrapping_shl(shift as u32)
        }
        Op::Extsb => (l_u as i8) as i64 as u64,
        Op::Extub => (l_u as u8) as u64,
        Op::Extsh => (l_u as i16) as i64 as u64,
        Op::Extuh => (l_u as u16) as u64,
        Op::Extsw => (l_u as i32) as i64 as u64,
        Op::Extuw => (l_u as u32) as u64,
        Op::Stosi => {
            if w {
                (cl.bits.s() as i64) as u64
            } else {
                (cl.bits.s() as i32) as u64
            }
        }
        Op::Stoui => {
            if w {
                cl.bits.s() as u64
            } else {
                cl.bits.s() as u32 as u64
            }
        }
        Op::Dtosi => {
            if w {
                (cl.bits.d() as i64) as u64
            } else {
                (cl.bits.d() as i32) as u64
            }
        }
        Op::Dtoui => {
            if w {
                cl.bits.d() as u64
            } else {
                cl.bits.d() as u32 as u64
            }
        }
        Op::Cast => {
            if cl.typ == ConType::Addr {
                typ = ConType::Addr;
                sym = cl.sym;
            }
            l_u
        }
        _ => {
            let (kind, class) =
                iscmp(op).unwrap_or_else(|| panic!("foldint: unreachable op {:?}", op));
            match Cls::from_i8(class as i8) {
                Cls::Kw => {
                    let left = (l_u as i32) as i64 as u64;
                    let right = (r_u as i32) as i64 as u64;
                    evaluate_integer_comparison(kind, left, right)
                }
                Cls::Kl => evaluate_integer_comparison(kind, l_u, r_u),
                Cls::Ks => evaluate_float_comparison(
                    kind - N_CMP_I as i32,
                    cl.bits.s() as f64,
                    cr.bits.s() as f64,
                ),
                Cls::Kd => {
                    evaluate_float_comparison(kind - N_CMP_I as i32, cl.bits.d(), cr.bits.d())
                }
                Cls::Kx => unreachable!("comparison class came from iscmp"),
            }
        }
    };

    Some(Con {
        typ,
        sym,
        bits: ConBits::from_i64(x as i64),
        flt: 0,
    })
}

fn foldflt(op: Op, w: bool, cl: &Con, cr: &Con) -> Con {
    if cl.typ != ConType::Bits || cr.typ != ConType::Bits {
        panic!(
            "invalid address operand for '{}'",
            OP_TABLE[op as usize].name
        );
    }
    let mut res = Con {
        typ: ConType::Bits,
        sym: Sym::default(),
        bits: ConBits::from_i64(0),
        flt: 0,
    };
    if w {
        let ld = cl.bits.d();
        let rd = cr.bits.d();
        let xd: f64 = match op {
            Op::Add => ld + rd,
            Op::Sub => ld - rd,
            Op::Neg => -ld,
            Op::Div => ld / rd,
            Op::Mul => ld * rd,
            Op::Swtof => (cl.bits.i() as i32) as f64,
            Op::Uwtof => (cl.bits.i() as u32) as f64,
            Op::Sltof => (cl.bits.i()) as f64,
            Op::Ultof => (cl.bits.i() as u64) as f64,
            Op::Exts => cl.bits.s() as f64,
            Op::Cast => ld,
            _ => panic!("foldflt: unreachable op {:?}", op),
        };
        res.bits = ConBits::from_f64(xd);
        res.flt = 2;
    } else {
        let ls = cl.bits.s();
        let rs = cr.bits.s();
        let xs: f32 = match op {
            Op::Add => ls + rs,
            Op::Sub => ls - rs,
            Op::Neg => -ls,
            Op::Div => ls / rs,
            Op::Mul => ls * rs,
            Op::Swtof => (cl.bits.i() as i32) as f32,
            Op::Uwtof => (cl.bits.i() as u32) as f32,
            Op::Sltof => (cl.bits.i()) as f32,
            Op::Ultof => (cl.bits.i() as u64) as f32,
            Op::Truncd => cl.bits.d() as f32,
            Op::Cast => ls,
            _ => panic!("foldflt: unreachable op {:?}", op),
        };
        res.bits = ConBits::from_f32(xs);
        res.flt = 1;
    }
    res
}

fn opfold(op: Op, cls: Cls, cl_id: u32, cr_id: u32, f: &mut Fn) -> LatVal {
    let cl = f.cons[cl_id as usize];
    let cr = f.cons[cr_id as usize];

    let mut c = if cls == Cls::Kw || cls == Cls::Kl {
        match foldint(op, cls == Cls::Kl, &cl, &cr) {
            Some(c) => c,
            None => return LatVal::Bot,
        }
    } else {
        foldflt(op, cls == Cls::Kd, &cl, &cr)
    };

    if !cls.is_wide() {
        let bits = c.bits.i() as u64 & 0xffffffff;
        c.bits = ConBits::from_i64(bits as i64);
    }

    let r = newcon(&c, f);
    debug_assert!(!(cls == Cls::Ks || cls == Cls::Kd) || c.flt != 0);
    match r {
        Ref::Con(id) => LatVal::Con(id.0),
        _ => unreachable!(),
    }
}
