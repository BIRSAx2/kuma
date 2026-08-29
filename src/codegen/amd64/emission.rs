//! AMD64 assembly emission.

use std::fmt::Write;

use crate::ir::builder;
use crate::ir::internal::*;

use super::registers::*;

const KI: i8 = -1;
const KA: i8 = -2;

#[derive(Copy, Clone)]
struct OmapEntry {
    op: Op,
    cls: i8,
    asm: &'static str,
}

static OMAP: &[OmapEntry] = &[
    OmapEntry {
        op: Op::Add,
        cls: KA,
        asm: "+add%k %1, %=",
    },
    OmapEntry {
        op: Op::Sub,
        cls: KA,
        asm: "-sub%k %1, %=",
    },
    OmapEntry {
        op: Op::And,
        cls: KI,
        asm: "+and%k %1, %=",
    },
    OmapEntry {
        op: Op::Or,
        cls: KI,
        asm: "+or%k %1, %=",
    },
    OmapEntry {
        op: Op::Xor,
        cls: KI,
        asm: "+xor%k %1, %=",
    },
    OmapEntry {
        op: Op::Sar,
        cls: KI,
        asm: "-sar%k %B1, %=",
    },
    OmapEntry {
        op: Op::Shr,
        cls: KI,
        asm: "-shr%k %B1, %=",
    },
    OmapEntry {
        op: Op::Shl,
        cls: KI,
        asm: "-shl%k %B1, %=",
    },
    OmapEntry {
        op: Op::Mul,
        cls: KI,
        asm: "+imul%k %1, %=",
    },
    OmapEntry {
        op: Op::Mul,
        cls: Cls::Ks as i8,
        asm: "+mulss %1, %=",
    },
    OmapEntry {
        op: Op::Mul,
        cls: Cls::Kd as i8,
        asm: "+mulsd %1, %=",
    },
    OmapEntry {
        op: Op::Div,
        cls: KA,
        asm: "-div%k %1, %=",
    },
    OmapEntry {
        op: Op::Storel,
        cls: KA,
        asm: "movq %L0, %M1",
    },
    OmapEntry {
        op: Op::Storew,
        cls: KA,
        asm: "movl %W0, %M1",
    },
    OmapEntry {
        op: Op::Storeh,
        cls: KA,
        asm: "movw %H0, %M1",
    },
    OmapEntry {
        op: Op::Storeb,
        cls: KA,
        asm: "movb %B0, %M1",
    },
    OmapEntry {
        op: Op::Stores,
        cls: KA,
        asm: "movss %S0, %M1",
    },
    OmapEntry {
        op: Op::Stored,
        cls: KA,
        asm: "movsd %D0, %M1",
    },
    OmapEntry {
        op: Op::Load,
        cls: KA,
        asm: "mov%k %M0, %=",
    },
    OmapEntry {
        op: Op::Loadsw,
        cls: Cls::Kl as i8,
        asm: "movslq %M0, %L=",
    },
    OmapEntry {
        op: Op::Loadsw,
        cls: Cls::Kw as i8,
        asm: "movl %M0, %W=",
    },
    OmapEntry {
        op: Op::Loaduw,
        cls: KI,
        asm: "movl %M0, %W=",
    },
    OmapEntry {
        op: Op::Loadsh,
        cls: KI,
        asm: "movsw%k %M0, %=",
    },
    OmapEntry {
        op: Op::Loaduh,
        cls: KI,
        asm: "movzw%k %M0, %=",
    },
    OmapEntry {
        op: Op::Loadsb,
        cls: KI,
        asm: "movsb%k %M0, %=",
    },
    OmapEntry {
        op: Op::Loadub,
        cls: KI,
        asm: "movzb%k %M0, %=",
    },
    OmapEntry {
        op: Op::Extsw,
        cls: Cls::Kl as i8,
        asm: "movslq %W0, %L=",
    },
    OmapEntry {
        op: Op::Extuw,
        cls: Cls::Kl as i8,
        asm: "movl %W0, %W=",
    },
    OmapEntry {
        op: Op::Extsh,
        cls: KI,
        asm: "movsw%k %H0, %=",
    },
    OmapEntry {
        op: Op::Extuh,
        cls: KI,
        asm: "movzw%k %H0, %=",
    },
    OmapEntry {
        op: Op::Extsb,
        cls: KI,
        asm: "movsb%k %B0, %=",
    },
    OmapEntry {
        op: Op::Extub,
        cls: KI,
        asm: "movzb%k %B0, %=",
    },
    OmapEntry {
        op: Op::Exts,
        cls: Cls::Kd as i8,
        asm: "cvtss2sd %0, %=",
    },
    OmapEntry {
        op: Op::Truncd,
        cls: Cls::Ks as i8,
        asm: "cvtsd2ss %0, %=",
    },
    OmapEntry {
        op: Op::Stosi,
        cls: KI,
        asm: "cvttss2si%k %0, %=",
    },
    OmapEntry {
        op: Op::Dtosi,
        cls: KI,
        asm: "cvttsd2si%k %0, %=",
    },
    OmapEntry {
        op: Op::Swtof,
        cls: KA,
        asm: "cvtsi2%k %W0, %=",
    },
    OmapEntry {
        op: Op::Sltof,
        cls: KA,
        asm: "cvtsi2%k %L0, %=",
    },
    OmapEntry {
        op: Op::Cast,
        cls: KI,
        asm: "movq %D0, %L=",
    },
    OmapEntry {
        op: Op::Cast,
        cls: KA,
        asm: "movq %L0, %D=",
    },
    OmapEntry {
        op: Op::Addr,
        cls: KI,
        asm: "lea%k %M0, %=",
    },
    OmapEntry {
        op: Op::Swap,
        cls: KI,
        asm: "xchg%k %0, %1",
    },
    OmapEntry {
        op: Op::Sign,
        cls: Cls::Kl as i8,
        asm: "cqto",
    },
    OmapEntry {
        op: Op::Sign,
        cls: Cls::Kw as i8,
        asm: "cltd",
    },
    OmapEntry {
        op: Op::Xdiv,
        cls: KI,
        asm: "div%k %0",
    },
    OmapEntry {
        op: Op::Xidiv,
        cls: KI,
        asm: "idiv%k %0",
    },
    OmapEntry {
        op: Op::Xcmp,
        cls: Cls::Ks as i8,
        asm: "ucomiss %S0, %S1",
    },
    OmapEntry {
        op: Op::Xcmp,
        cls: Cls::Kd as i8,
        asm: "ucomisd %D0, %D1",
    },
    OmapEntry {
        op: Op::Xcmp,
        cls: KI,
        asm: "cmp%k %0, %1",
    },
    OmapEntry {
        op: Op::Xtest,
        cls: KI,
        asm: "test%k %0, %1",
    },
    OmapEntry {
        op: Op::Flagieq,
        cls: KI,
        asm: "setz %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagine,
        cls: KI,
        asm: "setnz %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagisge,
        cls: KI,
        asm: "setge %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagisgt,
        cls: KI,
        asm: "setg %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagisle,
        cls: KI,
        asm: "setle %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagislt,
        cls: KI,
        asm: "setl %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagiuge,
        cls: KI,
        asm: "setae %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagiugt,
        cls: KI,
        asm: "seta %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagiule,
        cls: KI,
        asm: "setbe %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagiult,
        cls: KI,
        asm: "setb %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagfeq,
        cls: KI,
        asm: "setz %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagfge,
        cls: KI,
        asm: "setae %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagfgt,
        cls: KI,
        asm: "seta %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagfle,
        cls: KI,
        asm: "setbe %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagflt,
        cls: KI,
        asm: "setb %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagfne,
        cls: KI,
        asm: "setnz %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagfo,
        cls: KI,
        asm: "setnp %B=\n\tmovzb%k %B=, %=",
    },
    OmapEntry {
        op: Op::Flagfuo,
        cls: KI,
        asm: "setp %B=\n\tmovzb%k %B=, %=",
    },
];

