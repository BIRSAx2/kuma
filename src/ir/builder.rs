#![allow(dead_code)]

//! Shared IR helpers.

use std::collections::HashMap;

use crate::ir::internal::{
    Cls, Con, ConBits, ConId, ConType, Fn, Ins, N_CMP_I, Op, Ref, Sym, Tmp, TmpId,
};

/// Maps strings to stable integer IDs.
pub struct StringInterner {
    map: HashMap<String, u32>,
    strings: Vec<String>,
}

impl StringInterner {
    /// Create a new, empty interner.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            strings: Vec::new(),
        }
    }

    /// Return the existing ID for `s`, or assign one.
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.map.get(s) {
            return id;
        }
        let id = self.strings.len() as u32;
        self.strings.push(s.to_owned());
        self.map.insert(s.to_owned(), id);
        id
    }

    /// Return the string for `id`.
    ///
    /// # Panics
    /// Panics if the ID is out of range.
    pub fn get(&self, id: u32) -> &str {
        &self.strings[id as usize]
    }

    /// Number of interned strings.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Whether the interner is empty.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the symbol-table hash for `s`.
pub fn hash(s: &str) -> u32 {
    let mut h: u32 = 0;
    for &b in s.as_bytes() {
        h = (b as u32).wrapping_add(h.wrapping_mul(17));
    }
    h
}

/// Buffers instructions emitted in reverse program order.
pub struct InsBuffer {
    buf: Vec<Ins>,
}

impl InsBuffer {
    /// Create a new, empty instruction buffer.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Append an instruction to the reverse-order buffer.
    pub fn emit(&mut self, op: Op, cls: Cls, to: Ref, a0: Ref, a1: Ref) {
        self.buf.push(Ins {
            op,
            cls,
            to,
            arg: [a0, a1],
        });
    }

    /// Append an existing instruction.
    pub fn emiti(&mut self, ins: Ins) {
        self.buf.push(ins);
    }

    /// Return a copy in program order.
    pub fn finish(&self) -> Vec<Ins> {
        let mut out = self.buf.clone();
        out.reverse();
        out
    }

    /// Return the internal order; the last item was emitted most recently.
    pub fn as_slice(&self) -> &[Ins] {
        &self.buf
    }

    /// Number of buffered instructions.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Return the most recently emitted instruction.
    pub fn last_mut(&mut self) -> &mut Ins {
        self.buf.last_mut().expect("InsBuffer is empty")
    }

    /// Return the instruction at internal index `idx`.
    pub fn at_mut(&mut self, idx: usize) -> &mut Ins {
        &mut self.buf[idx]
    }
}

impl Default for InsBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Copy an instruction slice.
pub fn idup(src: &[Ins]) -> Vec<Ins> {
    src.to_vec()
}

/// Copy instructions from `src` into `dst`, appending them.
/// Returns the number of instructions copied.
pub fn icpy(dst: &mut Vec<Ins>, src: &[Ins]) -> usize {
    dst.extend_from_slice(src);
    src.len()
}

/// Add a temporary named `<prefix>.<counter>` and return its reference.
pub fn newtmp(prefix: &str, cls: Cls, f: &mut Fn) -> Ref {
    let t = f.tmps.len();
    let mut tmp = Tmp::default();
    if !prefix.is_empty() {
        tmp.name = format!("{}.{}", prefix, t);
    }
    tmp.cls = cls;
    tmp.slot = None;
    tmp.nuse = 1;
    tmp.ndef = 1;
    f.tmps.push(tmp);

    Ref::Tmp(TmpId(t as u32))
}

/// Adjust the use count of a temporary.
pub fn chuse(r: Ref, delta: i32, f: &mut Fn) {
    if let Ref::Tmp(id) = r {
        let t = &mut f.tmps[id.0 as usize];
        t.nuse = (t.nuse as i32 + delta) as u32;
    }
}

/// Return the path-compressed representative of a phi class.
pub fn phicls(t: usize, tmps: &mut [Tmp]) -> usize {
    let t1 = tmps[t].phi;
    if t1 == 0 {
        return t;
    }
    let t1 = t1 as usize;
    let result = phicls(t1, tmps);
    tmps[t].phi = result as i32;
    result
}

/// Check if two symbols refer to the same entity.
#[inline]
fn symeq(s0: &Sym, s1: &Sym) -> bool {
    s0.typ == s1.typ && s0.id == s1.id
}

/// Add or reuse a constant, excluding the undefined sentinel at index 0.
pub fn newcon(c: &Con, f: &mut Fn) -> Ref {
    for i in 1..f.cons.len() {
        let c1 = &f.cons[i];
        if c.typ == c1.typ && symeq(&c.sym, &c1.sym) && c.bits.i() == c1.bits.i() {
            return Ref::Con(ConId(i as u32));
        }
    }
    let i = f.cons.len();
    f.cons.push(*c);
    Ref::Con(ConId(i as u32))
}

