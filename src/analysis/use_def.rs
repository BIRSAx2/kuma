//! SSA construction and validation.

use crate::ir::builder::{clsmerge, newtmp, phicls};
use crate::ir::internal::{
    BSet, BlkId, Cls, Fn, Op, Phi, Ref, TMP0, TmpId, Use, UseDetail, UseType, Width,
};

/// A scoped binding: temporary `r` was defined in block `bid`.
#[derive(Clone, Debug)]
struct NameEntry {
    r: Ref,
    bid: BlkId,
}

fn adduse(f: &mut Fn, t: u32, typ: UseType, bid: u32, detail: UseDetail) {
    let tmp = &mut f.tmps[t as usize];
    tmp.nuse += 1;
    tmp.uses.push(Use { typ, bid, detail });
}

/// Recompute use/def information for every temporary.
///
/// Iterates all blocks, phi nodes, and instructions to populate each Tmp's
/// def, bid, ndef, nuse, cls, phi, width, and uses fields.
pub fn filluse(f: &mut Fn) {
    for t in TMP0 as usize..f.tmps.len() {
        let tmp = &mut f.tmps[t];
        tmp.def = None;
        tmp.bid = u32::MAX;
        tmp.ndef = 0;
        tmp.nuse = 0;
        tmp.cls = Cls::Kx;
        tmp.phi = 0;
        tmp.width = Width::WFull;
        tmp.uses.clear();
    }

    for blk_idx in 0..f.blks.len() {
        let bid = blk_idx as u32; // storage index, not RPO number

        for phi_idx in 0..f.blks[blk_idx].phi.len() {
            let to = f.blks[blk_idx].phi[phi_idx].to;
            let pcls = f.blks[blk_idx].phi[phi_idx].cls;
            if let Ref::Tmp(tid) = to {
                let tp = tid.0 as usize;
                f.tmps[tp].bid = bid;
                f.tmps[tp].ndef += 1;
                f.tmps[tp].cls = pcls;

                let tp_cls = phicls(tp, &mut f.tmps);

                let narg = f.blks[blk_idx].phi[phi_idx].args.len();
                for a in 0..narg {
                    let arg = f.blks[blk_idx].phi[phi_idx].args[a];
                    if let Ref::Tmp(atid) = arg {
                        let t = atid.0;
                        adduse(f, t, UseType::Phi, bid, UseDetail::PhiIdx(phi_idx as u32));
                        let t_cls = phicls(t as usize, &mut f.tmps);
                        if t_cls != tp_cls {
                            f.tmps[t_cls].phi = tp_cls as i32;
                        }
                    }
                }
            }
        }

        for ins_idx in 0..f.blks[blk_idx].ins.len() {
            let ins = f.blks[blk_idx].ins[ins_idx];
            let ito = ins.to;
            if ito != Ref::R
                && let Ref::Tmp(tid) = ito
            {
                let t = tid.0 as usize;
                let op = ins.op;
                let icls = ins.cls;

                let mut w = width_for_operation(op).unwrap_or(Width::WFull);

                if (w == Width::Wsw || w == Width::Wuw) && icls == Cls::Kw {
                    w = Width::WFull;
                }

                f.tmps[t].width = w;
                f.tmps[t].def = Some(ins_idx);
                f.tmps[t].bid = bid;
                f.tmps[t].ndef += 1;
                f.tmps[t].cls = icls;
            }
            for m in 0..2 {
                if let Ref::Tmp(tid) = ins.arg[m] {
                    let t = tid.0;
                    adduse(f, t, UseType::Ins, bid, UseDetail::InsIdx(ins_idx as u32));
                }
            }
        }

        if let Ref::Tmp(tid) = f.blks[blk_idx].jmp.arg {
            adduse(f, tid.0, UseType::Jmp, bid, UseDetail::Jmp);
        }
    }
}

fn width_for_operation(operation: Op) -> Option<Width> {
    match operation {
        Op::Parsb | Op::Loadsb | Op::Extsb => Some(Width::Wsb),
        Op::Parub | Op::Loadub | Op::Extub => Some(Width::Wub),
        Op::Parsh | Op::Loadsh | Op::Extsh => Some(Width::Wsh),
        Op::Paruh | Op::Loaduh | Op::Extuh => Some(Width::Wuh),
        Op::Loadsw | Op::Extsw => Some(Width::Wsw),
        Op::Loaduw | Op::Extuw => Some(Width::Wuw),
        _ => None,
    }
}

