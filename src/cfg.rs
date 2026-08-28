//! Control-flow graph analysis and simplification.

use crate::ir::{Blk, BlkId, Fn, Jmp};

/// Return RPO blocks, or every stored block before RPO is available.
fn all_blk_ids(f: &Fn) -> Vec<BlkId> {
    if f.rpo.is_empty() {
        (0..f.blks.len()).map(|i| BlkId(i as u32)).collect()
    } else {
        f.rpo.clone()
    }
}

/// Create a new, empty basic block.
pub fn newblk() -> Blk {
    Blk::default()
}

/// Remove the edge from block `bs` to its successor number `which` (0 → s1,
/// 1 → s2), including phi arguments and predecessor lists.
pub fn edgedel(f: &mut Fn, bs: BlkId, which: usize) {
    let b = &f.blks[bs.0 as usize];
    let s1 = b.s1;
    let s2 = b.s2;

    let bd = match which {
        0 => s1,
        1 => s2,
        _ => panic!("edgedel: which must be 0 or 1"),
    };

    let mult = if s1.is_some() && s1 == s2 { 2 } else { 1 };

    match which {
        0 => f.blks[bs.0 as usize].s1 = None,
        1 => f.blks[bs.0 as usize].s2 = None,
        _ => unreachable!(),
    }

    let bd = match bd {
        Some(id) => id,
        None => return,
    };

    if mult > 1 {
        return;
    }

    let phis = &mut f.blks[bd.0 as usize].phi;
    for phi in phis.iter_mut() {
        if let Some(a) = phi.blks.iter().position(|&blk| blk == bs) {
            phi.blks.remove(a);
            phi.args.remove(a);
        }
    }

    let preds = &mut f.blks[bd.0 as usize].pred;
    if let Some(a) = preds.iter().position(|&p| p == bs) {
        preds.remove(a);
    }
}

/// Fill predecessor lists for all blocks.
pub fn fillpreds(f: &mut Fn) {
    let ids = all_blk_ids(f);

    for &bid in &ids {
        f.blks[bid.0 as usize].pred.clear();
    }

    for &bid in &ids {
        let s1 = f.blks[bid.0 as usize].s1;
        let s2 = f.blks[bid.0 as usize].s2;
        if let Some(s) = s1 {
            f.blks[s.0 as usize].pred.push(bid);
        }
        if let Some(s) = s2
            && s1 != Some(s)
        {
            f.blks[s.0 as usize].pred.push(bid);
        }
    }
}

/// Visit reachable blocks and return the next descending RPO number.
fn rporec(f: &mut Fn, b: BlkId, mut x: u32) -> u32 {
    if f.blks[b.0 as usize].id != u32::MAX {
        return x; // already visited
    }
    f.blks[b.0 as usize].id = 1; // mark in-progress

    let mut first = f.blks[b.0 as usize].s1;
    let mut second = f.blks[b.0 as usize].s2;
    if let (Some(s1), Some(s2)) = (first, second)
        && f.blks[s1.0 as usize].loop_depth > f.blks[s2.0 as usize].loop_depth
    {
        first = Some(s2);
        second = Some(s1);
    }

    if let Some(s) = first {
        x = rporec(f, s, x);
    }
    if let Some(s) = second {
        x = rporec(f, s, x);
    }

    f.blks[b.0 as usize].id = x;
    assert!(x != u32::MAX);
    x.wrapping_sub(1)
}

/// Compute reverse post-order numbering.  Populates `f.rpo` and sets each
/// block's `id` to its RPO index.  Unreachable blocks are removed from
/// `f.rpo` (and have their edges deleted).
pub fn fillrpo(f: &mut Fn) {
    let nblk = f.blks.len() as u32;
    if nblk == 0 {
        return;
    }

    for blk in f.blks.iter_mut() {
        blk.id = u32::MAX;
    }

    let n_start = rporec(f, f.start, nblk - 1);
    let n = n_start.wrapping_add(1); // offset to subtract

    let unreachable: Vec<BlkId> = (0..nblk)
        .filter(|&i| f.blks[i as usize].id == u32::MAX)
        .map(BlkId)
        .collect();
    for uid in unreachable {
        if f.blks[uid.0 as usize].s1.is_some() {
            edgedel(f, uid, 0);
        }
        if f.blks[uid.0 as usize].s2.is_some() {
            edgedel(f, uid, 1);
        }
    }

    let reachable_count = nblk.wrapping_sub(n) as usize;
    let mut rpo = vec![BlkId::NONE; reachable_count];
    for i in 0..nblk {
        let blk = &mut f.blks[i as usize];
        if blk.id != u32::MAX {
            blk.id = blk.id.wrapping_sub(n);
            rpo[blk.id as usize] = BlkId(i);
        }
    }
    f.rpo = rpo;
}

