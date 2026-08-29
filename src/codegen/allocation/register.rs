//! Register allocation.
//!
//! Assigns physical registers to virtual temporaries, resolves parallel
//! moves, and inserts bridge blocks for phi resolution.

use crate::ir::builder::{InsBuffer, phicls};
use crate::ir::internal::{
    BSet, Cls, Fn, Ins, Jmp, Op, PhysicalRegister, Ref, TMP0, Target, TmpId,
};

/// Maximum number of simultaneous register mappings.
const RMAP_MAX: usize = TMP0 as usize;

/// Current mapping between temporaries and physical registers.
#[derive(Clone)]
struct RMap {
    /// Temporary for each mapping slot.
    t: [i32; RMAP_MAX],
    /// Register for each mapping slot.
    r: [i32; RMAP_MAX],
    /// Wait list: `w[reg]` = tmp that wants this register (for hints).
    w: [i32; RMAP_MAX],
    /// Bitset tracking which tmps and registers are allocated.
    b: BSet,
    /// Number of active mappings.
    n: usize,
}

impl RMap {
    fn new(ntmp: u32) -> Self {
        Self {
            t: [0; RMAP_MAX],
            r: [0; RMAP_MAX],
            w: [0; RMAP_MAX],
            b: BSet::new(ntmp),
            n: 0,
        }
    }

    fn copy_from(&mut self, other: &RMap) {
        self.t = other.t;
        self.r = other.r;
        self.w = other.w;
        self.b.copy_from(&other.b);
        self.n = other.n;
    }
}

struct PMEntry {
    src: Ref,
    dst: Ref,
    cls: Cls,
}

/// Parallel move state.
struct PMState {
    entries: Vec<PMEntry>,
}