#[derive(Copy, Clone)]
enum Size {
    Long,
    Word,
    Short,
    Byte,
}

struct E<'a> {
    out: &'a mut String,
    f: &'a mut Fn,
    apple: bool,
    asloc: &'static str,
    assym: &'static str,
    literals: &'a mut crate::codegen::emission::FPStash,
    debug_files: &'a mut crate::codegen::emission::DbgFileState,
}

#[inline]
fn preg(r: u32) -> Ref {
    Ref::Tmp(TmpId(r))
}

fn slot(r: Ref, f: &Fn) -> i32 {
    let s = r.sval();
    assert!(s <= f.slot);
    if s < 0 {
        -4 * s
    } else if f.vararg {
        -176 + -4 * (f.slot - s)
    } else {
        -4 * (f.slot - s)
    }
}

fn regtoa(reg: u32, sz: Size) -> String {
    let idx = match sz {
        Size::Long => 0,
        Size::Word => 1,
        Size::Short => 2,
        Size::Byte => 3,
    };
    if reg >= XMM0 {
        format!("xmm{}", reg - XMM0)
    } else {
        let names = match reg {
            RAX => ["rax", "eax", "ax", "al"],
            RBX => ["rbx", "ebx", "bx", "bl"],
            RCX => ["rcx", "ecx", "cx", "cl"],
            RDX => ["rdx", "edx", "dx", "dl"],
            RSI => ["rsi", "esi", "si", "sil"],
            RDI => ["rdi", "edi", "di", "dil"],
            RBP => ["rbp", "ebp", "bp", "bpl"],
            RSP => ["rsp", "esp", "sp", "spl"],
            R8 => ["r8", "r8d", "r8w", "r8b"],
            R9 => ["r9", "r9d", "r9w", "r9b"],
            R10 => ["r10", "r10d", "r10w", "r10b"],
            R11 => ["r11", "r11d", "r11w", "r11b"],
            R12 => ["r12", "r12d", "r12w", "r12b"],
            R13 => ["r13", "r13d", "r13w", "r13b"],
            R14 => ["r14", "r14d", "r14w", "r14b"],
            R15 => ["r15", "r15d", "r15w", "r15b"],
            _ => panic!("invalid reg {}", reg),
        };
        names[idx].to_string()
    }
}

