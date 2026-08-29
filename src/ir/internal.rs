#![allow(dead_code)]
#![allow(clippy::enum_variant_names)]

//! Kuma's intermediate representation.

use std::fmt;

pub const N_STRING: usize = 80;
pub const N_INS: usize = 1 << 20;
pub const N_ALIGN: usize = 3;
pub const N_FIELD: usize = 32;
pub const N_BIT: u32 = 64;

/// First non-register temporary index.
pub const TMP0: u32 = N_BIT;

macro_rules! define_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
        #[repr(transparent)]
        pub struct $name(pub u32);

        impl $name {
            pub const NONE: Self = Self(u32::MAX);

            #[inline]
            pub fn index(self) -> u32 {
                self.0
            }

            #[inline]
            pub fn is_none(self) -> bool {
                self == Self::NONE
            }
        }

        impl From<u32> for $name {
            #[inline]
            fn from(v: u32) -> Self {
                Self(v)
            }
        }

        impl From<usize> for $name {
            #[inline]
            fn from(v: usize) -> Self {
                Self(v as u32)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(BlkId, "Block index.");
define_id!(TmpId, "Temporary index.");
define_id!(ConId, "Constant index.");
define_id!(MemId, "Memory reference index.");
define_id!(TypId, "Type index.");

/// Encoded stack-slot location used after lowering.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct StackSlot(u32);

impl StackSlot {
    const MASK: u32 = 0x1fff_ffff;

    pub fn from_signed(value: i32) -> Self {
        Self(value as u32 & Self::MASK)
    }

    pub fn signed(self) -> i32 {
        (((self.0 as i64) << 3) as i32) >> 3
    }

    pub fn encoded(self) -> u32 {
        self.0
    }
}

/// A physical register chosen by a backend.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct PhysicalRegister(u32);

impl PhysicalRegister {
    pub fn from_index(index: i32) -> Option<Self> {
        (index >= 0).then_some(Self(index as u32))
    }

    pub fn index(self) -> i32 {
        self.0 as i32
    }
}

/// Bit set of physical registers.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct RegisterSet(u64);

impl RegisterSet {
    pub fn insert_all(&mut self, bits: u64) {
        self.0 |= bits;
    }

    pub fn bits(self) -> u64 {
        self.0
    }
}

/// An IR value reference.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Default)]
pub enum Ref {
    /// A temporary (register or virtual).
    Tmp(TmpId),
    /// A constant pool entry.
    Con(ConId),
    /// An immediate integer (29-bit, sign-extended).
    Int(i32),
    /// A type reference (from parser).
    Typ(TypId),
    /// A stack slot.
    Slot(StackSlot),
    /// A call site.
    Call(u32),
    /// A memory addressing mode.
    Mem(MemId),
    /// No reference.
    #[default]
    R,
}

impl Ref {
    /// Undefined constant: `Con(0)`.
    pub const UNDEF: Ref = Ref::Con(ConId(0));
    /// Zero constant: `Con(1)`.
    pub const CON_Z: Ref = Ref::Con(ConId(1));

    #[inline]
    pub fn is_tmp(self) -> bool {
        matches!(self, Ref::Tmp(_))
    }

    #[inline]
    pub fn is_none(self) -> bool {
        matches!(self, Ref::R)
    }

    /// Return the reference discriminant, or -1 for `R`.
    #[inline]
    pub fn rtype(self) -> i32 {
        match self {
            Ref::R => -1,
            Ref::Tmp(_) => 0,  // RTmp
            Ref::Con(_) => 1,  // RCon
            Ref::Int(_) => 2,  // RInt
            Ref::Typ(_) => 3,  // RType
            Ref::Slot(_) => 4, // RSlot
            Ref::Call(_) => 5, // RCall
            Ref::Mem(_) => 6,  // RMem
        }
    }

    /// Return the encoded value.
    #[inline]
    pub fn val(self) -> u32 {
        match self {
            Ref::R => 0,
            Ref::Tmp(id) => id.0,
            Ref::Con(id) => id.0,
            Ref::Int(v) => (v as u32) & 0x1fff_ffff,
            Ref::Typ(id) => id.0,
            Ref::Slot(slot) => slot.encoded(),
            Ref::Call(v) => v,
            Ref::Mem(id) => id.0,
        }
    }

    /// Sign-extend the 29-bit encoded value.
    #[inline]
    pub fn sval(self) -> i32 {
        let v = self.val();
        (((v as i64) << 3) as i32) >> 3
    }
}

impl fmt::Debug for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ref::Tmp(id) => write!(f, "Tmp({})", id.0),
            Ref::Con(id) => write!(f, "Con({})", id.0),
            Ref::Int(v) => write!(f, "Int({})", v),
            Ref::Typ(id) => write!(f, "Typ({})", id.0),
            Ref::Slot(slot) => write!(f, "Slot({})", slot.signed()),
            Ref::Call(v) => write!(f, "Call({})", v),
            Ref::Mem(id) => write!(f, "Mem({})", id.0),
            Ref::R => write!(f, "R"),
        }
    }
}

/// IR value class.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(i8)]
#[derive(Default)]
pub enum Cls {
    /// Unset class.
    #[default]
    Kx = -1,
    /// Word: i32.
    Kw = 0,
    /// Long: i64.
    Kl = 1,
    /// Single: f32.
    Ks = 2,
    /// Double: f64.
    Kd = 3,
}

impl Cls {
    /// Is this a wide type? (`Kl` or `Kd`, i.e. bit 0 set.)
    #[inline]
    pub fn is_wide(self) -> bool {
        matches!(self, Cls::Kl | Cls::Kd)
    }

    /// Is this a floating-point type? (`Ks` or `Kd`.)
    #[inline]
    pub fn is_float(self) -> bool {
        matches!(self, Cls::Ks | Cls::Kd)
    }

    /// Base class: 0 for integer (`Kw`/`Kl`), 1 for float (`Ks`/`Kd`).
    /// Returns -1 for `Kx`.
    #[inline]
    pub fn base(self) -> i32 {
        match self {
            Cls::Kx => -1,
            Cls::Kw | Cls::Kl => 0,
            Cls::Ks | Cls::Kd => 1,
        }
    }

    /// Convert an encoded class value.
    #[inline]
    pub fn from_i8(v: i8) -> Self {
        match v {
            0 => Cls::Kw,
            1 => Cls::Kl,
            2 => Cls::Ks,
            3 => Cls::Kd,
            _ => Cls::Kx,
        }
    }

    /// Convert to u8 index for array indexing (Kw=0, Kl=1, Ks=2, Kd=3).
    /// Panics on Kx.
    #[inline]
    pub fn as_index(self) -> usize {
        debug_assert!(self != Cls::Kx, "cannot index with Kx");
        self as i8 as usize
    }
}

/// Integer comparison kinds.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u8)]
pub enum CmpI {
    Cieq = 0,
    Cine = 1,
    Cisge = 2,
    Cisgt = 3,
    Cisle = 4,
    Cislt = 5,
    Ciuge = 6,
    Ciugt = 7,
    Ciule = 8,
    Ciult = 9,
}

pub const N_CMP_I: usize = 10;

/// Floating-point comparison kinds.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u8)]
pub enum CmpF {
    Cfeq = 0,
    Cfge = 1,
    Cfgt = 2,
    Cfle = 3,
    Cflt = 4,
    Cfne = 5,
    Cfo = 6,
    Cfuo = 7,
}