impl PMState {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    /// Add a parallel move entry.
    fn add(&mut self, src: Ref, dst: Ref, cls: Cls) {
        assert!(self.entries.len() < RMAP_MAX, "too many parallel moves");
        self.entries.push(PMEntry { src, dst, cls });
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum PMStat {
    ToMove,
    Moving,
    Moved,
}

#[derive(Copy, Clone)]
enum OperandLocation {
    Register(PhysicalRegister),
    Stack(crate::ir::internal::StackSlot),
}

impl OperandLocation {
    fn as_reference(self) -> Ref {
        match self {
            Self::Register(register) => Ref::Tmp(TmpId(register.index() as u32)),
            Self::Stack(slot) => Ref::Slot(slot),
        }
    }
}

fn hint_reg(t: usize, tmps: &mut [crate::ir::internal::Tmp]) -> i32 {
    let pc = phicls(t, tmps);
    tmps[pc].hint.register.map_or(-1, PhysicalRegister::index)
}

fn sethint(t: usize, r: i32, loop_depth: i32, tmps: &mut [crate::ir::internal::Tmp]) {
    let pc = phicls(t, tmps);
    let p = &mut tmps[pc];
    if p.hint.register.is_none() || p.hint.w > loop_depth {
        p.hint.register = PhysicalRegister::from_index(r);
        p.hint.w = loop_depth;
        tmps[t].visit = -1;
    }
}

/// Find the register assigned to temporary `t`, or -1 if not mapped.
fn rfind(m: &RMap, t: i32) -> i32 {
    for i in 0..m.n {
        if m.t[i] == t {
            return m.r[i];
        }
    }
    -1
}

/// Get a Ref for temporary `t`: either its assigned register or its spill slot.
fn rref(m: &RMap, t: i32, tmps: &[crate::ir::internal::Tmp]) -> Ref {
    let r = rfind(m, t);
    let location = if r == -1 {
        OperandLocation::Stack(
            tmps[t as usize]
                .slot
                .expect("temporary should have spilled"),
        )
    } else {
        OperandLocation::Register(
            PhysicalRegister::from_index(r).expect("allocated register index is non-negative"),
        )
    };
    location.as_reference()
}

/// Add a mapping: temporary `t` → register `r`.
fn radd(m: &mut RMap, t: i32, r: i32, target: &Target, regu: &mut u64) {
    assert!(t >= TMP0 as i32 || t == r, "invalid temporary");
    assert!(
        (target.gpr0 <= r && r < target.gpr0 + target.ngpr)
            || (target.fpr0 <= r && r < target.fpr0 + target.nfpr),
        "invalid register {}",
        r
    );
    assert!(!m.b.has(t as u32), "temporary {} has mapping", t);
    assert!(!m.b.has(r as u32), "register {} already allocated", r);
    assert!(
        m.n <= (target.ngpr + target.nfpr) as usize,
        "too many mappings"
    );
    m.b.set(t as u32);
    m.b.set(r as u32);
    m.t[m.n] = t;
    m.r[m.n] = r;
    m.n += 1;
    *regu |= 1u64 << r;
}

/// Try to allocate a register for temporary `t`.
/// If `try_only` is true and no register is available, returns Ref::R instead of panicking.
fn ralloctry(
    m: &mut RMap,
    t: i32,
    try_only: bool,
    target: &Target,
    regu: &mut u64,
    tmps: &mut [crate::ir::internal::Tmp],
) -> Ref {
    if t < TMP0 as i32 {
        assert!(m.b.has(t as u32));
        return Ref::Tmp(TmpId(t as u32));
    }
    if m.b.has(t as u32) {
        let r = rfind(m, t);
        assert!(r != -1);
        return Ref::Tmp(TmpId(r as u32));
    }

    let mut r = tmps[t as usize].visit;
    if r == -1 || m.b.has(r as u32) {
        r = hint_reg(t as usize, tmps);
    }
    if r == -1 || m.b.has(r as u32) {
        if try_only {
            return Ref::R;
        }
        let pc = phicls(t as usize, tmps);
        let hint_mask = tmps[pc].hint.avoid.bits();
        let used = m.b.bits_raw().first().copied().unwrap_or(0);
        let regs = hint_mask | used;

        let (r0, r1) = if tmps[t as usize].cls.base() == 0 {
            (target.gpr0, target.gpr0 + target.ngpr)
        } else {
            (target.fpr0, target.fpr0 + target.nfpr)
        };

        let mut found = -1;
        for ri in r0..r1 {
            if regs & (1u64 << ri) == 0 {
                found = ri;
                break;
            }
        }
        if found == -1 {
            for ri in r0..r1 {
                if !m.b.has(ri as u32) {
                    found = ri;
                    break;
                }
            }
        }
        assert!(found != -1, "no more regs");
        r = found;
    }

    radd(m, t, r, target, regu);
    sethint(t as usize, r, i32::MAX, tmps);
    tmps[t as usize].visit = r;

    let h = hint_reg(t as usize, tmps);
    if h != -1 && h != r && (r as usize) < RMAP_MAX {
        m.w[r as usize] = t;
    }
    Ref::Tmp(TmpId(r as u32))
}

#[inline]
fn ralloc(
    m: &mut RMap,
    t: i32,
    target: &Target,
    regu: &mut u64,
    tmps: &mut [crate::ir::internal::Tmp],
) -> Ref {
    ralloctry(m, t, false, target, regu, tmps)
}

/// Free the mapping for temporary `t`. Returns the register it was using, or -1.
fn rfree(m: &mut RMap, t: i32, target: &Target) -> i32 {
    assert!(
        t >= TMP0 as i32 || (1u64 << t) & target.rglob == 0,
        "cannot free global register"
    );
    if !m.b.has(t as u32) {
        return -1;
    }
    let mut idx = 0;
    while m.t[idx] != t {
        idx += 1;
        assert!(idx < m.n);
    }
    let r = m.r[idx];
    m.b.clr(t as u32);
    m.b.clr(r as u32);
    m.n -= 1;
    for j in idx..m.n {
        m.t[j] = m.t[j + 1];
        m.r[j] = m.r[j + 1];
    }
    assert!(t >= TMP0 as i32 || t == r);
    r
}

/// Resolve one entry in the parallel move, handling cycles with swaps.
fn pmrec(pm: &[PMEntry], status: &mut [PMStat], i: usize, k: &mut Cls, buf: &mut InsBuffer) -> i32 {
    if pm[i].src == pm[i].dst {
        status[i] = PMStat::Moved;
        return -1;
    }

    assert!(pm[i].cls.base() == k.base());
    let merged = (pm[i].cls as i8) | (*k as i8);
    *k = Cls::from_i8(merged);

    let npm = pm.len();
    let mut j = 0;
    while j < npm {
        if pm[j].dst == pm[i].src {
            break;
        }
        j += 1;
    }

    let st = if j == npm { PMStat::Moved } else { status[j] };

    let c;
    match st {
        PMStat::Moving => {
            c = j as i32;
            buf.emit(Op::Swap, *k, Ref::R, pm[i].src, pm[i].dst);
        }
        PMStat::ToMove => {
            status[i] = PMStat::Moving;
            let ci = pmrec(pm, status, j, k, buf);
            if ci == i as i32 {
                c = -1;
            } else if ci != -1 {
                c = ci;
                buf.emit(Op::Swap, *k, Ref::R, pm[i].src, pm[i].dst);
            } else {
                c = -1;
                buf.emit(Op::Copy, pm[i].cls, pm[i].dst, pm[i].src, Ref::R);
            }
        }
        PMStat::Moved => {
            c = -1;
            buf.emit(Op::Copy, pm[i].cls, pm[i].dst, pm[i].src, Ref::R);
        }
    }
    status[i] = PMStat::Moved;
    c
}

/// Generate instructions for all entries in the parallel move buffer.
fn pmgen(pm: &PMState, buf: &mut InsBuffer) {
    let npm = pm.entries.len();
    if npm == 0 {
        return;
    }
    let mut status = vec![PMStat::ToMove; npm];
    for i in 0..npm {
        if status[i] == PMStat::ToMove {
            let mut k = pm.entries[i].cls;
            pmrec(&pm.entries, &mut status, i, &mut k, buf);
        }
    }
}

fn do_move(
    r: i32,
    to: Ref,
    m: &mut RMap,
    target: &Target,
    regu: &mut u64,
    tmps: &mut [crate::ir::internal::Tmp],
) {
    let r1 = if to == Ref::R {
        -1
    } else {
        rfree(m, to.val() as i32, target)
    };

    if m.b.has(r as u32) {
        assert!(r1 != r);
        let mut n = 0;
        while m.r[n] != r {
            n += 1;
            assert!(n < m.n);
        }
        let t = m.t[n];
        rfree(m, t, target);
        m.b.set(r as u32);
        ralloc(m, t, target, regu, tmps);
        m.b.clr(r as u32);
    }

    let t = if to == Ref::R { r } else { to.val() as i32 };
    radd(m, t, r, target, regu);
}

fn regcpy(ins: &Ins) -> bool {
    ins.op == Op::Copy && crate::ir::internal::isreg(ins.arg[0])
}

/// Process a block of consecutive register copy instructions.
#[derive(Copy, Clone)]
struct InstructionCursor {
    block: usize,
    index: usize,
}

fn rega_dopm(
    f: &mut Fn,
    cursor: InstructionCursor,
    m: &mut RMap,
    target: &Target,
    regu: &mut u64,
    pm: &mut PMState,
    buf: &mut InsBuffer,
) -> usize {
    let InstructionCursor {
        block: b_idx,
        index: i_end,
    } = cursor;
    let m0_t = m.t;
    let m0_r = m.r;
    let m0_n = m.n;

    let mut start = i_end;
    while start > 0 && regcpy(&f.blks[b_idx].ins[start - 1]) {
        start -= 1;
    }

    let mut i = i_end;
    loop {
        let ins = f.blks[b_idx].ins[i];
        do_move(
            ins.arg[0].val() as i32,
            ins.to,
            m,
            target,
            regu,
            &mut f.tmps,
        );
        if i == start {
            break;
        }
        i -= 1;
    }

    assert!(m0_n <= m.n);

    if start > 0 && f.blks[b_idx].ins[start - 1].op == Op::Call {
        let call_ref = f.blks[b_idx].ins[start - 1].arg[1];
        let def = target.retregs(call_ref, None) | target.rglob;
        for &rs in target.rsave {
            if rs < 0 {
                break;
            }
            if (1u64 << rs) & def == 0 {
                do_move(rs, Ref::R, m, target, regu, &mut f.tmps);
            }
        }
    }

    pm.clear();
    for n in 0..m.n {
        let t = m.t[n];
        let s = f.tmps[t as usize].slot;
        let r1 = m.r[n];
        let mut r_old = -1;
        for j in 0..m0_n {
            if m0_t[j] == t {
                r_old = m0_r[j];
                break;
            }
        }
        if r_old != -1 {
            pm.add(
                Ref::Tmp(TmpId(r1 as u32)),
                Ref::Tmp(TmpId(r_old as u32)),
                f.tmps[t as usize].cls,
            );
        } else if let Some(slot) = s {
            pm.add(
                Ref::Tmp(TmpId(r1 as u32)),
                Ref::Slot(slot),
                f.tmps[t as usize].cls,
            );
        }
    }

    for ip in start..=i_end {
        let ins = f.blks[b_idx].ins[ip];
        if ins.to != Ref::R {
            rfree(m, ins.to.val() as i32, target);
        }
        let r = ins.arg[0].val() as i32;
        if rfind(m, r) == -1 {
            radd(m, r, r, target, regu);
        }
    }

    pmgen(pm, buf);
    start
}

fn doblk(
    f: &mut Fn,
    b_idx: usize,
    cur: &mut RMap,
    target: &Target,
    regu: &mut u64,
    pm: &mut PMState,
    stmov: &mut u32,
) {
    if let Ref::Tmp(tid) = f.blks[b_idx].jmp.arg {
        f.blks[b_idx].jmp.arg = ralloc(cur, tid.0 as i32, target, regu, &mut f.tmps);
    }

    let mut buf = InsBuffer::new();
    let nins = f.blks[b_idx].ins.len();
    let mut i1 = nins;

    while i1 > 0 {
        i1 -= 1;
        let ins = f.blks[b_idx].ins[i1];
        buf.emiti(ins);

        let mut rf: i32 = -1;

        match ins.op {
            Op::Call => {
                let call_ref = f.blks[b_idx].ins[i1].arg[1];
                let rs = target.argregs(call_ref, None) | target.rglob;
                for &rsave in target.rsave {
                    if rsave < 0 {
                        break;
                    }
                    if (1u64 << rsave) & rs == 0 {
                        rfree(cur, rsave, target);
                    }
                }
            }
            Op::Copy if regcpy(&f.blks[b_idx].ins[i1]) => {
                buf = {
                    let mut new_buf = InsBuffer::new();
                    let sl = buf.as_slice();
                    for &item in sl.iter().take(sl.len() - 1) {
                        new_buf.emiti(item);
                    }
                    new_buf
                };
                let old_len = buf.len();
                i1 = rega_dopm(
                    f,
                    InstructionCursor {
                        block: b_idx,
                        index: i1,
                    },
                    cur,
                    target,
                    regu,
                    pm,
                    &mut buf,
                );
                *stmov += (buf.len() - old_len) as u32;
                continue;
            }
            _ => {
                if ins.op == Op::Copy
                    && crate::ir::internal::isreg(ins.to)
                    && matches!(ins.arg[0], Ref::Tmp(tid) if tid.0 >= TMP0)
                    && let Ref::Tmp(to_tid) = ins.to
                {
                    sethint(
                        ins.arg[0].val() as usize,
                        to_tid.0 as i32,
                        i32::MAX,
                        &mut f.tmps,
                    );
                }

                if ins.to != Ref::R {
                    let r = ins.to.val() as i32;
                    if r < TMP0 as i32 && (1u64 << r) & target.rglob != 0 {
                    } else {
                        rf = rfree(cur, r, target);
                        if rf == -1 {
                            assert!(!crate::ir::internal::isreg(ins.to));
                            buf = {
                                let mut new_buf = InsBuffer::new();
                                let sl = buf.as_slice();
                                for &item in sl.iter().take(sl.len() - 1) {
                                    new_buf.emiti(item);
                                }
                                new_buf
                            };
                            continue;
                        }
                        {
                            let last_idx = buf.len() - 1;
                            let sl = buf.as_slice();
                            let mut patched = sl[last_idx];
                            patched.to = Ref::Tmp(TmpId(rf as u32));
                            let mut new_buf = InsBuffer::new();
                            for &item in sl.iter().take(last_idx) {
                                new_buf.emiti(item);
                            }
                            new_buf.emiti(patched);
                            buf = new_buf;
                        }
                    }
                }
            }
        }

        let cur_ins = *buf.as_slice().last().unwrap();
        let mut arg_refs: Vec<(usize, usize, i32)> = Vec::new(); // (arg_idx, mem_field, tmp_val)

        for x in 0..2 {
            match cur_ins.arg[x] {
                Ref::Mem(mid) => {
                    let m = &f.mems[mid.0 as usize];
                    if let Ref::Tmp(tid) = m.base {
                        arg_refs.push((x, 0, tid.0 as i32)); // 0 = base
                    }
                    if let Ref::Tmp(tid) = m.index {
                        arg_refs.push((x, 1, tid.0 as i32)); // 1 = index
                    }
                }
                Ref::Tmp(tid) => {
                    arg_refs.push((x, 2, tid.0 as i32)); // 2 = direct arg
                }
                _ => {}
            }
        }

        arg_refs.sort_by(|a, b| {
            let ha = hint_reg(a.2 as usize, &mut f.tmps) != -1;
            let hb = hint_reg(b.2 as usize, &mut f.tmps) != -1;
            hb.cmp(&ha)
        });

        for &(arg_idx, mem_field, tmp_val) in &arg_refs {
            let allocated = ralloc(cur, tmp_val, target, regu, &mut f.tmps);

            let last_idx = buf.len() - 1;
            let sl = buf.as_slice();
            let mut patched = sl[last_idx];

            match mem_field {
                0 | 1 => {
                    if let Ref::Mem(mid) = patched.arg[arg_idx] {
                        if mem_field == 0 {
                            f.mems[mid.0 as usize].base = allocated;
                        } else {
                            f.mems[mid.0 as usize].index = allocated;
                        }
                    }
                }
                _ => {
                    patched.arg[arg_idx] = allocated;
                }
            }

            let mut new_buf = InsBuffer::new();
            for &item in sl.iter().take(last_idx) {
                new_buf.emiti(item);
            }
            new_buf.emiti(patched);
            buf = new_buf;
        }

        {
            let last = buf.as_slice().last().unwrap();
            if last.op == Op::Copy && last.to == last.arg[0] {
                buf = {
                    let mut new_buf = InsBuffer::new();
                    let sl = buf.as_slice();
                    for &item in sl.iter().take(sl.len() - 1) {
                        new_buf.emiti(item);
                    }
                    new_buf
                };
            }
        }

        if rf != -1 {
            let rf_u = rf as usize;
            if rf_u < RMAP_MAX {
                let wt = cur.w[rf_u];
                if wt != 0 && !cur.b.has(rf as u32) && hint_reg(wt as usize, &mut f.tmps) == rf {
                    let rt = rfree(cur, wt, target);
                    if rt != -1 {
                        f.tmps[wt as usize].visit = -1;
                        ralloc(cur, wt, target, regu, &mut f.tmps);
                        assert!(cur.b.has(rf as u32));
                        buf.emit(
                            Op::Copy,
                            f.tmps[wt as usize].cls,
                            Ref::Tmp(TmpId(rt as u32)),
                            Ref::Tmp(TmpId(rf as u32)),
                            Ref::R,
                        );
                        *stmov += 1;
                        cur.w[rf_u] = 0;

                        let last_idx = buf.len() - 1;
                        if last_idx > 0 {
                            let sl = buf.as_slice();
                            let mut patched = sl[last_idx - 1];
                            for a in 0..2 {
                                if patched.arg[a] == Ref::Tmp(TmpId(rt as u32)) {
                                    patched.arg[a] = Ref::Tmp(TmpId(rf as u32));
                                }
                            }
                            let mut new_buf = InsBuffer::new();
                            for &item in sl.iter().take(last_idx - 1) {
                                new_buf.emiti(item);
                            }
                            new_buf.emiti(patched);
                            new_buf.emiti(sl[last_idx]); // the copy
                            buf = new_buf;
                        }
                    }
                }
            }
        }
    }

    f.blks[b_idx].ins = buf.finish();
}

fn carve_order(f: &Fn) -> Vec<u32> {
    let mut blk_ids: Vec<u32> = f.rpo.iter().map(|b| b.0).collect();
    blk_ids.sort_by(|&a, &b| {
        let la = f.blks[a as usize].loop_depth;
        let lb = f.blks[b as usize].loop_depth;
        if la == lb {
            let ia = f.blks[a as usize].id;
            let ib = f.blks[b as usize].id;
            ib.cmp(&ia)
        } else {
            lb.cmp(&la)
        }
    });
    blk_ids
}

fn prio2(t1: i32, t2: i32, tmps: &mut [crate::ir::internal::Tmp]) -> std::cmp::Ordering {
    let v1 = tmps[t1 as usize].visit;
    let v2 = tmps[t2 as usize].visit;
    if (v1 ^ v2) < 0 {
        return if v1 != -1 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Less
        };
    }
    let h1 = hint_reg(t1 as usize, tmps);
    let h2 = hint_reg(t2 as usize, tmps);
    if (h1 ^ h2) < 0 {
        return if h1 != -1 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Less
        };
    }
    tmps[t1 as usize].cost.cmp(&tmps[t2 as usize].cost)
}

