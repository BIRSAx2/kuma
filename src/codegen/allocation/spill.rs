//! Register spill insertion.
//!
//! Spills low-cost temporaries when register pressure exceeds target limits.

use crate::analysis::liveness::liveon;
use crate::ir::builder::{InsBuffer, phicls};
use crate::ir::internal::{BSet, Cls, Fn, Ins, Op, Ref, StackSlot, TMP0, Target, TmpId};

/// Aggregate looping information at loop headers.
fn aggreg(f: &mut Fn, hd: u32, b: u32) {
    let b_blk = &f.blks[b as usize];
    let b_gen_clone = b_blk.live_gen.clone();
    let b_nlive = b_blk.nlive;

    let hd_blk = &mut f.blks[hd as usize];
    hd_blk.live_gen.union(&b_gen_clone);
    for (k, &live) in b_nlive.iter().enumerate() {
        if live > hd_blk.nlive[k] {
            hd_blk.nlive[k] = live;
        }
    }
}

fn tmpuse(r: Ref, is_use: bool, loop_cost: i32, f: &mut Fn) {
    match r {
        Ref::Mem(mid) => {
            let m = f.mems[mid.0 as usize].clone();
            tmpuse(m.base, true, loop_cost, f);
            tmpuse(m.index, true, loop_cost, f);
        }
        Ref::Tmp(tid) if tid.0 >= TMP0 => {
            let t = &mut f.tmps[tid.0 as usize];
            if is_use {
                t.nuse += 1;
            } else {
                t.ndef += 1;
            }
            t.cost = t.cost.wrapping_add(loop_cost as u32);
        }
        _ => {}
    }
}

/// Compute spill costs and block loop depths.
///
/// Requires RPO and predecessor information.
pub fn fillcost(f: &mut Fn) {
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    crate::analysis::control_flow::loopiter(f, &mut |hd, b| {
        pairs.push((hd.0, b.0));
    });
    for (hd, b) in pairs {
        aggreg(f, hd, b);
    }

    let ntmp = f.tmps.len();
    for ti in 0..ntmp {
        if ti < TMP0 as usize {
            f.tmps[ti].cost = u32::MAX;
        } else {
            f.tmps[ti].cost = 0;
        }
        f.tmps[ti].nuse = 0;
        f.tmps[ti].ndef = 0;
    }

    let rpo = f.rpo.clone();
    for &bid in &rpo {
        let b_idx = bid.0 as usize;

        let nphi = f.blks[b_idx].phi.len();
        for pi in 0..nphi {
            let phi_to = f.blks[b_idx].phi[pi].to;
            tmpuse(phi_to, false, 0, f);
            let narg = f.blks[b_idx].phi[pi].narg();
            for a in 0..narg {
                let blk = f.blks[b_idx].phi[pi].blks[a];
                let n = f.blks[blk.0 as usize].loop_depth;
                if let Ref::Tmp(tid) = phi_to {
                    f.tmps[tid.0 as usize].cost =
                        f.tmps[tid.0 as usize].cost.wrapping_add(n as u32);
                }
                let arg = f.blks[b_idx].phi[pi].args[a];
                tmpuse(arg, true, n, f);
            }
        }

        let n = f.blks[b_idx].loop_depth;

        let nins = f.blks[b_idx].ins.len();
        for ii in 0..nins {
            let ins = f.blks[b_idx].ins[ii];
            tmpuse(ins.to, false, n, f);
            tmpuse(ins.arg[0], true, n, f);
            tmpuse(ins.arg[1], true, n, f);
        }

        let jmp_arg = f.blks[b_idx].jmp.arg;
        tmpuse(jmp_arg, true, n, f);
    }
}

/// Slot packing state.
struct SlotState {
    locs: i32,
    slot4: i32,
    slot8: i32,
}

impl SlotState {
    fn new(locs: i32) -> Self {
        Self {
            locs,
            slot4: 0,
            slot8: 0,
        }
    }