pub const N_CMP_F: usize = 8;
pub const N_CMP: usize = N_CMP_I + N_CMP_F;

/// IR opcodes in metadata-table order.
///
/// `Oxxx` is the sentinel / invalid opcode (value 0). `NOp` is the count.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u16)]
pub enum Op {
    Oxxx = 0,

    Add,
    Sub,
    Neg,
    Div,
    Rem,
    Udiv,
    Urem,
    Mul,
    And,
    Or,
    Xor,
    Sar,
    Shr,
    Shl,

    Ceqw,
    Cnew,
    Csgew,
    Csgtw,
    Cslew,
    Csltw,
    Cugew,
    Cugtw,
    Culew,
    Cultw,

    Ceql,
    Cnel,
    Csgel,
    Csgtl,
    Cslel,
    Csltl,
    Cugel,
    Cugtl,
    Culel,
    Cultl,

    Ceqs,
    Cges,
    Cgts,
    Cles,
    Clts,
    Cnes,
    Cos,
    Cuos,

    Ceqd,
    Cged,
    Cgtd,
    Cled,
    Cltd,
    Cned,
    Cod,
    Cuod,

    Storeb,
    Storeh,
    Storew,
    Storel,
    Stores,
    Stored,

    Loadsb,
    Loadub,
    Loadsh,
    Loaduh,
    Loadsw,
    Loaduw,
    Load,

    Extsb,
    Extub,
    Extsh,
    Extuh,
    Extsw,
    Extuw,

    Exts,
    Truncd,
    Stosi,
    Stoui,
    Dtosi,
    Dtoui,
    Swtof,
    Uwtof,
    Sltof,
    Ultof,
    Cast,

    Alloc4,
    Alloc8,
    Alloc16,

    Vaarg,
    Vastart,

    Copy,
    Dbgloc,

    Nop,
    Addr,
    Blit0,
    Blit1,
    Swap,
    Sign,
    Salloc,
    Xidiv,
    Xdiv,
    Xcmp,
    Xtest,
    Acmp,
    Acmn,
    Afcmp,
    Reqz,
    Rnez,

    Par,
    Parsb,
    Parub,
    Parsh,
    Paruh,
    Parc,
    Pare,

    Arg,
    Argsb,
    Argub,
    Argsh,
    Arguh,
    Argc,
    Arge,
    Argv,
    Call,

    Flagieq,
    Flagine,
    Flagisge,
    Flagisgt,
    Flagisle,
    Flagislt,
    Flagiuge,
    Flagiugt,
    Flagiule,
    Flagiult,
    Flagfeq,
    Flagfge,
    Flagfgt,
    Flagfle,
    Flagflt,
    Flagfne,
    Flagfo,
    Flagfuo,
}

/// Total number of opcodes (sentinel excluded from count but included in table).
pub const N_OP: usize = Op::Flagfuo as usize + 1;

impl Op {
    #[inline]
    pub fn is_store(self) -> bool {
        matches!(
            self,
            Self::Storeb | Self::Storeh | Self::Storew | Self::Storel | Self::Stores | Self::Stored
        )
    }

    #[inline]
    pub fn is_load(self) -> bool {
        matches!(
            self,
            Self::Loadsb
                | Self::Loadub
                | Self::Loadsh
                | Self::Loaduh
                | Self::Loadsw
                | Self::Loaduw
                | Self::Load
        )
    }

    #[inline]
    pub fn is_ext(self) -> bool {
        matches!(
            self,
            Self::Extsb | Self::Extub | Self::Extsh | Self::Extuh | Self::Extsw | Self::Extuw
        )
    }

    #[inline]
    pub fn is_par(self) -> bool {
        matches!(
            self,
            Self::Par
                | Self::Parsb
                | Self::Parub
                | Self::Parsh
                | Self::Paruh
                | Self::Parc
                | Self::Pare
        )
    }

    #[inline]
    pub fn is_arg(self) -> bool {
        matches!(
            self,
            Self::Arg
                | Self::Argsb
                | Self::Argub
                | Self::Argsh
                | Self::Arguh
                | Self::Argc
                | Self::Arge
                | Self::Argv
        )
    }

    #[inline]
    pub fn is_parbh(self) -> bool {
        matches!(self, Self::Parsb | Self::Parub | Self::Parsh | Self::Paruh)
    }

    #[inline]
    pub fn is_argbh(self) -> bool {
        matches!(self, Self::Argsb | Self::Argub | Self::Argsh | Self::Arguh)
    }

    #[inline]
    pub fn is_cmp(self) -> bool {
        matches!(
            self,
            Self::Ceqw
                | Self::Cnew
                | Self::Csgew
                | Self::Csgtw
                | Self::Cslew
                | Self::Csltw
                | Self::Cugew
                | Self::Cugtw
                | Self::Culew
                | Self::Cultw
                | Self::Ceql
                | Self::Cnel
                | Self::Csgel
                | Self::Csgtl
                | Self::Cslel
                | Self::Csltl
                | Self::Cugel
                | Self::Cugtl
                | Self::Culel
                | Self::Cultl
                | Self::Ceqs
                | Self::Cges
                | Self::Cgts
                | Self::Cles
                | Self::Clts
                | Self::Cnes
                | Self::Cos
                | Self::Cuos
                | Self::Ceqd
                | Self::Cged
                | Self::Cgtd
                | Self::Cled
                | Self::Cltd
                | Self::Cned
                | Self::Cod
                | Self::Cuod
        )
    }

    #[inline]
    pub fn is_flag(self) -> bool {
        matches!(
            self,
            Self::Flagieq
                | Self::Flagine
                | Self::Flagisge
                | Self::Flagisgt
                | Self::Flagisle
                | Self::Flagislt
                | Self::Flagiuge
                | Self::Flagiugt
                | Self::Flagiule
                | Self::Flagiult
                | Self::Flagfeq
                | Self::Flagfge
                | Self::Flagfgt
                | Self::Flagfle
                | Self::Flagflt
                | Self::Flagfne
                | Self::Flagfo
                | Self::Flagfuo
        )
    }

    #[inline]
    pub fn is_alloc(self) -> bool {
        matches!(self, Self::Alloc4 | Self::Alloc8 | Self::Alloc16)
    }

    /// Whether this operation can be constant-folded.
    #[inline]
    pub fn can_fold(self) -> bool {
        let i = self as usize;
        if i < N_OP {
            OP_TABLE[i].can_fold
        } else {
            false
        }
    }
}

/// Jump and terminator kinds.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u16)]
#[derive(Default)]
pub enum Jmp {
    #[default]
    Jxxx = 0,
    Retw,
    Retl,
    Rets,
    Retd,
    Retsb,
    Retub,
    Retsh,
    Retuh,
    Retc,
    Ret0,
    Jmp_,
    Jnz,
    Jfieq,
    Jfine,
    Jfisge,
    Jfisgt,
    Jfisle,
    Jfislt,
    Jfiuge,
    Jfiugt,
    Jfiule,
    Jfiult,
    Jffeq,
    Jffge,
    Jffgt,
    Jffle,
    Jfflt,
    Jffne,
    Jffo,
    Jffuo,
    Hlt,
}

impl Jmp {
    #[inline]
    pub fn is_ret(self) -> bool {
        matches!(
            self,
            Self::Retw
                | Self::Retl
                | Self::Rets
                | Self::Retd
                | Self::Retsb
                | Self::Retub
                | Self::Retsh
                | Self::Retuh
                | Self::Retc
                | Self::Ret0
        )
    }