fn sym_name(f: &Fn, id: u32) -> &str {
    f.strs.get(id as usize).map(String::as_str).unwrap_or("???")
}

fn emitcon(con: &Con, e: &mut E<'_>) {
    match con.typ {
        ConType::Addr => {
            let l = sym_name(e.f, con.sym.id);
            let p = if l.starts_with('"') { "" } else { e.assym };
            if con.sym.typ == SymType::Thr {
                if e.apple {
                    let _ = write!(e.out, "{p}{l}@TLVP");
                } else {
                    let _ = write!(e.out, "%fs:{p}{l}@tpoff");
                }
            } else {
                let _ = write!(e.out, "{p}{l}");
            }
            if con.bits.i() != 0 {
                let _ = write!(e.out, "{:+}", con.bits.i());
            }
        }
        ConType::Bits => {
            let _ = write!(e.out, "{}", con.bits.i());
        }
        ConType::Undef => {}
    }
}

fn addr_con(r: Ref, f: &Fn) -> Option<Con> {
    match r {
        Ref::Con(id) => {
            let con = f.cons[id.0 as usize];
            (con.typ == ConType::Addr).then_some(con)
        }
        _ => None,
    }
}

fn emit_addr_to_reg(con: &Con, reg: u32, e: &mut E<'_>) {
    let sym = sym_name(e.f, con.sym.id);
    let p = if sym.starts_with('"') { "" } else { e.assym };
    let regn = regtoa(reg, Size::Long);
    if con.sym.typ == SymType::Thr {
        let _ = writeln!(e.out, "\tmovq %fs:0, %{regn}");
        let _ = write!(e.out, "\tleaq {p}{sym}@tpoff");
        if con.bits.i() != 0 {
            let _ = write!(e.out, "{:+}", con.bits.i());
        }
        let _ = writeln!(e.out, "(%{regn}), %{regn}");
        return;
    }
    if e.apple {
        let _ = writeln!(e.out, "\tleaq {p}{sym}(%rip), %{regn}");
    } else {
        let _ = writeln!(e.out, "\tmovq {p}{sym}@GOTPCREL(%rip), %{regn}");
    }
    if con.bits.i() != 0 {
        let _ = writeln!(e.out, "\taddq ${}, %{regn}", con.bits.i());
    }
}

fn getarg(c: char, i: &Ins) -> Ref {
    match c {
        '0' => i.arg[0],
        '1' => i.arg[1],
        '=' => i.to,
        _ => panic!("invalid arg letter {c}"),
    }
}