    /// Assign a packed stack slot to temporary `t`.
    fn slot(&mut self, t: u32, tmps: &mut [crate::ir::internal::Tmp]) -> Ref {
        assert!(t >= TMP0, "cannot spill register");
        if let Some(slot) = tmps[t as usize].slot {
            return Ref::Slot(slot);
        }

        let s;
        if tmps[t as usize].cls.is_wide() {
            s = self.slot8;
            if self.slot4 == self.slot8 {
                self.slot4 += 2;
            }
            self.slot8 += 2;
        } else {
            s = self.slot4;
            if self.slot4 == self.slot8 {
                self.slot8 += 2;
                self.slot4 += 1;
            } else {
                self.slot4 = self.slot8;
            }
        }
        let s = s + self.locs;
        let slot = StackSlot::from_signed(s);
        tmps[t as usize].slot = Some(slot);
        Ref::Slot(slot)
    }
}

/// Sort by cost descending (highest cost first = most expensive to spill).
fn sort_by_cost(arr: &mut [u32], tmps: &[crate::ir::internal::Tmp]) {
    arr.sort_by(|&a, &b| tmps[b as usize].cost.cmp(&tmps[a as usize].cost));
}

/// Sort by: first prefer items in `fst` set, then by cost descending.
fn sort_by_fst_then_cost(arr: &mut [u32], fst: &BSet, tmps: &[crate::ir::internal::Tmp]) {
    arr.sort_by(|&a, &b| {
        let fa = fst.has(b) as i32 - fst.has(a) as i32;
        if fa != 0 {
            return fa.cmp(&0);
        }
        tmps[b as usize].cost.cmp(&tmps[a as usize].cost)
    });
}

/// Restricts bitset `b` to hold at most `k` temporaries, preferring those
/// present in `fst_set` (if given), then those with the largest spill cost.
/// Excess temps are spilled.
fn limit(
    b: &mut BSet,
    k: i32,
    fst_set: Option<&BSet>,
    slots: &mut SlotState,
    tmps: &mut [crate::ir::internal::Tmp],
) {
    let nt = b.count() as i32;
    if nt <= k {
        return;
    }

    let mut arr: Vec<u32> = b.iter().collect();
    b.zero();

    if let Some(fst) = fst_set {
        sort_by_fst_then_cost(&mut arr, fst, tmps);
    } else {
        sort_by_cost(&mut arr, tmps);
    }

    let k = k.max(0) as usize;
    for (i, &t) in arr.iter().enumerate() {
        if i < k {
            b.set(t);
        } else {
            slots.slot(t, tmps);
        }
    }
}

/// Spills temporaries to fit the target limits. Splits by register class,
/// limits each separately, then unions.
struct RegisterLimits<'a> {
    masks: &'a [BSet; 2],
    target: &'a Target,
}

fn limit2(
    b: &mut BSet,
    reserved: [i32; 2],
    fst_set: Option<&BSet>,
    limits: &RegisterLimits<'_>,
    slots: &mut SlotState,
    tmps: &mut [crate::ir::internal::Tmp],
) {
    let ntmp = tmps.len() as u32;
    let mut b2 = BSet::new(ntmp);
    b2.copy_from(b);
    b.inter(&limits.masks[0]);
    b2.inter(&limits.masks[1]);
    limit(b, limits.target.ngpr - reserved[0], fst_set, slots, tmps);
    limit(
        &mut b2,
        limits.target.nfpr - reserved[1],
        fst_set,
        slots,
        tmps,
    );
    b.union(&b2);
}

fn sethint(u: &BSet, r: u64, tmps: &mut [crate::ir::internal::Tmp]) {
    for t in u.iter() {
        if t >= TMP0 {
            let pc = phicls(t as usize, tmps);
            tmps[pc].hint.avoid.insert_all(r);
        }
    }
}