    /// Scalar class returned by this terminator, if any.
    pub fn return_class(self) -> Option<Cls> {
        match self {
            Self::Retw | Self::Retsb | Self::Retub | Self::Retsh | Self::Retuh => Some(Cls::Kw),
            Self::Retl => Some(Cls::Kl),
            Self::Rets => Some(Cls::Ks),
            Self::Retd => Some(Cls::Kd),
            _ => None,
        }
    }

    /// Backend condition-table index for a conditional flag jump.
    pub fn comparison_index(self) -> Option<u16> {
        match self {
            Self::Jfieq => Some(0),
            Self::Jfine => Some(1),
            Self::Jfisge => Some(2),
            Self::Jfisgt => Some(3),
            Self::Jfisle => Some(4),
            Self::Jfislt => Some(5),
            Self::Jfiuge => Some(6),
            Self::Jfiugt => Some(7),
            Self::Jfiule => Some(8),
            Self::Jfiult => Some(9),
            Self::Jffeq => Some(10),
            Self::Jffge => Some(11),
            Self::Jffgt => Some(12),
            Self::Jffle => Some(13),
            Self::Jfflt => Some(14),
            Self::Jffne => Some(15),
            Self::Jffo => Some(16),
            Self::Jffuo => Some(17),
            _ => None,
        }
    }

    #[inline]
    pub fn is_retbh(self) -> bool {
        matches!(self, Self::Retsb | Self::Retub | Self::Retsh | Self::Retuh)
    }

    #[inline]
    pub fn is_jmp(self) -> bool {
        self == Jmp::Jmp_
    }

    #[inline]
    pub fn is_jnz(self) -> bool {
        self == Jmp::Jnz
    }

    #[inline]
    pub fn is_flag_jmp(self) -> bool {
        self.comparison_index().is_some()
    }
}

/// Block jump/terminator info. Combines the jump type with its argument.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub struct JmpInfo {
    pub typ: Jmp,
    pub arg: Ref,
}

/// A single IR instruction.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Ins {
    pub op: Op,
    pub cls: Cls,
    pub to: Ref,
    pub arg: [Ref; 2],
}

impl Default for Ins {
    fn default() -> Self {
        Self {
            op: Op::Nop,
            cls: Cls::Kx,
            to: Ref::R,
            arg: [Ref::R, Ref::R],
        }
    }
}

/// A phi node.
#[derive(Clone, Debug)]
pub struct Phi {
    pub to: Ref,
    pub cls: Cls,
    pub args: Vec<Ref>,
    pub blks: Vec<BlkId>,
}

impl Phi {
    pub fn narg(&self) -> usize {
        debug_assert_eq!(self.args.len(), self.blks.len());
        self.args.len()
    }
}

/// A growable bitset.
#[derive(Clone, Debug, Default)]
pub struct BSet {
    /// Backing storage: one bit per index.
    bits: Vec<u64>,
    /// Number of u64 words.
    nt: u32,
}

impl BSet {
    /// Create a bitset that can hold at least `n` bits.
    pub fn new(n: u32) -> Self {
        let nt = n.div_ceil(64);
        Self {
            bits: vec![0u64; nt as usize],
            nt,
        }
    }

    /// Reset all bits to zero.
    pub fn zero(&mut self) {
        for w in self.bits.iter_mut() {
            *w = 0;
        }
    }

    /// Population count.
    pub fn count(&self) -> u32 {
        self.bits.iter().map(|w| w.count_ones()).sum()
    }

    /// Set bit `b`.
    #[inline]
    pub fn set(&mut self, b: u32) {
        debug_assert!(
            (b / 64) < self.nt,
            "bit {} out of range (nt={})",
            b,
            self.nt
        );
        self.bits[(b / 64) as usize] |= 1u64 << (b % 64);
    }

    /// Clear bit `b`.
    #[inline]
    pub fn clr(&mut self, b: u32) {
        debug_assert!(
            (b / 64) < self.nt,
            "bit {} out of range (nt={})",
            b,
            self.nt
        );
        self.bits[(b / 64) as usize] &= !(1u64 << (b % 64));
    }

    /// Test whether bit `b` is set.
    #[inline]
    pub fn has(&self, b: u32) -> bool {
        debug_assert!(
            (b / 64) < self.nt,
            "bit {} out of range (nt={})",
            b,
            self.nt
        );
        (self.bits[(b / 64) as usize] & (1u64 << (b % 64))) != 0
    }

    /// Copy contents from another bitset (must be same size).
    pub fn copy_from(&mut self, other: &BSet) {
        debug_assert_eq!(self.nt, other.nt);
        self.bits.copy_from_slice(&other.bits);
    }

    /// `self |= other`
    pub fn union(&mut self, other: &BSet) {
        debug_assert_eq!(self.nt, other.nt);
        for (a, b) in self.bits.iter_mut().zip(other.bits.iter()) {
            *a |= *b;
        }
    }

    /// `self &= other`
    pub fn inter(&mut self, other: &BSet) {
        debug_assert_eq!(self.nt, other.nt);
        for (a, b) in self.bits.iter_mut().zip(other.bits.iter()) {
            *a &= *b;
        }
    }

    /// `self &= !other`
    pub fn diff(&mut self, other: &BSet) {
        debug_assert_eq!(self.nt, other.nt);
        for (a, b) in self.bits.iter_mut().zip(other.bits.iter()) {
            *a &= !*b;
        }
    }

    /// Test equality.
    pub fn equal(&self, other: &BSet) -> bool {
        self.nt == other.nt && self.bits == other.bits
    }

    /// Reinitialize to hold at least `n` bits.
    pub fn init(&mut self, n: u32) {
        let nt = n.div_ceil(64);
        self.bits.clear();
        self.bits.resize(nt as usize, 0);
        self.nt = nt;
    }

    /// Return the backing words used for register masks.
    #[inline]
    pub fn bits_raw(&self) -> &[u64] {
        &self.bits
    }

    /// Mutable direct access to backing storage.
    #[inline]
    pub fn bits_raw_mut(&mut self) -> &mut [u64] {
        &mut self.bits
    }

    /// Iterate over all set bit indices.
    pub fn iter(&self) -> BSetIter<'_> {
        BSetIter {
            bits: &self.bits,
            word_idx: 0,
            word: if self.bits.is_empty() {
                0
            } else {
                self.bits[0]
            },
            base: 0,
        }
    }
}

/// Iterator over set bits in a `BSet`.
pub struct BSetIter<'a> {
    bits: &'a [u64],
    word_idx: usize,
    word: u64,
    base: u32,
}

impl<'a> Iterator for BSetIter<'a> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        loop {
            if self.word != 0 {
                let tz = self.word.trailing_zeros();
                self.word &= self.word.wrapping_sub(1); // clear lowest set bit
                return Some(self.base + tz);
            }
            self.word_idx += 1;
            if self.word_idx >= self.bits.len() {
                return None;
            }
            self.word = self.bits[self.word_idx];
            self.base = (self.word_idx as u32) * 64;
        }
    }
}

/// A basic block.
#[derive(Clone, Debug, Default)]
pub struct Blk {
    /// Phi nodes.
    pub phi: Vec<Phi>,
    /// Instructions.
    pub ins: Vec<Ins>,
    /// Block terminator.
    pub jmp: JmpInfo,
    /// Successor 1 (jump/fall-through target).
    pub s1: Option<BlkId>,
    /// Successor 2 (conditional branch target).
    pub s2: Option<BlkId>,