/// Get or create an integer constant with the given value.
pub fn getcon(val: i64, f: &mut Fn) -> Ref {
    for c in 1..f.cons.len() {
        if f.cons[c].typ == ConType::Bits && f.cons[c].bits.i() == val {
            return Ref::Con(ConId(c as u32));
        }
    }
    let c = f.cons.len();
    f.cons.push(Con {
        typ: ConType::Bits,
        sym: Sym::default(),
        bits: ConBits::from_i64(val),
        flt: 0,
    });
    Ref::Con(ConId(c as u32))
}

/// Add `b` to `a`, returning false for two address constants.
pub fn addcon(a: &mut Con, b: &Con) -> bool {
    if a.typ == ConType::Undef {
        *a = *b;
        return true;
    }

    if b.typ == ConType::Addr {
        if a.typ == ConType::Addr {
            return false;
        }
        a.typ = ConType::Addr;
        a.sym = b.sym;
    }
    a.bits = ConBits::from_i64(a.bits.i().wrapping_add(b.bits.i()));
    true
}

/// Merge two value classes, returning true if they conflict.
pub fn clsmerge(dest: &mut Cls, src: Cls) -> bool {
    if src == Cls::Kx {
        return false;
    }
    if *dest == Cls::Kx {
        *dest = src;
        return false;
    }
    if (*dest == Cls::Kw && src == Cls::Kl) || (*dest == Cls::Kl && src == Cls::Kw) {
        *dest = Cls::Kw;
        return false;
    }
    *dest != src
}

/// Return whether a reference is a physical register.
#[inline]
pub fn isreg(r: Ref) -> bool {
    crate::ir::internal::isreg(r)
}

/// Return a comparison's kind and value class.
pub fn iscmp(op: Op) -> Option<(i32, i32)> {
    use crate::ir::internal::{CmpF, CmpI};

    let (kind, class) = match op {
        Op::Ceqw => (CmpI::Cieq as i32, Cls::Kw),
        Op::Cnew => (CmpI::Cine as i32, Cls::Kw),
        Op::Csgew => (CmpI::Cisge as i32, Cls::Kw),
        Op::Csgtw => (CmpI::Cisgt as i32, Cls::Kw),
        Op::Cslew => (CmpI::Cisle as i32, Cls::Kw),
        Op::Csltw => (CmpI::Cislt as i32, Cls::Kw),
        Op::Cugew => (CmpI::Ciuge as i32, Cls::Kw),
        Op::Cugtw => (CmpI::Ciugt as i32, Cls::Kw),
        Op::Culew => (CmpI::Ciule as i32, Cls::Kw),
        Op::Cultw => (CmpI::Ciult as i32, Cls::Kw),
        Op::Ceql => (CmpI::Cieq as i32, Cls::Kl),
        Op::Cnel => (CmpI::Cine as i32, Cls::Kl),
        Op::Csgel => (CmpI::Cisge as i32, Cls::Kl),
        Op::Csgtl => (CmpI::Cisgt as i32, Cls::Kl),
        Op::Cslel => (CmpI::Cisle as i32, Cls::Kl),
        Op::Csltl => (CmpI::Cislt as i32, Cls::Kl),
        Op::Cugel => (CmpI::Ciuge as i32, Cls::Kl),
        Op::Cugtl => (CmpI::Ciugt as i32, Cls::Kl),
        Op::Culel => (CmpI::Ciule as i32, Cls::Kl),
        Op::Cultl => (CmpI::Ciult as i32, Cls::Kl),
        Op::Ceqs => (N_CMP_I as i32 + CmpF::Cfeq as i32, Cls::Ks),
        Op::Cges => (N_CMP_I as i32 + CmpF::Cfge as i32, Cls::Ks),
        Op::Cgts => (N_CMP_I as i32 + CmpF::Cfgt as i32, Cls::Ks),
        Op::Cles => (N_CMP_I as i32 + CmpF::Cfle as i32, Cls::Ks),
        Op::Clts => (N_CMP_I as i32 + CmpF::Cflt as i32, Cls::Ks),
        Op::Cnes => (N_CMP_I as i32 + CmpF::Cfne as i32, Cls::Ks),
        Op::Cos => (N_CMP_I as i32 + CmpF::Cfo as i32, Cls::Ks),
        Op::Cuos => (N_CMP_I as i32 + CmpF::Cfuo as i32, Cls::Ks),
        Op::Ceqd => (N_CMP_I as i32 + CmpF::Cfeq as i32, Cls::Kd),
        Op::Cged => (N_CMP_I as i32 + CmpF::Cfge as i32, Cls::Kd),
        Op::Cgtd => (N_CMP_I as i32 + CmpF::Cfgt as i32, Cls::Kd),
        Op::Cled => (N_CMP_I as i32 + CmpF::Cfle as i32, Cls::Kd),
        Op::Cltd => (N_CMP_I as i32 + CmpF::Cflt as i32, Cls::Kd),
        Op::Cned => (N_CMP_I as i32 + CmpF::Cfne as i32, Cls::Kd),
        Op::Cod => (N_CMP_I as i32 + CmpF::Cfo as i32, Cls::Kd),
        Op::Cuod => (N_CMP_I as i32 + CmpF::Cfuo as i32, Cls::Kd),
        _ => return None,
    };
    Some((kind, class as i8 as i32))
}