/// Emit reloads for temps in `u` that are not in `v`.
fn reloads(
    u: &BSet,
    v: &BSet,
    buf: &mut InsBuffer,
    slots: &mut SlotState,
    tmps: &mut [crate::ir::internal::Tmp],
) {
    for t in u.iter() {
        if t >= TMP0 && !v.has(t) {
            let cls = tmps[t as usize].cls;
            let s = slots.slot(t, tmps);
            buf.emit(Op::Load, cls, Ref::Tmp(TmpId(t)), s, Ref::R);
        }
    }
}

/// Emit a store for a ref if it has a slot.
fn store(r: Ref, buf: &mut InsBuffer, tmps: &[crate::ir::internal::Tmp]) {
    if let Ref::Tmp(tid) = r
        && let Some(slot) = tmps[tid.0 as usize].slot
    {
        let store_op = match tmps[tid.0 as usize].cls {
            Cls::Kw => Op::Storew,
            Cls::Kl => Op::Storel,
            Cls::Ks => Op::Stores,
            Cls::Kd => Op::Stored,
            Cls::Kx => Op::Storew,
        };
        buf.emit(store_op, Cls::Kw, Ref::R, r, Ref::Slot(slot));
    }
}

fn regcpy(ins: &Ins) -> bool {
    ins.op == Op::Copy && crate::ir::internal::isreg(ins.arg[0])
}

/// Process a block of consecutive register copy instructions as a single unit.
///
/// Returns the instruction index before the block.
fn dopm(
    f: &mut Fn,
    b_idx: usize,
    mut i: usize,
    v: &mut BSet,
    buf: &mut InsBuffer,
    slots: &mut SlotState,
    limits: &RegisterLimits<'_>,
) -> usize {
    let ntmp = f.tmps.len() as u32;
    let mut u = BSet::new(ntmp);

    let start = {
        let mut s = i;
        while s > 0 && regcpy(&f.blks[b_idx].ins[s - 1]) {
            s -= 1;
        }
        s
    };

    let j = i + 1;
    loop {
        let ins = f.blks[b_idx].ins[i];
        if ins.to != Ref::R {
            let tv = ins.to.val();
            if v.has(tv) {
                v.clr(tv);
                store(ins.to, buf, &f.tmps);
            }
        }
        if let Ref::Tmp(tid) = ins.arg[0] {
            v.set(tid.0);
        }
        if i == start {
            break;
        }
        i -= 1;
    }

    u.copy_from(v);

    if start > 0 && f.blks[b_idx].ins[start - 1].op == Op::Call {
        let call_ref = f.blks[b_idx].ins[start - 1].arg[1];
        let retregs = limits.target.retregs(call_ref, None);
        if !v.bits_raw().is_empty() {
            v.bits_raw_mut()[0] &= !retregs;
        }
        limit2(
            v,
            [limits.target.nrsave[0], limits.target.nrsave[1]],
            None,
            limits,
            slots,
            &mut f.tmps,
        );
        let mut _r: u64 = 0;
        for &rs in limits.target.rsave {
            if rs < 0 {
                break;
            }
            _r |= 1u64 << rs;
        }
        let argregs = limits.target.argregs(call_ref, None);
        if !v.bits_raw().is_empty() {
            v.bits_raw_mut()[0] |= argregs;
        }
    } else {
        limit2(v, [0, 0], None, limits, slots, &mut f.tmps);
    }

    let r_val = v.bits_raw().first().copied().unwrap_or(0);
    sethint(v, r_val, &mut f.tmps);
    reloads(&u, v, buf, slots, &mut f.tmps);

    for idx in (start..j).rev() {
        buf.emiti(f.blks[b_idx].ins[idx]);
    }

    start
}

fn merge(u: &mut BSet, bu_loop: i32, v: &BSet, bv_loop: i32, tmps: &[crate::ir::internal::Tmp]) {
    if bu_loop <= bv_loop {
        u.union(v);
    } else {
        for t in v.iter() {
            if tmps[t as usize].slot.is_none() {
                u.set(t);
            }
        }
    }
}