    /// Block id (RPO number).
    pub id: u32,
    /// Visit counter for graph traversals.
    pub visit: u32,

    /// Immediate dominator.
    pub idom: Option<BlkId>,
    /// First dominated child (dominator tree).
    pub dom: Option<BlkId>,
    /// Next sibling in dominator tree.
    pub dlink: Option<BlkId>,
    /// Dominance frontier.
    pub fron: Vec<BlkId>,

    /// Predecessors.
    pub pred: Vec<BlkId>,

    /// Live-in set.
    pub r#in: BSet,
    /// Live-out set.
    pub out: BSet,
    /// Gen set (defined before used).
    pub live_gen: BSet,

    /// Number of live values at block entry: `[int, float]`.
    pub nlive: [i32; 2],
    /// Loop nesting depth.
    pub loop_depth: i32,

    /// Block name (label).
    pub name: String,
}

/// Where a value is used.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum UseType {
    Xxx = 0,
    Phi = 1,
    Ins = 2,
    Jmp = 3,
}

/// Detail about the specific use site.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum UseDetail {
    /// Index of the instruction within its block.
    InsIdx(u32),
    /// Index of the phi within its block.
    PhiIdx(u32),
    /// Jump use (no extra index needed).
    Jmp,
    /// Unknown / unset.
    None,
}

/// A single use-def chain entry.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Use {
    pub typ: UseType,
    pub bid: u32,
    pub detail: UseDetail,
}

/// Alias analysis classification.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u8)]
#[derive(Default)]
pub enum AliasType {
    /// Bottom: no information.
    #[default]
    Bot = 0,
    /// Stack local (non-escaping).
    Loc = 1,
    /// Known constant address.
    Con = 2,
    /// Stack-escaping.
    Esc = 3,
    /// Known symbol.
    Sym = 4,
    /// Unknown / top.
    Unk = 6,
}

impl AliasType {
    /// Is this a stack-based alias? (Loc or Esc: bit 0 set.)
    #[inline]
    pub fn is_stack(self) -> bool {
        (self as u8) & 1 != 0
    }
}

/// Alias query result.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u8)]
pub enum AliasResult {
    NoAlias = 0,
    MayAlias = 1,
    MustAlias = 2,
}

/// Local alias metadata (for stack slots).
#[derive(Clone, Debug, Default)]
pub struct AliasLoc {
    /// Size in bytes, or -1 if larger than N_BIT.
    pub sz: i32,
    /// Bitmask of accessed bytes.
    pub m: u64,
}

/// Alias analysis information for a temporary.
#[derive(Clone, Debug)]
pub struct Alias {
    pub typ: AliasType,
    pub base: i32,
    pub offset: i64,
    /// Symbol info (for AliasType::Sym / Con).
    pub sym: Sym,
    /// Local access info (for AliasType::Loc / Esc).
    pub loc: AliasLoc,
    /// Pointer to the underlying slot alias, if any.
    pub slot: Option<Box<Alias>>,
}

impl Default for Alias {
    fn default() -> Self {
        Self {
            typ: AliasType::Bot,
            base: 0,
            offset: 0,
            sym: Sym::default(),
            loc: AliasLoc::default(),
            slot: None,
        }
    }
}

/// Sub-register access width. Must match `Oload`/`Oext` order.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u8)]
#[derive(Default)]
pub enum Width {
    #[default]
    WFull = 0,
    Wsb = 1,
    Wub = 2,
    Wsh = 3,
    Wuh = 4,
    Wsw = 5,
    Wuw = 6,
}

/// Register allocation hint for a temporary.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub struct RegHint {
    /// Preferred physical register.
    pub register: Option<PhysicalRegister>,
    /// Weight.
    pub w: i32,
    /// Bitmask of registers to avoid.
    pub avoid: RegisterSet,
}

/// A temporary (virtual register) with all associated metadata.
#[derive(Clone, Debug)]
pub struct Tmp {
    pub name: String,
    /// Defining instruction index (within the block), or None.
    pub def: Option<usize>,
    /// Use-def chain.
    pub uses: Vec<Use>,
    /// Number of definitions.
    pub ndef: u32,
    /// Number of uses.
    pub nuse: u32,
    /// Id of a defining block.
    pub bid: u32,
    /// Spill cost.
    pub cost: u32,
    /// Stack-slot assignment after ABI lowering or spilling.
    pub slot: Option<StackSlot>,
    /// Value class.
    pub cls: Cls,
    /// Register allocation hint.
    pub hint: RegHint,
    /// Phi class.
    pub phi: i32,
    /// Alias information.
    pub alias: Alias,
    /// Access width.
    pub width: Width,
    /// Visit counter.
    pub visit: i32,
}

impl Default for Tmp {
    fn default() -> Self {
        Self {
            name: String::new(),
            def: None,
            uses: Vec::new(),
            ndef: 0,
            nuse: 0,
            bid: 0,
            cost: 0,
            slot: None,
            cls: Cls::Kx,
            hint: RegHint::default(),
            phi: 0,
            alias: Alias::default(),
            width: Width::WFull,
            visit: 0,
        }
    }
}

/// Symbol type.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u8)]
#[derive(Default)]
pub enum SymType {
    /// Global symbol.
    #[default]
    Glo = 0,
    /// Thread-local symbol.
    Thr = 1,
}

/// A symbol reference.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct Sym {
    pub typ: SymType,
    pub id: u32,
}

/// Constant type.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u8)]
#[derive(Default)]
pub enum ConType {
    #[default]
    Undef = 0,
    Bits = 1,
    Addr = 2,
}

/// Constant bits: stored as a union-like value.
#[derive(Copy, Clone, Debug, Default)]
pub struct ConBits {
    raw: u64,
}

impl ConBits {
    pub fn from_i64(v: i64) -> Self {
        Self { raw: v as u64 }
    }

    pub fn from_f64(v: f64) -> Self {
        Self { raw: v.to_bits() }
    }

    pub fn from_f32(v: f32) -> Self {
        Self {
            raw: v.to_bits() as u64,
        }
    }

    #[inline]
    pub fn i(self) -> i64 {
        self.raw as i64
    }

    #[inline]
    pub fn d(self) -> f64 {
        f64::from_bits(self.raw)
    }

    #[inline]
    pub fn s(self) -> f32 {
        f32::from_bits(self.raw as u32)
    }
}

/// A constant value.
#[derive(Copy, Clone, Debug, Default)]
pub struct Con {
    pub typ: ConType,
    pub sym: Sym,
    pub bits: ConBits,
    /// 0 = not float, 1 = print as f32, 2 = print as f64.
    pub flt: u8,
}

/// Memory addressing mode (primarily for amd64, kept for compatibility).
#[derive(Clone, Debug, Default)]
pub struct Mem {
    pub offset: Con,
    pub base: Ref,
    pub index: Ref,
    pub scale: i32,
}

/// Linkage information for a symbol.
#[derive(Clone, Debug, Default)]
pub struct Lnk {
    pub export: bool,
    pub thread: bool,
    pub align: u8,
    pub sec: Option<String>,
    pub secf: Option<String>,
}