fn emit_ref(refv: Ref, size: Size, mem: bool, e: &mut E<'_>) {
    match refv {
        Ref::Tmp(id) => {
            let _ = if mem {
                write!(e.out, "(%{})", regtoa(id.0, Size::Long))
            } else {
                write!(e.out, "%{}", regtoa(id.0, size))
            };
        }
        Ref::Slot(_) => {
            let _ = write!(e.out, "{}(%rbp)", slot(refv, e.f));
        }
        Ref::Mem(id) => {
            let m = e.f.mems[id.0 as usize].clone();
            let mut off = m.offset;
            let mut base = m.base;
            let index = m.index;
            let scale = m.scale;
            if let Ref::Slot(_) = base {
                let off2 = Con {
                    typ: ConType::Bits,
                    sym: Sym::default(),
                    bits: ConBits::from_i64(slot(base, e.f) as i64),
                    flt: 0,
                };
                let _ = builder::addcon(&mut off, &off2);
                base = Ref::Tmp(TmpId(RBP));
            }
            if off.typ != ConType::Undef {
                emitcon(&off, e);
            }
            let _ = write!(e.out, "(");
            if !base.is_none() {
                let _ = write!(e.out, "%{}", regtoa(base.val(), Size::Long));
            } else if off.typ == ConType::Addr {
                let _ = write!(e.out, "%rip");
            }
            if !index.is_none() {
                let _ = write!(e.out, ", %{}, {}", regtoa(index.val(), Size::Long), scale);
            }
            let _ = write!(e.out, ")");
        }
        Ref::Con(id) => {
            let con = e.f.cons[id.0 as usize];
            if mem {
                emitcon(&con, e);
                if con.typ == ConType::Addr && (con.sym.typ != SymType::Thr || e.apple) {
                    let _ = write!(e.out, "(%rip)");
                }
            } else {
                let _ = write!(e.out, "$");
                emitcon(&con, e);
            }
        }
        _ => panic!("unreachable"),
    }
}

fn emitcopy(to: Ref, from: Ref, cls: Cls, e: &mut E<'_>) {
    emitins(
        Ins {
            op: Op::Copy,
            cls,
            to,
            arg: [from, Ref::R],
        },
        e,
    );
}

fn emitf(fmt: &str, ins: &Ins, e: &mut E<'_>) {
    let mut i = *ins;
    let mut s = fmt;
    if let Some(rest) = s.strip_prefix('+') {
        if i.arg[1] == i.to {
            i.arg.swap(0, 1);
        }
        s = rest;
    }
    if let Some(rest) = s.strip_prefix('-') {
        if i.arg[0] != i.to {
            emitcopy(i.to, i.arg[0], i.cls, e);
        }
        s = rest;
    }

    let clstoa = ["l", "q", "ss", "sd"];
    let _ = write!(e.out, "\t");
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            let _ = write!(e.out, "{c}");
            continue;
        }
        let Some(spec) = chars.next() else { break };
        match spec {
            '%' => {
                let _ = write!(e.out, "%");
            }
            'k' => {
                let _ = write!(e.out, "{}", clstoa[i.cls.as_index()]);
            }
            '0' | '1' | '=' => {
                let sz = if i.cls.is_wide() {
                    Size::Long
                } else {
                    Size::Word
                };
                emit_ref(getarg(spec, &i), sz, false, e);
            }
            'D' => emit_ref(getarg(chars.next().unwrap(), &i), Size::Long, false, e),
            'S' => emit_ref(getarg(chars.next().unwrap(), &i), Size::Long, false, e),
            'L' => emit_ref(getarg(chars.next().unwrap(), &i), Size::Long, false, e),
            'W' => emit_ref(getarg(chars.next().unwrap(), &i), Size::Word, false, e),
            'H' => emit_ref(getarg(chars.next().unwrap(), &i), Size::Short, false, e),
            'B' => emit_ref(getarg(chars.next().unwrap(), &i), Size::Byte, false, e),
            'M' => emit_ref(getarg(chars.next().unwrap(), &i), Size::Long, true, e),
            _ => panic!("invalid format specifier %{spec}"),
        }
    }
    let _ = writeln!(e.out);
}