fn refindex(t: usize, f: &mut Fn) -> Ref {
    let name = f.tmps[t].name.clone();
    let cls = f.tmps[t].cls;
    newtmp(&name, cls, f)
}

/// Insert phi nodes at iterated dominance frontiers using a worklist algorithm.
fn phiins(f: &mut Fn) -> Result<(), String> {
    let nblk = f.blks.len();
    let mut u = BSet::new(nblk as u32);
    let mut defs = BSet::new(nblk as u32);

    let mut blist: Vec<usize> = Vec::with_capacity(nblk);

    let nt = f.tmps.len();
    for t in TMP0 as usize..nt {
        f.tmps[t].visit = 0;
        if f.tmps[t].phi != 0 {
            continue;
        }

        if f.tmps[t].ndef == 1 {
            let defb = f.tmps[t].bid;
            let mut ok = true;
            for u_entry in &f.tmps[t].uses {
                if u_entry.bid != defb {
                    ok = false;
                    break;
                }
            }
            let start_idx = f.start.0;
            if ok || defb == start_idx {
                continue;
            }
        }

        u.zero();
        let mut k = Cls::Kx;
        blist.clear();

        for blk_idx in 0..f.blks.len() {
            f.blks[blk_idx].visit = 0;
            let mut r = Ref::R;

            let nins = f.blks[blk_idx].ins.len();
            for i in 0..nins {
                if r != Ref::R {
                    if f.blks[blk_idx].ins[i].arg[0] == Ref::Tmp(TmpId(t as u32)) {
                        f.blks[blk_idx].ins[i].arg[0] = r;
                    }
                    if f.blks[blk_idx].ins[i].arg[1] == Ref::Tmp(TmpId(t as u32)) {
                        f.blks[blk_idx].ins[i].arg[1] = r;
                    }
                }
                if f.blks[blk_idx].ins[i].to == Ref::Tmp(TmpId(t as u32)) {
                    if !f.blks[blk_idx].out.has(t as u32) {
                        r = refindex(t, f);
                        f.blks[blk_idx].ins[i].to = r;
                    } else {
                        let bid = f.blks[blk_idx].id;
                        if !u.has(bid) {
                            u.set(bid);
                            blist.push(blk_idx);
                        }
                        let icls = f.blks[blk_idx].ins[i].cls;
                        if clsmerge(&mut k, icls) {
                            return Err(format!(
                                "temporary %{} has conflicting classes",
                                f.tmps[t].name
                            ));
                        }
                    }
                }
            }
            if r != Ref::R && f.blks[blk_idx].jmp.arg == Ref::Tmp(TmpId(t as u32)) {
                f.blks[blk_idx].jmp.arg = r;
            }
        }

        defs.copy_from(&u);
        let mut wi = 0; // worklist read index
        while wi < blist.len() {
            f.tmps[t].visit = t as i32;
            let blk_idx = blist[wi];
            wi += 1;
            let bid = f.blks[blk_idx].id;
            u.clr(bid);

            let nfron = f.blks[blk_idx].fron.len();
            for fi in 0..nfron {
                let a_id = f.blks[blk_idx].fron[fi];
                let a_idx = a_id.0 as usize;

                if f.blks[a_idx].visit > 0 {
                    continue;
                }
                f.blks[a_idx].visit += 1;

                if !f.blks[a_idx].r#in.has(t as u32) {
                    continue;
                }

                let phi = Phi {
                    cls: k,
                    to: Ref::Tmp(TmpId(t as u32)),
                    args: Vec::new(),
                    blks: Vec::new(),
                };
                f.blks[a_idx].phi.push(phi);

                let a_rpoid = f.blks[a_idx].id;
                if !defs.has(a_rpoid) && !u.has(a_rpoid) {
                    u.set(a_rpoid);
                    blist.push(a_idx);
                }
            }
        }
    }
    Ok(())
}

/// Does block `a` dominate block `b`?
fn dom(f: &Fn, a: BlkId, b: BlkId) -> bool {
    a == b || sdom(f, a, b)
}