/// The main function IR representation.
#[derive(Clone, Debug)]
pub struct Fn {
    /// Entry block.
    pub start: BlkId,
    /// All basic blocks (indexed by `BlkId`).
    pub blks: Vec<Blk>,
    /// All temporaries (indexed by `TmpId`).
    pub tmps: Vec<Tmp>,
    /// All constants (indexed by `ConId`).
    pub cons: Vec<Con>,
    /// All memory addressing modes (indexed by `MemId`).
    pub mems: Vec<Mem>,
    /// Reverse post-order block list.
    pub rpo: Vec<BlkId>,
    /// Index in typ[] for aggregate return, -1 if none.
    pub retty: i32,
    /// Return value reference.
    pub retr: Ref,
    /// Global register mask (registers live across calls).
    pub reg: u64,
    /// Stack slot counter.
    pub slot: i32,
    /// Whether the function is variadic.
    pub vararg: bool,
    /// Whether dynamic stack allocation is used.
    pub dynalloc: bool,
    /// Function name.
    pub name: String,
    /// Linkage info.
    pub lnk: Lnk,
    /// Symbol name table (indexed by Sym.id).
    pub strs: Vec<String>,
}

impl Default for Fn {
    fn default() -> Self {
        Self {
            start: BlkId(0),
            blks: Vec::new(),
            tmps: Vec::new(),
            cons: Vec::new(),
            mems: Vec::new(),
            rpo: Vec::new(),
            retty: -1,
            retr: Ref::R,
            reg: 0,
            slot: 0,
            vararg: false,
            dynalloc: false,
            name: String::new(),
            lnk: Lnk::default(),
            strs: Vec::new(),
        }
    }
}

/// Field type within an aggregate.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
#[repr(u8)]
#[derive(Default)]
pub enum FieldType {
    #[default]
    End = 0,
    Fb = 1,
    Fh = 2,
    Fw = 3,
    Fl = 4,
    Fs = 5,
    Fd = 6,
    FPad = 7,
    FTyp = 8,
}

/// A single field in an aggregate type.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct Field {
    pub typ: FieldType,
    /// Length, or index in `typ[]` for `FTyp`.
    pub len: u32,
}

/// An aggregate type definition.
#[derive(Clone, Debug, Default)]
pub struct Typ {
    pub name: String,
    pub is_dark: bool,
    pub is_union: bool,
    pub align: i32,
    pub size: u64,
    pub nunion: u32,
    /// One `Vec<Field>` per union variant.
    pub fields: Vec<Vec<Field>>,
}

/// A data segment item.
#[derive(Clone, Debug)]
pub enum DatItem {
    Start,
    End,
    Byte(i64),
    Half(i64),
    Word(i64),
    Long(i64),
    Zero(u64),
    Str(String),
    Ref { name: String, off: i64 },
    FltD(f64),
    FltS(f32),
}

/// A data definition entry.
#[derive(Clone, Debug)]
pub struct Dat {
    pub item: DatItem,
    pub name: Option<String>,
    pub lnk: Option<Lnk>,
}

/// Private machine architecture selected by the backend adapter.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Architecture {
    Amd64,
    Aarch64,
}

/// Target-specific configuration.
#[derive(Clone, Debug)]
pub struct Target {
    pub architecture: Architecture,
    pub apple: bool,
    /// First general-purpose register.
    pub gpr0: i32,
    /// Number of general-purpose registers.
    pub ngpr: i32,
    /// First floating-point register.
    pub fpr0: i32,
    /// Number of floating-point registers.
    pub nfpr: i32,
    /// Globally live registers (e.g., sp, fp).
    pub rglob: u64,
    /// Number of globally live registers.
    pub nrglob: i32,
    /// Caller-save register list.
    pub rsave: &'static [i32],
    /// Number of caller-save registers: `[gpr, fpr]`.
    pub nrsave: [i32; 2],
}

impl Target {
    /// Return call-result registers and optionally their per-class counts.
    pub fn retregs(&self, call: Ref, nlv: Option<&mut [i32; 2]>) -> u64 {
        match self.architecture {
            Architecture::Amd64 => crate::codegen::amd64::retregs(call, nlv),
            Architecture::Aarch64 => crate::codegen::aarch64::retregs(call, nlv),
        }
    }

    /// Return call-argument registers and optionally their per-class counts.
    pub fn argregs(&self, call: Ref, m: Option<&mut [i32; 2]>) -> u64 {
        match self.architecture {
            Architecture::Amd64 => crate::codegen::amd64::argregs(call, m),
            Architecture::Aarch64 => crate::codegen::aarch64::argregs(call, m),
        }
    }

    /// Return the number of memory operands an instruction can fold.
    pub fn memargs(&self, op: Op) -> i32 {
        match self.architecture {
            Architecture::Amd64 => crate::codegen::amd64::memargs(op),
            Architecture::Aarch64 => 0,
        }
    }
}

/// Metadata for a single opcode.
#[derive(Copy, Clone, Debug)]
pub struct OpInfo {
    pub name: &'static str,
    /// Argument class constraints: `argcls[arg_position][class]`.
    /// Each entry maps a `Cls` to the required `Cls` for that argument.
    /// `Cls::Kx` means "not applicable".
    pub argcls: [[Cls; 4]; 2],
    /// Whether the operation can be constant-folded.
    pub can_fold: bool,
}

impl Default for OpInfo {
    fn default() -> Self {
        Self {
            name: "xxx",
            argcls: [[Cls::Kx; 4]; 2],
            can_fold: false,
        }
    }
}

/// Use Kx, Kw, Kl, Ks, Kd short-hands within the table.
const X: Cls = Cls::Kx;
const W: Cls = Cls::Kw;
const L: Cls = Cls::Kl;
const S: Cls = Cls::Ks;
const D: Cls = Cls::Kd;

/// Build opcode metadata from two argument-class tables.
const fn oi(name: &'static str, a: [[Cls; 4]; 2], can_fold: bool) -> OpInfo {
    OpInfo {
        name,
        argcls: a,
        can_fold,
    }
}

/// Unavailable argument class.
const E: Cls = Cls::Kx;

/// Pointer-sized memory argument class.
const M: Cls = Cls::Kw;