/// Intersect two blocks in the dominator tree, walking up by RPO id.
fn inter(f: &Fn, mut b1: BlkId, mut b2: BlkId) -> BlkId {
    loop {
        if b1 == b2 {
            return b1;
        }
        let id1 = f.blks[b1.0 as usize].id;
        let id2 = f.blks[b2.0 as usize].id;
        if id1 < id2 {
            std::mem::swap(&mut b1, &mut b2);
        }
        while f.blks[b1.0 as usize].id > f.blks[b2.0 as usize].id {
            b1 = f.blks[b1.0 as usize].idom.expect("inter: idom must exist");
        }
    }
}

/// Compute the dominator tree.  Sets `idom`, `dom`, `dlink` on each block.
pub fn filldom(f: &mut Fn) {
    let ids = all_blk_ids(f);

    for &bid in &ids {
        let blk = &mut f.blks[bid.0 as usize];
        blk.idom = None;
        blk.dom = None;
        blk.dlink = None;
    }

    loop {
        let mut changed = 0u32;
        for idx in 1..f.rpo.len() {
            let b = f.rpo[idx];
            let preds: Vec<BlkId> = f.blks[b.0 as usize].pred.clone();
            let mut d: Option<BlkId> = None;
            for &p in &preds {
                if f.blks[p.0 as usize].idom.is_some() || p == f.start {
                    d = Some(match d {
                        None => p,
                        Some(cur) => inter(f, cur, p),
                    });
                }
            }
            if d != f.blks[b.0 as usize].idom {
                changed += 1;
                f.blks[b.0 as usize].idom = d;
            }
        }
        if changed == 0 {
            break;
        }
    }

    for &bid in &ids {
        let d = f.blks[bid.0 as usize].idom;
        if let Some(d) = d {
            assert!(d != bid);
            let old_child = f.blks[d.0 as usize].dom;
            f.blks[bid.0 as usize].dlink = old_child;
            f.blks[d.0 as usize].dom = Some(bid);
        }
    }
}

/// Returns true if `a` strictly dominates `b`.
pub fn sdom(f: &Fn, a: BlkId, b: BlkId) -> bool {
    if a == b {
        return false;
    }
    let mut cur = b;
    while f.blks[cur.0 as usize].id > f.blks[a.0 as usize].id {
        cur = match f.blks[cur.0 as usize].idom {
            Some(id) => id,
            None => return false,
        };
    }
    a == cur
}

/// Returns true if `a` dominates `b` (a == b counts).
pub fn dom(f: &Fn, a: BlkId, b: BlkId) -> bool {
    a == b || sdom(f, a, b)
}

/// Compute dominance frontiers.  Sets `fron` on each block.
pub fn fillfron(f: &mut Fn) {
    let ids = all_blk_ids(f);

    for &bid in &ids {
        f.blks[bid.0 as usize].fron.clear();
    }

    for &bid in &ids {
        let s1 = f.blks[bid.0 as usize].s1;
        let s2 = f.blks[bid.0 as usize].s2;

        for s in [s1, s2].into_iter().flatten() {
            let mut a = bid;
            loop {
                if sdom(f, a, s) {
                    break;
                }
                if !f.blks[a.0 as usize].fron.contains(&s) {
                    f.blks[a.0 as usize].fron.push(s);
                }
                a = match f.blks[a.0 as usize].idom {
                    Some(id) => id,
                    None => break,
                };
            }
        }
    }
}