/// Does block `a` strictly dominate block `b`?
fn sdom(f: &Fn, a: BlkId, b: BlkId) -> bool {
    let mut cur = f.blks[b.0 as usize].idom;
    while let Some(c) = cur {
        if c == a {
            return true;
        }
        cur = f.blks[c.0 as usize].idom;
    }
    false
}

fn rendef(r: &mut Ref, bid: BlkId, stk: &mut [Vec<NameEntry>], f: &mut Fn) {
    if let Ref::Tmp(tid) = *r {
        let t = tid.0 as usize;
        if t >= f.tmps.len() || f.tmps[t].visit == 0 {
            return;
        }
        let r1 = refindex(t, f);
        if let Ref::Tmp(r1_tid) = r1 {
            f.tmps[r1_tid.0 as usize].visit = t as i32;
        }
        stk[t].push(NameEntry { r: r1, bid });
        *r = r1;
    }
}

fn getstk(t: usize, bid: BlkId, stk: &mut [Vec<NameEntry>], f: &Fn) -> Ref {
    while let Some(entry) = stk[t].last() {
        if dom(f, entry.bid, bid) {
            return entry.r;
        }
        stk[t].pop();
    }
    Ref::UNDEF
}

fn renblk(bid: BlkId, stk: &mut [Vec<NameEntry>], f: &mut Fn) {
    let b_idx = bid.0 as usize;

    for pi in 0..f.blks[b_idx].phi.len() {
        let mut to = f.blks[b_idx].phi[pi].to;
        rendef(&mut to, bid, stk, f);
        f.blks[b_idx].phi[pi].to = to;
    }

    let nins = f.blks[b_idx].ins.len();
    for i in 0..nins {
        for m in 0..2 {
            if let Ref::Tmp(tid) = f.blks[b_idx].ins[i].arg[m] {
                let t = tid.0 as usize;
                if t < f.tmps.len() && f.tmps[t].visit != 0 {
                    f.blks[b_idx].ins[i].arg[m] = getstk(t, bid, stk, f);
                }
            }
        }
        let mut to = f.blks[b_idx].ins[i].to;
        rendef(&mut to, bid, stk, f);
        f.blks[b_idx].ins[i].to = to;
    }

    if let Ref::Tmp(tid) = f.blks[b_idx].jmp.arg {
        let t = tid.0 as usize;
        if t < f.tmps.len() && f.tmps[t].visit != 0 {
            f.blks[b_idx].jmp.arg = getstk(t, bid, stk, f);
        }
    }

    let s1 = f.blks[b_idx].s1;
    let s2 = f.blks[b_idx].s2;
    let succs: [Option<BlkId>; 2] = [s1, if s2 == s1 { None } else { s2 }];
    for succ_opt in &succs {
        if let Some(s_id) = *succ_opt {
            let s_idx = s_id.0 as usize;
            let nphi = f.blks[s_idx].phi.len();
            for pi in 0..nphi {
                let phi_to = f.blks[s_idx].phi[pi].to;
                if let Ref::Tmp(tid) = phi_to {
                    let t = tid.0 as usize;
                    let visit = if t < f.tmps.len() { f.tmps[t].visit } else { 0 };
                    if visit != 0 {
                        let t_orig = visit as usize;
                        let arg = getstk(t_orig, bid, stk, f);
                        f.blks[s_idx].phi[pi].args.push(arg);
                        f.blks[s_idx].phi[pi].blks.push(bid);
                    }
                }
            }
        }
    }

    let mut children = Vec::new();
    let mut child = f.blks[b_idx].dom;
    while let Some(c) = child {
        children.push(c);
        child = f.blks[c.0 as usize].dlink;
    }
    for c in children {
        renblk(c, stk, f);
    }
}

/// Promote stack slots to SSA temporaries and construct SSA form.
///
/// Requires RPO and use information to have been computed.
/// Calls filldom, fillfron, filllive, then inserts phi nodes and renames
/// temporaries by walking the dominator tree.
pub fn ssa(f: &mut Fn) -> Result<(), String> {
    crate::analysis::control_flow::filldom(f);
    crate::analysis::control_flow::fillfron(f);
    crate::analysis::liveness::filllive_no_target(f);

    phiins(f)?;

    let nt = f.tmps.len();
    let mut stk: Vec<Vec<NameEntry>> = Vec::with_capacity(nt);
    for _ in 0..nt {
        stk.push(Vec::new());
    }

    renblk(f.start, &mut stk, f);
    Ok(())
}