/// Static opcode metadata table, indexed by `Op` discriminant.
///
/// Index 0 is `Oxxx`; remaining entries follow `Op` discriminants.
pub static OP_TABLE: [OpInfo; N_OP] = {
    let mut t = [OpInfo {
        name: "",
        argcls: [[X; 4]; 2],
        can_fold: false,
    }; N_OP];

    t[Op::Oxxx as usize] = oi("xxx", [[X, X, X, X], [X, X, X, X]], false);

    t[Op::Add as usize] = oi("add", [[W, L, S, D], [W, L, S, D]], true);
    t[Op::Sub as usize] = oi("sub", [[W, L, S, D], [W, L, S, D]], true);
    t[Op::Neg as usize] = oi("neg", [[W, L, S, D], [X, X, X, X]], true);
    t[Op::Div as usize] = oi("div", [[W, L, S, D], [W, L, S, D]], true);
    t[Op::Rem as usize] = oi("rem", [[W, L, E, E], [W, L, E, E]], true);
    t[Op::Udiv as usize] = oi("udiv", [[W, L, E, E], [W, L, E, E]], true);
    t[Op::Urem as usize] = oi("urem", [[W, L, E, E], [W, L, E, E]], true);
    t[Op::Mul as usize] = oi("mul", [[W, L, S, D], [W, L, S, D]], true);
    t[Op::And as usize] = oi("and", [[W, L, E, E], [W, L, E, E]], true);
    t[Op::Or as usize] = oi("or", [[W, L, E, E], [W, L, E, E]], true);
    t[Op::Xor as usize] = oi("xor", [[W, L, E, E], [W, L, E, E]], true);
    t[Op::Sar as usize] = oi("sar", [[W, L, E, E], [W, W, E, E]], true);
    t[Op::Shr as usize] = oi("shr", [[W, L, E, E], [W, W, E, E]], true);
    t[Op::Shl as usize] = oi("shl", [[W, L, E, E], [W, W, E, E]], true);

    t[Op::Ceqw as usize] = oi("ceqw", [[W, W, E, E], [W, W, E, E]], true);
    t[Op::Cnew as usize] = oi("cnew", [[W, W, E, E], [W, W, E, E]], true);
    t[Op::Csgew as usize] = oi("csgew", [[W, W, E, E], [W, W, E, E]], true);
    t[Op::Csgtw as usize] = oi("csgtw", [[W, W, E, E], [W, W, E, E]], true);
    t[Op::Cslew as usize] = oi("cslew", [[W, W, E, E], [W, W, E, E]], true);
    t[Op::Csltw as usize] = oi("csltw", [[W, W, E, E], [W, W, E, E]], true);
    t[Op::Cugew as usize] = oi("cugew", [[W, W, E, E], [W, W, E, E]], true);
    t[Op::Cugtw as usize] = oi("cugtw", [[W, W, E, E], [W, W, E, E]], true);
    t[Op::Culew as usize] = oi("culew", [[W, W, E, E], [W, W, E, E]], true);
    t[Op::Cultw as usize] = oi("cultw", [[W, W, E, E], [W, W, E, E]], true);

    t[Op::Ceql as usize] = oi("ceql", [[L, L, E, E], [L, L, E, E]], true);
    t[Op::Cnel as usize] = oi("cnel", [[L, L, E, E], [L, L, E, E]], true);
    t[Op::Csgel as usize] = oi("csgel", [[L, L, E, E], [L, L, E, E]], true);
    t[Op::Csgtl as usize] = oi("csgtl", [[L, L, E, E], [L, L, E, E]], true);
    t[Op::Cslel as usize] = oi("cslel", [[L, L, E, E], [L, L, E, E]], true);
    t[Op::Csltl as usize] = oi("csltl", [[L, L, E, E], [L, L, E, E]], true);
    t[Op::Cugel as usize] = oi("cugel", [[L, L, E, E], [L, L, E, E]], true);
    t[Op::Cugtl as usize] = oi("cugtl", [[L, L, E, E], [L, L, E, E]], true);
    t[Op::Culel as usize] = oi("culel", [[L, L, E, E], [L, L, E, E]], true);
    t[Op::Cultl as usize] = oi("cultl", [[L, L, E, E], [L, L, E, E]], true);

    t[Op::Ceqs as usize] = oi("ceqs", [[S, S, E, E], [S, S, E, E]], true);
    t[Op::Cges as usize] = oi("cges", [[S, S, E, E], [S, S, E, E]], true);
    t[Op::Cgts as usize] = oi("cgts", [[S, S, E, E], [S, S, E, E]], true);
    t[Op::Cles as usize] = oi("cles", [[S, S, E, E], [S, S, E, E]], true);
    t[Op::Clts as usize] = oi("clts", [[S, S, E, E], [S, S, E, E]], true);
    t[Op::Cnes as usize] = oi("cnes", [[S, S, E, E], [S, S, E, E]], true);
    t[Op::Cos as usize] = oi("cos", [[S, S, E, E], [S, S, E, E]], true);
    t[Op::Cuos as usize] = oi("cuos", [[S, S, E, E], [S, S, E, E]], true);

    t[Op::Ceqd as usize] = oi("ceqd", [[D, D, E, E], [D, D, E, E]], true);
    t[Op::Cged as usize] = oi("cged", [[D, D, E, E], [D, D, E, E]], true);
    t[Op::Cgtd as usize] = oi("cgtd", [[D, D, E, E], [D, D, E, E]], true);
    t[Op::Cled as usize] = oi("cled", [[D, D, E, E], [D, D, E, E]], true);
    t[Op::Cltd as usize] = oi("cltd", [[D, D, E, E], [D, D, E, E]], true);
    t[Op::Cned as usize] = oi("cned", [[D, D, E, E], [D, D, E, E]], true);
    t[Op::Cod as usize] = oi("cod", [[D, D, E, E], [D, D, E, E]], true);
    t[Op::Cuod as usize] = oi("cuod", [[D, D, E, E], [D, D, E, E]], true);

    t[Op::Storeb as usize] = oi("storeb", [[W, E, E, E], [M, E, E, E]], false);
    t[Op::Storeh as usize] = oi("storeh", [[W, E, E, E], [M, E, E, E]], false);
    t[Op::Storew as usize] = oi("storew", [[W, E, E, E], [M, E, E, E]], false);
    t[Op::Storel as usize] = oi("storel", [[L, E, E, E], [M, E, E, E]], false);
    t[Op::Stores as usize] = oi("stores", [[S, E, E, E], [M, E, E, E]], false);
    t[Op::Stored as usize] = oi("stored", [[D, E, E, E], [M, E, E, E]], false);

    t[Op::Loadsb as usize] = oi("loadsb", [[M, M, E, E], [X, X, E, E]], false);
    t[Op::Loadub as usize] = oi("loadub", [[M, M, E, E], [X, X, E, E]], false);
    t[Op::Loadsh as usize] = oi("loadsh", [[M, M, E, E], [X, X, E, E]], false);
    t[Op::Loaduh as usize] = oi("loaduh", [[M, M, E, E], [X, X, E, E]], false);
    t[Op::Loadsw as usize] = oi("loadsw", [[M, M, E, E], [X, X, E, E]], false);
    t[Op::Loaduw as usize] = oi("loaduw", [[M, M, E, E], [X, X, E, E]], false);
    t[Op::Load as usize] = oi("load", [[M, M, M, M], [X, X, X, X]], false);

    t[Op::Extsb as usize] = oi("extsb", [[W, W, E, E], [X, X, E, E]], true);
    t[Op::Extub as usize] = oi("extub", [[W, W, E, E], [X, X, E, E]], true);
    t[Op::Extsh as usize] = oi("extsh", [[W, W, E, E], [X, X, E, E]], true);
    t[Op::Extuh as usize] = oi("extuh", [[W, W, E, E], [X, X, E, E]], true);
    t[Op::Extsw as usize] = oi("extsw", [[E, W, E, E], [E, X, E, E]], true);
    t[Op::Extuw as usize] = oi("extuw", [[E, W, E, E], [E, X, E, E]], true);

    t[Op::Exts as usize] = oi("exts", [[E, E, E, S], [E, E, E, X]], true);
    t[Op::Truncd as usize] = oi("truncd", [[E, E, D, E], [E, E, X, E]], true);
    t[Op::Stosi as usize] = oi("stosi", [[S, S, E, E], [X, X, E, E]], true);
    t[Op::Stoui as usize] = oi("stoui", [[S, S, E, E], [X, X, E, E]], true);
    t[Op::Dtosi as usize] = oi("dtosi", [[D, D, E, E], [X, X, E, E]], true);
    t[Op::Dtoui as usize] = oi("dtoui", [[D, D, E, E], [X, X, E, E]], true);
    t[Op::Swtof as usize] = oi("swtof", [[E, E, W, W], [E, E, X, X]], true);
    t[Op::Uwtof as usize] = oi("uwtof", [[E, E, W, W], [E, E, X, X]], true);
    t[Op::Sltof as usize] = oi("sltof", [[E, E, L, L], [E, E, X, X]], true);
    t[Op::Ultof as usize] = oi("ultof", [[E, E, L, L], [E, E, X, X]], true);
    t[Op::Cast as usize] = oi("cast", [[S, D, W, L], [X, X, X, X]], true);

    t[Op::Alloc4 as usize] = oi("alloc4", [[E, L, E, E], [E, X, E, E]], false);
    t[Op::Alloc8 as usize] = oi("alloc8", [[E, L, E, E], [E, X, E, E]], false);
    t[Op::Alloc16 as usize] = oi("alloc16", [[E, L, E, E], [E, X, E, E]], false);

    t[Op::Vaarg as usize] = oi("vaarg", [[M, M, M, M], [X, X, X, X]], false);
    t[Op::Vastart as usize] = oi("vastart", [[M, E, E, E], [X, E, E, E]], false);

    t[Op::Copy as usize] = oi("copy", [[W, L, S, D], [X, X, X, X]], false);
    t[Op::Dbgloc as usize] = oi("dbgloc", [[W, E, E, E], [W, E, E, E]], false);

    t[Op::Nop as usize] = oi("nop", [[X, X, X, X], [X, X, X, X]], false);
    t[Op::Addr as usize] = oi("addr", [[M, M, E, E], [X, X, E, E]], false);
    t[Op::Blit0 as usize] = oi("blit0", [[M, E, E, E], [M, E, E, E]], false);
    t[Op::Blit1 as usize] = oi("blit1", [[W, E, E, E], [X, E, E, E]], false);
    t[Op::Swap as usize] = oi("swap", [[W, L, S, D], [W, L, S, D]], false);
    t[Op::Sign as usize] = oi("sign", [[W, L, E, E], [X, X, E, E]], false);
    t[Op::Salloc as usize] = oi("salloc", [[E, L, E, E], [E, X, E, E]], false);
    t[Op::Xidiv as usize] = oi("xidiv", [[W, L, E, E], [X, X, E, E]], false);
    t[Op::Xdiv as usize] = oi("xdiv", [[W, L, E, E], [X, X, E, E]], false);
    t[Op::Xcmp as usize] = oi("xcmp", [[W, L, S, D], [W, L, S, D]], false);
    t[Op::Xtest as usize] = oi("xtest", [[W, L, E, E], [W, L, E, E]], false);
    t[Op::Acmp as usize] = oi("acmp", [[W, L, E, E], [W, L, E, E]], false);
    t[Op::Acmn as usize] = oi("acmn", [[W, L, E, E], [W, L, E, E]], false);
    t[Op::Afcmp as usize] = oi("afcmp", [[E, E, S, D], [E, E, S, D]], false);
    t[Op::Reqz as usize] = oi("reqz", [[W, L, E, E], [X, X, E, E]], false);
    t[Op::Rnez as usize] = oi("rnez", [[W, L, E, E], [X, X, E, E]], false);

    t[Op::Par as usize] = oi("par", [[X, X, X, X], [X, X, X, X]], false);
    t[Op::Parsb as usize] = oi("parsb", [[X, X, X, X], [X, X, X, X]], false);
    t[Op::Parub as usize] = oi("parub", [[X, X, X, X], [X, X, X, X]], false);
    t[Op::Parsh as usize] = oi("parsh", [[X, X, X, X], [X, X, X, X]], false);
    t[Op::Paruh as usize] = oi("paruh", [[X, X, X, X], [X, X, X, X]], false);
    t[Op::Parc as usize] = oi("parc", [[E, X, E, E], [E, X, E, E]], false);
    t[Op::Pare as usize] = oi("pare", [[E, X, E, E], [E, X, E, E]], false);

    t[Op::Arg as usize] = oi("arg", [[W, L, S, D], [X, X, X, X]], false);
    t[Op::Argsb as usize] = oi("argsb", [[W, E, E, E], [X, X, X, X]], false);
    t[Op::Argub as usize] = oi("argub", [[W, E, E, E], [X, X, X, X]], false);
    t[Op::Argsh as usize] = oi("argsh", [[W, E, E, E], [X, X, X, X]], false);
    t[Op::Arguh as usize] = oi("arguh", [[W, E, E, E], [X, X, X, X]], false);
    t[Op::Argc as usize] = oi("argc", [[E, X, E, E], [E, L, E, E]], false);
    t[Op::Arge as usize] = oi("arge", [[E, L, E, E], [E, X, E, E]], false);
    t[Op::Argv as usize] = oi("argv", [[X, X, X, X], [X, X, X, X]], false);
    t[Op::Call as usize] = oi("call", [[M, M, M, M], [X, X, X, X]], false);

    t[Op::Flagieq as usize] = oi("flagieq", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagine as usize] = oi("flagine", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagisge as usize] = oi("flagisge", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagisgt as usize] = oi("flagisgt", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagisle as usize] = oi("flagisle", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagislt as usize] = oi("flagislt", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagiuge as usize] = oi("flagiuge", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagiugt as usize] = oi("flagiugt", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagiule as usize] = oi("flagiule", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagiult as usize] = oi("flagiult", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagfeq as usize] = oi("flagfeq", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagfge as usize] = oi("flagfge", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagfgt as usize] = oi("flagfgt", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagfle as usize] = oi("flagfle", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagflt as usize] = oi("flagflt", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagfne as usize] = oi("flagfne", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagfo as usize] = oi("flagfo", [[X, X, E, E], [X, X, E, E]], false);
    t[Op::Flagfuo as usize] = oi("flagfuo", [[X, X, E, E], [X, X, E, E]], false);

    t
};