/// Insert spill and reload instructions.
///
/// Requires spill costs, RPO, and liveness information.
pub fn spill(f: &mut Fn, t: &Target) {
    let ntmp = f.tmps.len() as u32;

    let mut u = BSet::new(ntmp);
    let mut v = BSet::new(ntmp);
    let mut w = BSet::new(ntmp);
    let mut mask = [BSet::new(ntmp), BSet::new(ntmp)];

    let mut slots = SlotState::new(f.slot);

    for ti in 0..ntmp as i32 {
        let mut k = 0;
        if ti >= t.fpr0 && ti < t.fpr0 + t.nfpr {
            k = 1;
        }
        if ti >= TMP0 as i32 {
            let cls = f.tmps[ti as usize].cls;
            k = cls.base().max(0) as usize;
        }
        mask[k].set(ti as u32);
    }
    let limits = RegisterLimits {
        masks: &mask,
        target: t,
    };

    let rpo = f.rpo.clone();
    for bp_idx in (0..rpo.len()).rev() {
        let bid = rpo[bp_idx];
        let b_idx = bid.0 as usize;
        let mut buf = InsBuffer::new();

        let s1 = f.blks[b_idx].s1;
        let s2 = f.blks[b_idx].s2;

        let mut hd: Option<u32> = None;
        if let Some(s) = s1
            && f.blks[s.0 as usize].id <= f.blks[b_idx].id
        {
            hd = Some(s.0);
        }
        if let Some(s) = s2
            && f.blks[s.0 as usize].id <= f.blks[b_idx].id
            && hd.is_none_or(|header| f.blks[s.0 as usize].id >= f.blks[header as usize].id)
        {
            hd = Some(s.0);
        }

        if let Some(hd_id) = hd {
            v.zero();
            if !f.blks[hd_id as usize].live_gen.bits_raw().is_empty() {
                f.blks[hd_id as usize].live_gen.bits_raw_mut()[0] |= t.rglob;
            }
            for (k, class_mask) in mask.iter().enumerate() {
                let n = if k == 0 { t.ngpr } else { t.nfpr };
                u.copy_from(&f.blks[b_idx].out);
                u.inter(class_mask);
                w.copy_from(&u);
                u.inter(&f.blks[hd_id as usize].live_gen);
                w.diff(&f.blks[hd_id as usize].live_gen);
                if (u.count() as i32) < n {
                    let j = w.count() as i32;
                    let l = f.blks[hd_id as usize].nlive[k];
                    limit(&mut w, n - (l - j), None, &mut slots, &mut f.tmps);
                    u.union(&w);
                } else {
                    limit(&mut u, n, None, &mut slots, &mut f.tmps);
                }
                v.union(&u);
            }
        } else if let Some(s1_id) = s1 {
            v.zero();
            liveon(f, &mut w, bid.0, s1_id.0);
            let b_loop = f.blks[b_idx].loop_depth;
            let s1_loop = f.blks[s1_id.0 as usize].loop_depth;
            merge(&mut v, b_loop, &w, s1_loop, &f.tmps);

            if let Some(s2_id) = s2 {
                liveon(f, &mut u, bid.0, s2_id.0);
                let s2_loop = f.blks[s2_id.0 as usize].loop_depth;
                merge(&mut v, b_loop, &u, s2_loop, &f.tmps);
                w.inter(&u);
            }
            limit2(&mut v, [0, 0], Some(&w), &limits, &mut slots, &mut f.tmps);
        } else {
            v.copy_from(&f.blks[b_idx].out);
            if let Ref::Call(_) = f.blks[b_idx].jmp.arg {
                let retregs = t.retregs(f.blks[b_idx].jmp.arg, None);
                if !v.bits_raw().is_empty() {
                    v.bits_raw_mut()[0] |= retregs;
                }
            }
        }

        for ti in f.blks[b_idx].out.iter() {
            if ti >= TMP0 && !v.has(ti) {
                slots.slot(ti, &mut f.tmps);
            }
        }
        f.blks[b_idx].out.copy_from(&v);

        if let Ref::Tmp(tid) = f.blks[b_idx].jmp.arg {
            let tv = tid.0;
            assert!(f.tmps[tv as usize].cls.base() == 0);
            let lvarg = v.has(tv);
            v.set(tv);
            u.copy_from(&v);
            limit2(&mut v, [0, 0], None, &limits, &mut slots, &mut f.tmps);
            if !v.has(tv) {
                if !lvarg {
                    u.clr(tv);
                }
                f.blks[b_idx].jmp.arg = slots.slot(tv, &mut f.tmps);
            }
            reloads(&u, &v, &mut buf, &mut slots, &mut f.tmps);
        }

        let nins = f.blks[b_idx].ins.len();
        let mut i = nins;
        while i > 0 {
            i -= 1;

            if regcpy(&f.blks[b_idx].ins[i]) {
                i = dopm(f, b_idx, i, &mut v, &mut buf, &mut slots, &limits);
                continue;
            }

            w.zero();
            let ins = f.blks[b_idx].ins[i];

            if ins.to != Ref::R {
                let tv = ins.to.val();
                if v.has(tv) {
                    v.clr(tv);
                } else {
                    assert!(tv >= TMP0, "dead reg");
                    v.set(tv);
                    w.set(tv);
                }
            }

            let mut j = t.memargs(ins.op);
            for n in 0..2 {
                if let Ref::Mem(_) = ins.arg[n] {
                    j -= 1;
                }
            }

            let mut lvarg = [false; 2];
            for (n, &arg) in ins.arg.iter().enumerate() {
                match arg {
                    Ref::Mem(mid) => {
                        let m = f.mems[mid.0 as usize].clone();
                        if let Ref::Tmp(tid) = m.base {
                            v.set(tid.0);
                            w.set(tid.0);
                        }
                        if let Ref::Tmp(tid) = m.index {
                            v.set(tid.0);
                            w.set(tid.0);
                        }
                    }
                    Ref::Tmp(tid) => {
                        let tv = tid.0;
                        lvarg[n] = v.has(tv);
                        v.set(tv);
                        if j <= 0 {
                            w.set(tv);
                        }
                        j -= 1;
                    }
                    _ => {}
                }
            }

            u.copy_from(&v);
            limit2(&mut v, [0, 0], Some(&w), &limits, &mut slots, &mut f.tmps);

            for (n, was_live) in lvarg.iter().copied().enumerate() {
                if let Ref::Tmp(tid) = f.blks[b_idx].ins[i].arg[n] {
                    let tv = tid.0;
                    if !v.has(tv) {
                        if !was_live {
                            u.clr(tv);
                        }
                        f.blks[b_idx].ins[i].arg[n] = slots.slot(tv, &mut f.tmps);
                    }
                }
            }

            reloads(&u, &v, &mut buf, &mut slots, &mut f.tmps);

            if ins.to != Ref::R {
                let tv = ins.to.val();
                store(ins.to, &mut buf, &f.tmps);
                if tv >= TMP0 {
                    v.clr(tv);
                }
            }

            buf.emiti(f.blks[b_idx].ins[i]);

            let r = v.bits_raw().first().copied().unwrap_or(0);
            if r != 0 {
                sethint(&v, r, &mut f.tmps);
            }
        }

        let nphi = f.blks[b_idx].phi.len();
        for pi in 0..nphi {
            let phi_to = f.blks[b_idx].phi[pi].to;
            if let Ref::Tmp(tid) = phi_to {
                let tv = tid.0;
                if v.has(tv) {
                    v.clr(tv);
                    store(phi_to, &mut buf, &f.tmps);
                } else if f.blks[b_idx].r#in.has(tv) {
                    f.blks[b_idx].phi[pi].to = slots.slot(tv, &mut f.tmps);
                }
            }
        }

        f.blks[b_idx].r#in.copy_from(&v);

        let new_ins = buf.finish();
        f.blks[b_idx].ins = new_ins;
    }

    slots.slot8 += slots.slot8 & 3;
    f.slot += slots.slot8;
}