/// Check whether a use of `target` in phi `p` (defined in block `b`) violates
/// SSA: the argument must come from a predecessor that is dominated by `b`.
fn phicheck(f: &Fn, phi: &Phi, def_bid: BlkId, target: Ref) -> bool {
    for (n, arg) in phi.args.iter().enumerate() {
        if *arg == target {
            let from = phi.blks[n];
            if from != def_bid && !sdom(f, def_bid, from) {
                return true; // violation
            }
        }
    }
    false
}

/// Verify SSA invariants: each temporary defined exactly once, every use
/// dominated by its definition, phi arguments come from correct predecessors.
pub fn ssacheck(f: &Fn) -> Result<(), String> {
    for t in TMP0 as usize..f.tmps.len() {
        let tmp = &f.tmps[t];
        if tmp.ndef > 1 {
            return Err(format!(
                "ssa temporary %{} is defined more than once",
                tmp.name
            ));
        }
        if tmp.nuse > 0 && tmp.ndef == 0 {
            let bu_bid = tmp.uses[0].bid;
            let bu_name = &f.blks[bu_bid as usize].name;
            if tmp.visit != 0 {
                return Err(format!("%{} violates the ssa invariant", tmp.name));
            } else {
                return Err(format!(
                    "ssa temporary %{} is used undefined in @{}",
                    tmp.name, bu_name
                ));
            }
        }
    }

    for blk_idx in 0..f.blks.len() {
        let bid = BlkId(blk_idx as u32);
        for pi in 0..f.blks[blk_idx].phi.len() {
            let r = f.blks[blk_idx].phi[pi].to;
            if let Ref::Tmp(tid) = r {
                let t = tid.0 as usize;
                let tmp = &f.tmps[t];
                for u in &tmp.uses {
                    let bu_bid = BlkId(u.bid);
                    match u.typ {
                        UseType::Phi => {
                            if let UseDetail::PhiIdx(phi_idx) = u.detail {
                                let bu_blk_idx = u.bid as usize;
                                if phi_idx < f.blks[bu_blk_idx].phi.len() as u32 {
                                    let use_phi = &f.blks[bu_blk_idx].phi[phi_idx as usize];
                                    if phicheck(f, use_phi, bid, r) {
                                        return Err(check_error(f, t));
                                    }
                                }
                            }
                        }
                        _ => {
                            if bu_bid != bid && !sdom(f, bid, bu_bid) {
                                return Err(check_error(f, t));
                            }
                        }
                    }
                }
            }
        }

        for ins_idx in 0..f.blks[blk_idx].ins.len() {
            let ito = f.blks[blk_idx].ins[ins_idx].to;
            if let Ref::Tmp(tid) = ito {
                let t = tid.0 as usize;
                let tmp = &f.tmps[t];
                for u in &tmp.uses {
                    let bu_bid = BlkId(u.bid);
                    match u.typ {
                        UseType::Phi => {
                            if let UseDetail::PhiIdx(phi_idx) = u.detail {
                                let bu_blk_idx = u.bid as usize;
                                if phi_idx < f.blks[bu_blk_idx].phi.len() as u32 {
                                    let use_phi = &f.blks[bu_blk_idx].phi[phi_idx as usize];
                                    if phicheck(f, use_phi, bid, ito) {
                                        return Err(check_error(f, t));
                                    }
                                }
                            }
                        }
                        _ => {
                            if bu_bid == bid {
                                if u.typ == UseType::Ins
                                    && let UseDetail::InsIdx(ui) = u.detail
                                    && ui <= ins_idx as u32
                                {
                                    return Err(check_error(f, t));
                                }
                            } else if !sdom(f, bid, bu_bid) {
                                return Err(check_error(f, t));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn check_error(f: &Fn, t: usize) -> String {
    let tmp = &f.tmps[t];
    if tmp.visit != 0 {
        format!("%{} violates the ssa invariant", tmp.name)
    } else {
        format!("ssa temporary %{} is used undefined", tmp.name)
    }
}