/// Check if a reference refers to a physical register (TmpId < Tmp0).
#[inline]
pub fn isreg(r: Ref) -> bool {
    match r {
        Ref::Tmp(id) => id.0 > 0 && id.0 < TMP0,
        _ => false,
    }
}

/// Get the argument class for an instruction's nth argument, given the
/// instruction's class. Returns `Cls::Kx` if not applicable.
#[inline]
pub fn argcls(ins: &Ins, n: usize) -> Cls {
    debug_assert!(n < 2);
    let op = ins.op as usize;
    if op >= N_OP {
        return Cls::Kx;
    }
    let cls = ins.cls;
    if cls == Cls::Kx {
        return Cls::Kx;
    }
    OP_TABLE[op].argcls[n][cls as i8 as usize]
}

/// Negate a condition code index.
pub fn cc_neg(c: u16) -> u16 {
    /// [negation, swap] indexed by condition code (0..N_CMP).
    static CMPTAB: [[u16; 2]; N_CMP] = [
        [1, 0],   // 0  ieq  -> neg=ine(1),  swap=ieq(0)
        [0, 1],   // 1  ine  -> neg=ieq(0),  swap=ine(1)
        [5, 4],   // 2  isge -> neg=islt(5), swap=isle(4)
        [4, 5],   // 3  isgt -> neg=isle(4), swap=islt(5)
        [3, 2],   // 4  isle -> neg=isgt(3), swap=isge(2)
        [2, 3],   // 5  islt -> neg=isge(2), swap=isgt(3)
        [9, 8],   // 6  iuge -> neg=iult(9), swap=iule(8)
        [8, 9],   // 7  iugt -> neg=iule(8), swap=iult(9)
        [7, 6],   // 8  iule -> neg=iugt(7), swap=iuge(6)
        [6, 7],   // 9  iult -> neg=iuge(6), swap=iugt(7)
        [15, 10], // 10 feq -> neg=fne(15), swap=feq(10)
        [14, 13], // 11 fge -> neg=flt(14), swap=fle(13)
        [13, 14], // 12 fgt -> neg=fle(13), swap=flt(14)
        [12, 11], // 13 fle -> neg=fgt(12), swap=fge(11)
        [11, 12], // 14 flt -> neg=fge(11), swap=fgt(12)
        [10, 15], // 15 fne -> neg=feq(10), swap=fne(15)
        [17, 16], // 16 fo  -> neg=fuo(17), swap=fo(16)
        [16, 17], // 17 fuo -> neg=fo(16),  swap=fuo(17)
    ];
    debug_assert!((c as usize) < N_CMP, "cc_neg: index {c} out of range");
    CMPTAB[c as usize][0]
}