/// Assign physical registers and resolve parallel moves at CFG edges.
///
/// Requires RPO, spill costs, and spill information.
pub fn rega(f: &mut Fn, target: &Target) {
    let ntmp = f.tmps.len() as u32;
    let nblk = f.rpo.len();
    let mut regu: u64 = 0;
    let mut _stmov: u32 = 0;
    let mut pm = PMState::new();

    let mut end: Vec<RMap> = (0..nblk).map(|_| RMap::new(ntmp)).collect();
    let mut beg: Vec<RMap> = (0..nblk).map(|_| RMap::new(ntmp)).collect();
    let mut cur = RMap::new(ntmp);

    for t in 0..f.tmps.len() {
        f.tmps[t].hint.register = if t < TMP0 as usize {
            PhysicalRegister::from_index(t as i32)
        } else {
            None
        };
        f.tmps[t].hint.w = i32::MAX;
        f.tmps[t].visit = -1;
    }

    let start_idx = f.start.0 as usize;
    for ii in 0..f.blks[start_idx].ins.len() {
        let ins = f.blks[start_idx].ins[ii];
        if ins.op != Op::Copy || !crate::ir::internal::isreg(ins.arg[0]) {
            break;
        }
        if let Ref::Tmp(tid) = ins.to {
            sethint(
                tid.0 as usize,
                ins.arg[0].val() as i32,
                i32::MAX,
                &mut f.tmps,
            );
        }
    }

    let blk_order = carve_order(f);

    for &bid in &blk_order {
        let b_idx = bid as usize;
        let n = f.blks[b_idx].id as usize;
        let _loop_depth = f.blks[b_idx].loop_depth;

        cur.n = 0;
        cur.b.zero();
        cur.w = [0; RMAP_MAX];

        let mut rl: Vec<i32> = Vec::new();
        for t in f.blks[b_idx].out.iter() {
            if t >= TMP0 {
                let tv = t as i32;
                let mut j = rl.len();
                rl.push(tv);
                while j > 0 && prio2(tv, rl[j - 1], &mut f.tmps) == std::cmp::Ordering::Greater {
                    rl[j] = rl[j - 1];
                    rl[j - 1] = tv;
                    j -= 1;
                }
            }
        }

        for r in f.blks[b_idx].out.iter() {
            if r < TMP0 {
                radd(&mut cur, r as i32, r as i32, target, &mut regu);
            }
        }

        for &t in &rl {
            ralloctry(&mut cur, t, true, target, &mut regu, &mut f.tmps);
        }
        for &t in &rl {
            ralloc(&mut cur, t, target, &mut regu, &mut f.tmps);
        }

        end[n].copy_from(&cur);

        doblk(f, b_idx, &mut cur, target, &mut regu, &mut pm, &mut _stmov);

        f.blks[b_idx].r#in.copy_from(&cur.b);
        for pi in 0..f.blks[b_idx].phi.len() {
            if let Ref::Tmp(tid) = f.blks[b_idx].phi[pi].to {
                f.blks[b_idx].r#in.clr(tid.0);
            }
        }

        beg[n].copy_from(&cur);
    }

    let rpo = f.rpo.clone();
    for &sid in &rpo {
        let s_idx = sid.0 as usize;
        let npred = f.blks[s_idx].pred.len();
        if npred <= 1 {
            continue;
        }
        let s_id = f.blks[s_idx].id as usize;
        let m = &beg[s_id];

        let mut rl_map = vec![0i32; RMAP_MAX];

        let nphi = f.blks[s_idx].phi.len();
        for pi in 0..nphi {
            let phi_to = f.blks[s_idx].phi[pi].to;
            if let Ref::Tmp(tid) = phi_to {
                let r = rfind(m, tid.0 as i32);
                if r == -1 {
                    continue;
                }
                let narg = f.blks[s_idx].phi[pi].narg();
                for u in 0..narg {
                    let blk = f.blks[s_idx].phi[pi].blks[u];
                    let src = f.blks[s_idx].phi[pi].args[u];
                    if let Ref::Tmp(src_tid) = src {
                        let b_id = f.blks[blk.0 as usize].id as usize;
                        let x = rfind(&end[b_id], src_tid.0 as i32);
                        if x == -1 {
                            continue; // spilled
                        }
                        let cur_val = rl_map[r as usize];
                        rl_map[r as usize] = if cur_val == 0 || cur_val == x { x } else { -1 };
                    }
                }
                if rl_map[r as usize] == 0 {
                    rl_map[r as usize] = -1;
                }
            }
        }

        for j in 0..m.n {
            let t = m.t[j];
            let r = m.r[j];
            if rl_map[r as usize] != 0 || t < TMP0 as i32 {
                continue;
            }
            let preds = f.blks[s_idx].pred.clone();
            for &pred in &preds {
                let p_id = f.blks[pred.0 as usize].id as usize;
                let x = rfind(&end[p_id], t);
                if x == -1 {
                    continue; // spilled
                }
                let cur_val = rl_map[r as usize];
                rl_map[r as usize] = if cur_val == 0 || cur_val == x { x } else { -1 };
            }
            if rl_map[r as usize] == 0 {
                rl_map[r as usize] = -1;
            }
        }

        pm.clear();
        let m_n = beg[s_id].n;
        let m_t = beg[s_id].t;
        let m_r = beg[s_id].r;

        for j in 0..m_n {
            let t = m_t[j];
            let r = m_r[j];
            let x = rl_map[r as usize];
            assert!(x != 0 || t < TMP0 as i32);
            if x > 0 && !beg[s_id].b.has(x as u32) {
                pm.add(
                    Ref::Tmp(TmpId(x as u32)),
                    Ref::Tmp(TmpId(r as u32)),
                    f.tmps[t as usize].cls,
                );
                beg[s_id].r[j] = x;
                beg[s_id].b.set(x as u32);
            }
        }

        let mut buf = InsBuffer::new();
        pmgen(&pm, &mut buf);
        let new_ins = buf.finish();
        let j = new_ins.len();
        if j == 0 {
            continue;
        }
        let old_ins = f.blks[s_idx].ins.clone();
        let mut combined = new_ins;
        combined.extend(old_ins);
        f.blks[s_idx].ins = combined;
    }

    let mut blist: Vec<usize> = Vec::new(); // indices of new blocks

    let all_blks: Vec<u32> = {
        let mut v = Vec::new();
        for bid in &f.rpo {
            v.push(bid.0);
        }
        v
    };

    for &bid in &all_blks {
        let b_idx = bid as usize;
        let successors = [f.blks[b_idx].s1, f.blks[b_idx].s2];

        for (slot, s_opt) in successors.iter().enumerate() {
            let s_bid = match s_opt {
                Some(s) => *s,
                None => continue,
            };
            let s_idx = s_bid.0 as usize;
            let s_rpo = f.blks[s_idx].id as usize;
            let b_rpo = f.blks[b_idx].id as usize;

            pm.clear();

            let nphi = f.blks[s_idx].phi.len();
            for pi in 0..nphi {
                let dst = f.blks[s_idx].phi[pi].to;
                match dst {
                    Ref::Slot(_) | Ref::Tmp(_) => {}
                    _ => continue,
                }
                let dst_resolved = if let Ref::Tmp(tid) = dst {
                    let r = rfind(&beg[s_rpo], tid.0 as i32);
                    if r == -1 {
                        continue;
                    }
                    Ref::Tmp(TmpId(r as u32))
                } else {
                    dst
                };

                let narg = f.blks[s_idx].phi[pi].narg();
                let mut found_u = None;
                for u in 0..narg {
                    if f.blks[s_idx].phi[pi].blks[u].0 == bid {
                        found_u = Some(u);
                        break;
                    }
                }
                let u = match found_u {
                    Some(u) => u,
                    None => {
                        continue;
                    }
                };

                let src = f.blks[s_idx].phi[pi].args[u];
                let src_resolved = if let Ref::Tmp(tid) = src {
                    rref(&end[b_rpo], tid.0 as i32, &f.tmps)
                } else {
                    src
                };
                let cls = f.blks[s_idx].phi[pi].cls;
                pm.add(src_resolved, dst_resolved, cls);
            }

            for t in f.blks[s_idx].r#in.iter() {
                if t >= TMP0 {
                    let src = rref(&end[b_rpo], t as i32, &f.tmps);
                    let dst = rref(&beg[s_rpo], t as i32, &f.tmps);
                    pm.add(src, dst, f.tmps[t as usize].cls);
                }
            }

            let mut buf = InsBuffer::new();
            pmgen(&pm, &mut buf);
            let new_ins = buf.finish();
            if new_ins.is_empty() {
                continue;
            }

            let new_bid = f.blks.len();
            let mut b1 = crate::ir::internal::Blk {
                loop_depth: (f.blks[b_idx].loop_depth + f.blks[s_idx].loop_depth) / 2,
                name: format!("{}_{}", f.blks[b_idx].name, f.blks[s_idx].name),
                ins: new_ins,
                ..crate::ir::internal::Blk::default()
            };
            b1.jmp.typ = Jmp::Jmp_;
            b1.s1 = Some(s_bid);
            f.blks.push(b1);
            blist.push(new_bid);

            let new_blk_id = crate::ir::internal::BlkId(new_bid as u32);
            if slot == 0 {
                f.blks[b_idx].s1 = Some(new_blk_id);
            } else {
                f.blks[b_idx].s2 = Some(new_blk_id);
            }
        }
    }

    for bid in &f.rpo {
        f.blks[bid.0 as usize].phi.clear();
    }
    for &new_idx in &blist {
        f.blks[new_idx].phi.clear();
    }

    f.reg = regu;
}