/// Emit a 16-byte-aligned stack allocation.
pub fn salloc(to: Ref, sz: Ref, f: &mut Fn, buf: &mut InsBuffer) {
    f.dynalloc = true;

    if let Ref::Con(id) = sz {
        let raw_sz = f.cons[id.0 as usize].bits.i();
        if raw_sz < 0 || raw_sz >= (i32::MAX as i64) - 15 {
            panic!("invalid alloc size {}", raw_sz);
        }
        let aligned = (raw_sz + 15) & !15;
        let con_ref = getcon(aligned, f);
        buf.emit(Op::Salloc, Cls::Kl, to, con_ref, Ref::R);
    } else {
        let r0 = newtmp("isel", Cls::Kl, f);
        let r1 = newtmp("isel", Cls::Kl, f);
        let neg16 = getcon(-16, f);
        let fifteen = getcon(15, f);

        buf.emit(Op::Salloc, Cls::Kl, to, r0, Ref::R);
        buf.emit(Op::And, Cls::Kl, r0, r1, neg16);
        buf.emit(Op::Add, Cls::Kl, r1, sz, fifteen);

        if let Ref::Tmp(tid) = sz
            && f.tmps[tid.0 as usize].slot.is_some()
        {
            let to_name = if let Ref::Tmp(to_id) = to {
                f.tmps[to_id.0 as usize].name.clone()
            } else {
                "?".to_string()
            };
            panic!(
                "unlikely alloc argument %{} for %{}",
                f.tmps[tid.0 as usize].name, to_name,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::internal::{Cls, Con, ConBits, ConType, Op, Ref, Sym, TmpId};

    #[test]
    fn test_hash_basic() {
        assert_eq!(hash("hello"), hash("hello"));
        assert_ne!(hash("hello"), hash("world"));
        assert_eq!(hash(""), 0);
    }

    #[test]
    fn test_string_interner() {
        let mut si = StringInterner::new();
        let id_a = si.intern("alpha");
        let id_b = si.intern("beta");
        let id_a2 = si.intern("alpha");

        assert_eq!(id_a, id_a2);
        assert_ne!(id_a, id_b);
        assert_eq!(si.get(id_a), "alpha");
        assert_eq!(si.get(id_b), "beta");
        assert_eq!(si.len(), 2);
    }

    #[test]
    fn test_ins_buffer() {
        let mut buf = InsBuffer::new();
        buf.emit(Op::Add, Cls::Kw, Ref::R, Ref::R, Ref::R);
        buf.emit(Op::Sub, Cls::Kl, Ref::R, Ref::R, Ref::R);
        assert_eq!(buf.len(), 2);

        let ins = buf.finish();
        assert_eq!(ins[0].op, Op::Sub);
        assert_eq!(ins[1].op, Op::Add);
    }

    #[test]
    fn test_newtmp() {
        let mut f = Fn::default();
        f.cons.push(Con::default());

        let r = newtmp("test", Cls::Kw, &mut f);
        assert!(r.is_tmp());
        if let Ref::Tmp(id) = r {
            assert!(f.tmps[id.0 as usize].name.starts_with("test."));
            assert_eq!(f.tmps[id.0 as usize].cls, Cls::Kw);
            assert_eq!(f.tmps[id.0 as usize].slot, None);
        }
    }

    #[test]
    fn test_chuse() {
        let mut f = Fn::default();
        let tmp = Tmp {
            nuse: 5,
            ..Tmp::default()
        };
        f.tmps.push(tmp);

        let r = Ref::Tmp(TmpId(0));
        chuse(r, 3, &mut f);
        assert_eq!(f.tmps[0].nuse, 8);
        chuse(r, -2, &mut f);
        assert_eq!(f.tmps[0].nuse, 6);

        chuse(Ref::R, 10, &mut f);
        chuse(Ref::Int(42), 10, &mut f);
    }

    #[test]
    fn test_getcon() {
        let mut f = Fn::default();
        f.cons.push(Con::default());

        let r1 = getcon(42, &mut f);
        let r2 = getcon(42, &mut f);
        let r3 = getcon(99, &mut f);

        assert_eq!(r1, r2);
        assert_ne!(r1, r3);
        assert!(matches!(r1, Ref::Con(_)));
        assert_eq!(f.cons.len(), 3);
    }

    #[test]
    fn test_newcon() {
        let mut f = Fn::default();
        f.cons.push(Con::default()); // undef sentinel

        let c1 = Con {
            typ: ConType::Bits,
            sym: Sym::default(),
            bits: ConBits::from_i64(100),
            flt: 0,
        };
        let r1 = newcon(&c1, &mut f);
        let r2 = newcon(&c1, &mut f);
        assert_eq!(r1, r2);
        assert_eq!(f.cons.len(), 2); // sentinel + one constant
    }

    #[test]
    fn test_addcon() {
        let mut a = Con {
            typ: ConType::Undef,
            ..Con::default()
        };
        let b = Con {
            typ: ConType::Bits,
            bits: ConBits::from_i64(42),
            ..Con::default()
        };
        assert!(addcon(&mut a, &b));
        assert_eq!(a.typ, ConType::Bits);
        assert_eq!(a.bits.i(), 42);

        let c = Con {
            typ: ConType::Bits,
            bits: ConBits::from_i64(8),
            ..Con::default()
        };
        assert!(addcon(&mut a, &c));
        assert_eq!(a.bits.i(), 50);

        let mut d = Con {
            typ: ConType::Addr,
            bits: ConBits::from_i64(10),
            ..Con::default()
        };
        let e = Con {
            typ: ConType::Addr,
            bits: ConBits::from_i64(20),
            ..Con::default()
        };
        assert!(!addcon(&mut d, &e));
    }

    #[test]
    fn test_clsmerge() {
        let mut k = Cls::Kx;
        assert!(!clsmerge(&mut k, Cls::Kw));
        assert_eq!(k, Cls::Kw);

        assert!(!clsmerge(&mut k, Cls::Kw));
        assert_eq!(k, Cls::Kw);

        let mut kl = Cls::Kl;
        assert!(!clsmerge(&mut kl, Cls::Kw));
        assert_eq!(kl, Cls::Kw);

        let mut k2 = Cls::Ks;
        assert!(!clsmerge(&mut k2, Cls::Kx));
        assert_eq!(k2, Cls::Ks);

        let mut k3 = Cls::Kw;
        assert!(clsmerge(&mut k3, Cls::Ks));
    }

    #[test]
    fn test_iscmp() {
        let r = iscmp(Op::Ceqw);
        assert!(r.is_some());
        let (kind, cls) = r.unwrap();
        assert_eq!(kind, 0); // Cieq
        assert_eq!(cls, Cls::Kw as i8 as i32);

        let r = iscmp(Op::Ceql);
        assert!(r.is_some());
        let (kind, cls) = r.unwrap();
        assert_eq!(kind, 0); // Cieq
        assert_eq!(cls, Cls::Kl as i8 as i32);

        assert!(iscmp(Op::Add).is_none());
    }

    #[test]
    fn test_phicls() {
        let mut tmps = vec![Tmp::default(); 5];
        tmps[4].phi = 3;
        tmps[3].phi = 2;

        let rep = phicls(4, &mut tmps);
        assert_eq!(rep, 2);
        assert_eq!(tmps[4].phi, 2);
        assert_eq!(tmps[3].phi, 2);
    }

    #[test]
    fn test_salloc_constant() {
        let mut f = Fn::default();
        f.cons.push(Con::default()); // undef sentinel

        let sz_ref = getcon(100, &mut f);
        let to = newtmp("out", Cls::Kl, &mut f);

        let mut buf = InsBuffer::new();
        salloc(to, sz_ref, &mut f, &mut buf);

        assert!(f.dynalloc);
        let ins = buf.finish();
        assert_eq!(ins.len(), 1);
        assert_eq!(ins[0].op, Op::Salloc);
        if let Ref::Con(id) = ins[0].arg[0] {
            assert_eq!(f.cons[id.0 as usize].bits.i(), 112);
        } else {
            panic!("expected Con ref for aligned size");
        }
    }

    #[test]
    fn test_idup() {
        let ins = vec![
            Ins {
                op: Op::Add,
                cls: Cls::Kw,
                to: Ref::R,
                arg: [Ref::R, Ref::R],
            },
            Ins {
                op: Op::Sub,
                cls: Cls::Kl,
                to: Ref::R,
                arg: [Ref::R, Ref::R],
            },
        ];
        let dup = idup(&ins);
        assert_eq!(dup.len(), 2);
        assert_eq!(dup[0].op, Op::Add);
        assert_eq!(dup[1].op, Op::Sub);
    }
}