/// Recursively mark blocks belonging to a natural loop with header `hd`,
/// starting from block `b` and walking predecessors.
fn loopmark(f: &mut Fn, hd: BlkId, b: BlkId, cb: &mut dyn FnMut(BlkId, BlkId)) {
    let hd_id = f.blks[hd.0 as usize].id;
    let b_id = f.blks[b.0 as usize].id;
    if b_id < hd_id {
        return;
    }
    if f.blks[b.0 as usize].visit == hd_id {
        return;
    }
    f.blks[b.0 as usize].visit = hd_id;
    cb(hd, b);
    let preds: Vec<BlkId> = f.blks[b.0 as usize].pred.clone();
    for p in preds {
        loopmark(f, hd, p, cb);
    }
}

/// Iterate over back-edges in the CFG and invoke `cb(header, body_block)`
/// for every block in each natural loop.
pub fn loopiter(f: &mut Fn, cb: &mut dyn FnMut(BlkId, BlkId)) {
    let ids = all_blk_ids(f);

    for &bid in &ids {
        f.blks[bid.0 as usize].visit = u32::MAX;
    }

    for idx in 0..f.rpo.len() {
        let b = f.rpo[idx];
        let preds: Vec<BlkId> = f.blks[b.0 as usize].pred.clone();
        for p in preds {
            if f.blks[p.0 as usize].id >= idx as u32 {
                loopmark(f, b, p, cb);
            }
        }
    }
}

/// Compute loop nesting depth for each block.  Each level of nesting
/// multiplies `loop_depth` by 10.
pub fn fillloop(f: &mut Fn) {
    let ids = all_blk_ids(f);

    for &bid in &ids {
        f.blks[bid.0 as usize].loop_depth = 1;
    }

    let mut pairs: Vec<BlkId> = Vec::new();
    loopiter(f, &mut |_hd, b| {
        pairs.push(b);
    });
    for b in pairs {
        f.blks[b.0 as usize].loop_depth *= 10;
    }
}

/// Path-compressed union-find lookup.
fn uffind(bid: BlkId, uf: &mut Vec<Option<BlkId>>) -> BlkId {
    match uf[bid.0 as usize] {
        None => bid,
        Some(parent) => {
            let root = uffind(parent, uf);
            uf[bid.0 as usize] = Some(root);
            root
        }
    }
}

/// Simplify jump chains and introduce a unified return block.
///
/// Requires RPO and resolved phi nodes.
pub fn simpljmp(f: &mut Fn) {
    let mut ret = newblk();
    ret.jmp.typ = Jmp::Ret0;
    let ret_id = BlkId(f.blks.len() as u32);
    ret.id = ret_id.0;
    f.blks.push(ret);

    let nblk = f.blks.len();
    let mut uf: Vec<Option<BlkId>> = vec![None; nblk];

    for i in 0..nblk {
        let bid = BlkId(i as u32);
        if bid == ret_id {
            continue;
        }
        let blk = &f.blks[i];
        assert!(blk.phi.is_empty(), "simpljmp: block has phi nodes");

        if blk.jmp.typ == Jmp::Ret0 {
            f.blks[i].jmp.typ = Jmp::Jmp_;
            f.blks[i].s1 = Some(ret_id);
        }

        let blk = &f.blks[i];
        if blk.ins.is_empty()
            && blk.jmp.typ == Jmp::Jmp_
            && let Some(s1) = blk.s1
        {
            let target = uffind(s1, &mut uf);
            if target != bid {
                uf[i] = Some(target);
            }
        }
    }

    for i in 0..nblk {
        if let Some(s1) = f.blks[i].s1 {
            let resolved = uffind(s1, &mut uf);
            f.blks[i].s1 = Some(resolved);
        }
        if let Some(s2) = f.blks[i].s2 {
            let resolved = uffind(s2, &mut uf);
            f.blks[i].s2 = Some(resolved);
        }
        if f.blks[i].s1.is_some() && f.blks[i].s1 == f.blks[i].s2 {
            f.blks[i].jmp.typ = Jmp::Jmp_;
            f.blks[i].s2 = None;
        }
    }
}