/// Swap a condition code's operand order.
pub fn cc_swap(c: u16) -> u16 {
    static CMPTAB: [[u16; 2]; N_CMP] = [
        [1, 0],
        [0, 1],
        [5, 4],
        [4, 5],
        [3, 2],
        [2, 3],
        [9, 8],
        [8, 9],
        [7, 6],
        [6, 7],
        [15, 10],
        [14, 13],
        [13, 14],
        [12, 11],
        [11, 12],
        [10, 15],
        [17, 16],
        [16, 17],
    ];
    debug_assert!((c as usize) < N_CMP, "cc_swap: index {c} out of range");
    CMPTAB[c as usize][1]
}

/// Merge two classes. Returns `true` if `*dest` changed.
/// `Kx` is top (merges with anything), same class stays, otherwise error.
pub fn clsmerge(dest: &mut Cls, src: Cls) -> bool {
    if src == Cls::Kx {
        return false;
    }
    if *dest == Cls::Kx {
        *dest = src;
        return true;
    }
    *dest == src // returns false if they match (no change), panics would be wrong here
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_newtypes() {
        let b = BlkId::from(42u32);
        assert_eq!(b.index(), 42);
        assert_eq!(b.0, 42);
        assert!(!b.is_none());
        assert!(BlkId::NONE.is_none());
    }

    #[test]
    fn test_ref_basics() {
        assert!(Ref::R.is_none());
        assert!(!Ref::R.is_tmp());
        assert!(Ref::Tmp(TmpId(5)).is_tmp());
        assert_eq!(Ref::R.rtype(), -1);
        assert_eq!(Ref::Tmp(TmpId(1)).rtype(), 0);
        assert_eq!(Ref::Con(ConId(1)).rtype(), 1);
        assert_eq!(Ref::UNDEF, Ref::Con(ConId(0)));
        assert_eq!(Ref::CON_Z, Ref::Con(ConId(1)));
    }

    #[test]
    fn test_cls() {
        assert!(Cls::Kl.is_wide());
        assert!(Cls::Kd.is_wide());
        assert!(!Cls::Kw.is_wide());
        assert!(!Cls::Ks.is_wide());

        assert!(Cls::Ks.is_float());
        assert!(Cls::Kd.is_float());
        assert!(!Cls::Kw.is_float());

        assert_eq!(Cls::Kw.base(), 0);
        assert_eq!(Cls::Kl.base(), 0);
        assert_eq!(Cls::Ks.base(), 1);
        assert_eq!(Cls::Kd.base(), 1);
    }

    #[test]
    fn test_op_ranges() {
        assert!(Op::Add.can_fold());
        assert!(!Op::Storeb.can_fold());
        assert!(Op::Storeb.is_store());
        assert!(Op::Stored.is_store());
        assert!(!Op::Load.is_store());
        assert!(Op::Load.is_load());
        assert!(Op::Loadsb.is_load());
        assert!(Op::Extsb.is_ext());
        assert!(Op::Extuw.is_ext());
        assert!(!Op::Copy.is_ext());
        assert!(Op::Par.is_par());
        assert!(Op::Pare.is_par());
        assert!(!Op::Arg.is_par());
        assert!(Op::Arg.is_arg());
        assert!(Op::Argv.is_arg());
    }

    #[test]
    fn test_jmp_ranges() {
        assert!(Jmp::Retw.is_ret());
        assert!(Jmp::Ret0.is_ret());
        assert!(!Jmp::Jmp_.is_ret());
        assert!(Jmp::Retsb.is_retbh());
        assert!(Jmp::Retuh.is_retbh());
        assert!(!Jmp::Retw.is_retbh());
    }

    #[test]
    fn test_bset() {
        let mut bs = BSet::new(128);
        assert_eq!(bs.count(), 0);
        bs.set(0);
        bs.set(63);
        bs.set(64);
        bs.set(127);
        assert_eq!(bs.count(), 4);
        assert!(bs.has(0));
        assert!(bs.has(63));
        assert!(bs.has(64));
        assert!(bs.has(127));
        assert!(!bs.has(1));

        let items: Vec<u32> = bs.iter().collect();
        assert_eq!(items, vec![0, 63, 64, 127]);

        bs.clr(63);
        assert!(!bs.has(63));
        assert_eq!(bs.count(), 3);

        let mut bs2 = BSet::new(128);
        bs2.set(64);
        bs2.set(100);

        let mut bs3 = bs.clone();
        bs3.inter(&bs2);
        assert_eq!(bs3.count(), 1);
        assert!(bs3.has(64));
    }

    #[test]
    fn test_op_table_names() {
        assert_eq!(OP_TABLE[Op::Add as usize].name, "add");
        assert_eq!(OP_TABLE[Op::Ceqw as usize].name, "ceqw");
        assert_eq!(OP_TABLE[Op::Storeb as usize].name, "storeb");
        assert_eq!(OP_TABLE[Op::Load as usize].name, "load");
        assert_eq!(OP_TABLE[Op::Nop as usize].name, "nop");
        assert_eq!(OP_TABLE[Op::Call as usize].name, "call");
        assert_eq!(OP_TABLE[Op::Flagfuo as usize].name, "flagfuo");
    }

    #[test]
    fn test_op_count() {
        const { assert!(N_OP > 90) };
    }

    #[test]
    fn test_con_bits() {
        let cb = ConBits::from_i64(42);
        assert_eq!(cb.i(), 42);

        let cb = ConBits::from_f64(std::f64::consts::PI);
        assert!((cb.d() - std::f64::consts::PI).abs() < f64::EPSILON);

        let cb = ConBits::from_f32(2.5);
        assert!((cb.s() - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_isreg() {
        assert!(!isreg(Ref::R));
        assert!(!isreg(Ref::Con(ConId(0))));
        assert!(isreg(Ref::Tmp(TmpId(1))));
        assert!(isreg(Ref::Tmp(TmpId(63))));
        assert!(!isreg(Ref::Tmp(TmpId(0)))); // R is Tmp(0)
        assert!(!isreg(Ref::Tmp(TmpId(TMP0)))); // first non-reg
        assert!(!isreg(Ref::Tmp(TmpId(TMP0 + 1))));
    }

    #[test]
    fn test_width_default() {
        assert_eq!(Width::default(), Width::WFull);
    }
}