fn omap_match(ins: &Ins) -> &'static str {
    for ent in OMAP {
        if ent.op != ins.op {
            continue;
        }
        if ent.cls == ins.cls as i8 || (ent.cls == KI && ins.cls.base() == 0) || ent.cls == KA {
            return ent.asm;
        }
    }
    panic!("no amd64 asm match for {:?}", ins.op);
}

fn negmask_index(cls: Cls, literals: &mut crate::codegen::emission::FPStash) -> usize {
    let bytes: [u8; 16] = match cls {
        Cls::Ks => [0, 0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        Cls::Kd => [0, 0, 0, 0, 0, 0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0],
        _ => panic!("bad neg mask class"),
    };
    literals.stash(&bytes)
}

fn emitins(mut i: Ins, e: &mut E<'_>) {
    match i.op {
        Op::Nop => {}
        Op::Mul => {
            if matches!(i.arg[1], Ref::Con(_)) {
                i.arg.swap(0, 1);
            }
            if i.cls.base() == 0
                && matches!(i.arg[0], Ref::Con(_))
                && matches!(i.arg[1], Ref::Tmp(_))
            {
                emitf("imul%k %0, %1, %=", &i, e);
            } else {
                emitf(omap_match(&i), &i, e);
            }
        }
        Op::Sub => {
            if i.to == i.arg[1] && i.arg[0] != i.to {
                emitins(
                    Ins {
                        op: Op::Neg,
                        cls: i.cls,
                        to: i.to,
                        arg: [i.to, Ref::R],
                    },
                    e,
                );
                emitf("add%k %0, %=", &i, e);
            } else {
                emitf(omap_match(&i), &i, e);
            }
        }
        Op::Neg => {
            if i.to != i.arg[0] {
                emitf("mov%k %0, %=", &i, e);
            }
            if i.cls.base() == 0 {
                emitf("neg%k %=", &i, e);
            } else {
                let n = negmask_index(i.cls, e.literals);
                let op = if i.cls == Cls::Ks { "xorps" } else { "xorpd" };
                let _ = writeln!(
                    e.out,
                    "\t{op} {}fp{}(%rip), %{}",
                    e.asloc,
                    n,
                    regtoa(i.to.val(), Size::Long)
                );
            }
        }
        Op::Div => {
            if i.to == i.arg[1] {
                i.arg[1] = preg(XMM15);
                emitf("mov%k %=, %1", &i, e);
                emitf("mov%k %0, %=", &i, e);
                i.arg[0] = i.to;
            }
            emitf(omap_match(&i), &i, e);
        }
        Op::Copy => {
            if i.to.is_none() || i.arg[0].is_none() || i.to == i.arg[0] {
                return;
            }
            if !e.apple
                && let Some(con) = addr_con(i.arg[0], e.f)
            {
                let dst = if builder::isreg(i.to) {
                    i.to.val()
                } else {
                    R11
                };
                emit_addr_to_reg(&con, dst, e);
                if !builder::isreg(i.to) {
                    emitcopy(i.to, preg(R11), i.cls, e);
                }
                return;
            }
            if i.cls == Cls::Kl
                && matches!(i.arg[0], Ref::Con(_))
                && let Ref::Con(id) = i.arg[0]
            {
                let c = e.f.cons[id.0 as usize];
                if c.typ == ConType::Bits {
                    let val = c.bits.i();
                    if builder::isreg(i.to) && (0..=u32::MAX as i64).contains(&val) {
                        emitf("movl %W0, %W=", &i, e);
                        return;
                    }
                    if matches!(i.to, Ref::Slot(_))
                        && (val < i32::MIN as i64 || val > i32::MAX as i64)
                    {
                        let base = slot(i.to, e.f);
                        let lo = (val as u64 & 0xffff_ffff) as u32 as i32;
                        let hi = ((val as u64 >> 32) & 0xffff_ffff) as u32 as i32;
                        let _ = writeln!(e.out, "\tmovl ${lo}, {base}(%rbp)");
                        let _ = writeln!(e.out, "\tmovl ${hi}, {}(%rbp)", base + 4);
                        return;
                    }
                }
            }
            if builder::isreg(i.to)
                && matches!(i.arg[0], Ref::Con(id) if e.f.cons[id.0 as usize].typ == ConType::Addr)
            {
                emitf("lea%k %M0, %=", &i, e);
                return;
            }
            if matches!(i.to, Ref::Slot(_)) && matches!(i.arg[0], Ref::Slot(_) | Ref::Mem(_)) {
                i.cls = if i.cls.is_wide() { Cls::Kd } else { Cls::Ks };
                i.arg[1] = preg(XMM15);
                emitf("mov%k %0, %1", &i, e);
                emitf("mov%k %1, %=", &i, e);
                return;
            }
            emitf("mov%k %0, %=", &i, e);
        }
        Op::Addr => {
            if !e.apple
                && let Some(con) = addr_con(i.arg[0], e.f)
            {
                emit_addr_to_reg(&con, i.to.val(), e);
                return;
            }
            emitf(omap_match(&i), &i, e);
        }
        op if op.is_load() => {
            if !e.apple
                && let Some(con) = addr_con(i.arg[0], e.f)
            {
                emit_addr_to_reg(&con, R11, e);
                i.arg[0] = preg(R11);
            }
            emitf(omap_match(&i), &i, e);
        }
        op if op.is_store() => {
            if !e.apple {
                if let Some(con) = addr_con(i.arg[0], e.f) {
                    emit_addr_to_reg(&con, R11, e);
                    i.arg[0] = preg(R11);
                }
                if let Some(con) = addr_con(i.arg[1], e.f) {
                    emit_addr_to_reg(&con, R11, e);
                    i.arg[1] = preg(R11);
                }
            }
            emitf(omap_match(&i), &i, e);
        }
        Op::Call => match i.arg[0] {
            Ref::Con(id) => {
                let con = e.f.cons[id.0 as usize];
                let _ = write!(e.out, "\tcallq ");
                emitcon(&con, e);
                let _ = writeln!(e.out);
            }
            Ref::Tmp(_) => emitf("callq *%L0", &i, e),
            _ => panic!("invalid call argument"),
        },
        Op::Salloc => {
            emitf("subq %L0, %%rsp", &i, e);
            if !i.to.is_none() {
                emitcopy(i.to, preg(RSP), Cls::Kl, e);
            }
        }
        Op::Swap => {
            if i.cls.base() == 0 {
                emitf(omap_match(&i), &i, e);
            } else {
                emitcopy(preg(XMM15), i.arg[0], i.cls, e);
                emitcopy(i.arg[0], i.arg[1], i.cls, e);
                emitcopy(i.arg[1], preg(XMM15), i.cls, e);
            }
        }
        Op::Dbgloc => {
            e.debug_files
                .emitdbgloc(i.arg[0].val(), i.arg[1].val(), e.out);
        }
        _ => emitf(omap_match(&i), &i, e),
    }
}

fn framesz(f: &Fn) -> u64 {
    let mut parity = 0u64;
    for &r in AMD64_SYSV_RCLOB {
        parity ^= (f.reg >> r) & 1;
    }
    let mut slots = f.slot as u64;
    slots = (slots + 3) & !3;
    4 * slots + 8 * parity + if f.vararg { 176 } else { 0 }
}

fn cmpneg_idx(c: u16) -> u16 {
    match c {
        0 => 1,
        1 => 0,
        2 => 4,
        3 => 5,
        4 => 2,
        5 => 3,
        6 => 8,
        7 => 9,
        8 => 6,
        9 => 7,
        10 => 15,
        11 => 14,
        12 => 13,
        13 => 12,
        14 => 11,
        15 => 10,
        16 => 17,
        17 => 16,
        _ => c,
    }
}

static CC_TOA: [&str; N_CMP] = [
    "z", "nz", "ge", "g", "le", "l", "ae", "a", "be", "b", "z", "ae", "a", "be", "b", "nz", "np",
    "p",
];

pub fn emitfn(
    f: &mut Fn,
    t: &Target,
    out: &mut String,
    state: &mut crate::codegen::emission::EmissionState,
) {
    let asloc = if t.apple { "L" } else { ".L" };
    let assym = if t.apple { "_" } else { "" };
    crate::codegen::emission::emitfnlnk_target(&f.name, &f.lnk, t, out);
    let _ = writeln!(out, "\tpushq %rbp");
    let _ = writeln!(out, "\tmovq %rsp, %rbp");

    let mut fs = framesz(f);
    if fs != 0 {
        let _ = writeln!(out, "\tsubq ${}, %rsp", fs);
    }
    if f.vararg {
        let mut o = -176;
        for &r in ARG_GPRS.iter() {
            let _ = writeln!(out, "\tmovq %{}, {}(%rbp)", regtoa(r, Size::Long), o);
            o += 8;
        }
        for n in 0..8 {
            let _ = writeln!(out, "\tmovaps %xmm{}, {}(%rbp)", n, o);
            o += 16;
        }
    }

    let id0 = state.next_block_label;
    let mut e = E {
        out,
        f,
        apple: t.apple,
        asloc,
        assym,
        literals: &mut state.floating_literals,
        debug_files: &mut state.debug_files,
    };
    for &r in AMD64_SYSV_RCLOB {
        if e.f.reg & super::registers::bit(r as u32) != 0 {
            emitf(
                "pushq %L0",
                &Ins {
                    op: Op::Nop,
                    cls: Cls::Kl,
                    to: Ref::R,
                    arg: [preg(r as u32), Ref::R],
                },
                &mut e,
            );
            fs += 8;
        }
    }

    let rpo = e.f.rpo.clone();
    let nblk = rpo.len();
    let mut lbl = false;

    for (idx, &bid) in rpo.iter().enumerate() {
        let blk = e.f.blks[bid.0 as usize].clone();
        if lbl || blk.pred.len() > 1 {
            let _ = writeln!(e.out, "{}bb{}:", e.asloc, id0 + blk.id);
        }
        for ins in blk.ins {
            emitins(ins, &mut e);
        }
        lbl = true;
        match blk.jmp.typ {
            Jmp::Hlt => {
                let _ = writeln!(e.out, "\tud2");
            }
            Jmp::Ret0 => {
                if e.f.dynalloc {
                    let _ = writeln!(e.out, "\tmovq %rbp, %rsp");
                    let _ = writeln!(e.out, "\tsubq ${}, %rsp", fs);
                }
                for &r in AMD64_SYSV_RCLOB.iter().rev() {
                    if e.f.reg & super::registers::bit(r as u32) != 0 {
                        emitf(
                            "popq %L0",
                            &Ins {
                                op: Op::Nop,
                                cls: Cls::Kl,
                                to: Ref::R,
                                arg: [preg(r as u32), Ref::R],
                            },
                            &mut e,
                        );
                    }
                }
                let _ = writeln!(e.out, "\tleave");
                let _ = writeln!(e.out, "\tret");
            }
            Jmp::Jmp_ => {
                let next = if idx + 1 < nblk {
                    Some(rpo[idx + 1])
                } else {
                    None
                };
                if blk.s1 != next {
                    let s1 = blk.s1.expect("jmp missing target");
                    let _ = writeln!(
                        e.out,
                        "\tjmp {}bb{}",
                        e.asloc,
                        id0 + e.f.blks[s1.0 as usize].id
                    );
                } else {
                    lbl = false;
                }
            }
            _ => {
                let mut c = blk
                    .jmp
                    .typ
                    .comparison_index()
                    .unwrap_or_else(|| panic!("unhandled jump {:?}", blk.jmp.typ));
                let next = if idx + 1 < nblk {
                    Some(rpo[idx + 1])
                } else {
                    None
                };
                let (mut s1, mut s2) = (blk.s1, blk.s2);
                if s2 == next {
                    std::mem::swap(&mut s1, &mut s2);
                } else {
                    c = cmpneg_idx(c);
                }
                if let Some(tgt) = s2 {
                    let _ = writeln!(
                        e.out,
                        "\tj{} {}bb{}",
                        CC_TOA[c as usize],
                        e.asloc,
                        id0 + e.f.blks[tgt.0 as usize].id
                    );
                }
                if s1 != next {
                    let tgt = s1.expect("cond jmp missing fallthrough target");
                    let _ = writeln!(
                        e.out,
                        "\tjmp {}bb{}",
                        e.asloc,
                        id0 + e.f.blks[tgt.0 as usize].id
                    );
                } else {
                    lbl = false;
                }
            }
        }
    }

    state.next_block_label = id0 + nblk as u32;
    if !t.apple {
        crate::codegen::emission::elf_emitfnfin(&e.f.name, e.out);
    }
}
