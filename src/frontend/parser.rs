#![allow(dead_code)]

//! IR tokenizer, parser, type checker, and printer.

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use crate::ir::builder::newcon;
use crate::ir::internal::{
    Blk, BlkId, Cls, Con, ConBits, ConType, Dat, DatItem, Field, FieldType, Fn, Ins, Jmp, Lnk,
    N_FIELD, N_INS, OP_TABLE, Op, Phi, Ref, SymType, TMP0, Tmp, TmpId, Typ, TypId,
};

/// Parsed IR declarations.
#[derive(Clone)]
pub struct ParseResult {
    /// Parsed type definitions.
    pub types: Vec<Typ>,
    /// Parsed data definitions (each inner vec is one data block:
    /// Start, items..., End).
    pub data: Vec<Vec<Dat>>,
    /// Parsed functions.
    pub functions: Vec<Fn>,
}

type PResult<T> = Result<T, super::ParseFailure>;

/// Token kinds produced by the lexer.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum Tok {
    Txxx,
    Top(Op),
    Tloadw,
    Tloadl,
    Tloads,
    Tloadd,
    Talloc1,
    Talloc2,
    Tblit,
    Tcall,
    Tenv,
    Tphi,
    Tjmp,
    Tjnz,
    Tret,
    Thlt,
    Texport,
    Tthread,
    Tfunc,
    Ttype,
    Tdata,
    Tsection,
    Talign,
    Tdbgfile,
    Tl,
    Tw,
    Tsh,
    Tuh,
    Th,
    Tsb,
    Tub,
    Tb,
    Td,
    Ts,
    Tz,
    Tint,
    Tflts,
    Tfltd,
    Ttmp,
    Tlbl,
    Tglo,
    Ttyp,
    Tstr,
    Tplus,
    Teq,
    Tcomma,
    Tlparen,
    Trparen,
    Tlbrace,
    Trbrace,
    Tnl,
    Tdots,
    Teof,
}

#[derive(Clone, Debug)]
struct TokVal {
    chr: char,
    fltd: f64,
    flts: f32,
    num: i64,
    str_val: String,
}

impl Default for TokVal {
    fn default() -> Self {
        Self {
            chr: '\0',
            fltd: 0.0,
            flts: 0.0,
            num: 0,
            str_val: String::new(),
        }
    }
}

const KSB: i32 = 4;
const KUB: i32 = 5;
const KSH: i32 = 6;
const KUH: i32 = 7;
const KC: i32 = 8;
const K0: i32 = 9;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum PState {
    Label,
    Phi,
    Instruction,
    End,
}

const N_PRED: usize = 63;

const SOURCE_OPERATIONS: &[Op] = &[
    Op::Add,
    Op::Sub,
    Op::Neg,
    Op::Div,
    Op::Rem,
    Op::Udiv,
    Op::Urem,
    Op::Mul,
    Op::And,
    Op::Or,
    Op::Xor,
    Op::Sar,
    Op::Shr,
    Op::Shl,
    Op::Ceqw,
    Op::Cnew,
    Op::Csgew,
    Op::Csgtw,
    Op::Cslew,
    Op::Csltw,
    Op::Cugew,
    Op::Cugtw,
    Op::Culew,
    Op::Cultw,
    Op::Ceql,
    Op::Cnel,
    Op::Csgel,
    Op::Csgtl,
    Op::Cslel,
    Op::Csltl,
    Op::Cugel,
    Op::Cugtl,
    Op::Culel,
    Op::Cultl,
    Op::Ceqs,
    Op::Cges,
    Op::Cgts,
    Op::Cles,
    Op::Clts,
    Op::Cnes,
    Op::Cos,
    Op::Cuos,
    Op::Ceqd,
    Op::Cged,
    Op::Cgtd,
    Op::Cled,
    Op::Cltd,
    Op::Cned,
    Op::Cod,
    Op::Cuod,
    Op::Storeb,
    Op::Storeh,
    Op::Storew,
    Op::Storel,
    Op::Stores,
    Op::Stored,
    Op::Loadsb,
    Op::Loadub,
    Op::Loadsh,
    Op::Loaduh,
    Op::Loadsw,
    Op::Loaduw,
    Op::Load,
    Op::Extsb,
    Op::Extub,
    Op::Extsh,
    Op::Extuh,
    Op::Extsw,
    Op::Extuw,
    Op::Exts,
    Op::Truncd,
    Op::Stosi,
    Op::Stoui,
    Op::Dtosi,
    Op::Dtoui,
    Op::Swtof,
    Op::Uwtof,
    Op::Sltof,
    Op::Ultof,
    Op::Cast,
    Op::Alloc4,
    Op::Alloc8,
    Op::Alloc16,
    Op::Vaarg,
    Op::Vastart,
    Op::Copy,
    Op::Dbgloc,
];

fn build_keyword_map() -> HashMap<String, Tok> {
    let mut m = HashMap::new();

    for &operation in SOURCE_OPERATIONS {
        let name = OP_TABLE[operation as usize].name;
        if !name.is_empty() && name != "xxx" {
            m.insert(name.to_string(), Tok::Top(operation));
        }
    }

    m.insert("loadw".into(), Tok::Tloadw);
    m.insert("loadl".into(), Tok::Tloadl);
    m.insert("loads".into(), Tok::Tloads);
    m.insert("loadd".into(), Tok::Tloadd);
    m.insert("alloc1".into(), Tok::Talloc1);
    m.insert("alloc2".into(), Tok::Talloc2);
    m.insert("blit".into(), Tok::Tblit);
    m.insert("call".into(), Tok::Tcall);
    m.insert("env".into(), Tok::Tenv);
    m.insert("phi".into(), Tok::Tphi);
    m.insert("jmp".into(), Tok::Tjmp);
    m.insert("jnz".into(), Tok::Tjnz);
    m.insert("ret".into(), Tok::Tret);
    m.insert("hlt".into(), Tok::Thlt);
    m.insert("export".into(), Tok::Texport);
    m.insert("thread".into(), Tok::Tthread);
    m.insert("function".into(), Tok::Tfunc);
    m.insert("type".into(), Tok::Ttype);
    m.insert("data".into(), Tok::Tdata);
    m.insert("section".into(), Tok::Tsection);
    m.insert("align".into(), Tok::Talign);
    m.insert("dbgfile".into(), Tok::Tdbgfile);
    m.insert("sb".into(), Tok::Tsb);
    m.insert("ub".into(), Tok::Tub);
    m.insert("sh".into(), Tok::Tsh);
    m.insert("uh".into(), Tok::Tuh);
    m.insert("b".into(), Tok::Tb);
    m.insert("h".into(), Tok::Th);
    m.insert("w".into(), Tok::Tw);
    m.insert("l".into(), Tok::Tl);
    m.insert("s".into(), Tok::Ts);
    m.insert("d".into(), Tok::Td);
    m.insert("z".into(), Tok::Tz);
    m.insert("...".into(), Tok::Tdots);

    m
}

fn subword_parameter_op(argument: bool, class: i32) -> Op {
    match (argument, class) {
        (false, KSB) => Op::Parsb,
        (false, KUB) => Op::Parub,
        (false, KSH) => Op::Parsh,
        (false, KUH) => Op::Paruh,
        (true, KSB) => Op::Argsb,
        (true, KUB) => Op::Argub,
        (true, KSH) => Op::Argsh,
        (true, KUH) => Op::Arguh,
        _ => unreachable!("sub-word parameter class was validated by parsecls"),
    }
}

struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
    lnum: u32,
    thead: Option<Tok>,
    tokval: TokVal,
    kwmap: HashMap<String, Tok>,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
            lnum: 1,
            thead: None,
            tokval: TokVal::default(),
            kwmap: build_keyword_map(),
        }
    }

    fn err<T>(&self, msg: &str) -> PResult<T> {
        Err(super::ParseFailure::at(
            self.input,
            self.pos,
            self.lnum,
            msg.to_owned(),
        ))
    }

    fn errf<T>(&self, msg: String) -> PResult<T> {
        Err(super::ParseFailure::at(
            self.input, self.pos, self.lnum, msg,
        ))
    }

    fn peekch(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn getch(&mut self) -> Option<u8> {
        let c = self.input.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn ungetch(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
        }
    }

    /// Read a (possibly negative) decimal integer.
    fn getint(&mut self) -> i64 {
        let mut n: u64 = 0;
        let c = self.getch().unwrap_or(b'0');
        let neg = c == b'-';
        let first = if neg || c == b'+' {
            self.getch().unwrap_or(b'0')
        } else {
            c
        };

        if first == b'0'
            && let Some(c2) = self.peekch()
            && (c2 == b'x' || c2 == b'X')
        {
            self.getch(); // consume 'x'
            loop {
                match self.peekch() {
                    Some(d @ b'0'..=b'9') => {
                        n = n.wrapping_mul(16).wrapping_add((d - b'0') as u64);
                        self.getch();
                    }
                    Some(d @ b'a'..=b'f') => {
                        n = n.wrapping_mul(16).wrapping_add((d - b'a' + 10) as u64);
                        self.getch();
                    }
                    Some(d @ b'A'..=b'F') => {
                        n = n.wrapping_mul(16).wrapping_add((d - b'A' + 10) as u64);
                        self.getch();
                    }
                    _ => break,
                }
            }
            if neg {
                n = 1u64.wrapping_add(!n);
            }
            return n as i64;
        }

        n = (first - b'0') as u64;
        while let Some(d @ b'0'..=b'9') = self.peekch() {
            n = 10u64.wrapping_mul(n).wrapping_add((d - b'0') as u64);
            self.getch();
        }
        if neg {
            n = 1u64.wrapping_add(!n);
        }
        n as i64
    }

    /// Core tokenizer. Returns the next token.
    fn lex(&mut self) -> PResult<Tok> {
        while let Some(b' ') | Some(b'\t') = self.peekch() {
            self.getch();
        }

        let c = match self.getch() {
            None => return Ok(Tok::Teof),
            Some(c) => c,
        };

        self.tokval.chr = c as char;

        match c {
            b',' => return Ok(Tok::Tcomma),
            b'(' => return Ok(Tok::Tlparen),
            b')' => return Ok(Tok::Trparen),
            b'{' => return Ok(Tok::Tlbrace),
            b'}' => return Ok(Tok::Trbrace),
            b'=' => return Ok(Tok::Teq),
            b'+' => return Ok(Tok::Tplus),
            b's' => {
                if self.peekch() == Some(b'_') {
                    self.getch(); // consume '_'
                    let start = self.pos;
                    while let Some(c2) = self.peekch() {
                        if c2.is_ascii_digit()
                            || c2 == b'.'
                            || c2 == b'-'
                            || c2 == b'+'
                            || c2 == b'e'
                            || c2 == b'E'
                        {
                            self.getch();
                        } else {
                            break;
                        }
                    }
                    let s = std::str::from_utf8(&self.input[start..self.pos]).unwrap_or("0");
                    if let Ok(v) = s.parse::<f32>() {
                        self.tokval.flts = v;
                        return Ok(Tok::Tflts);
                    }
                    self.pos = start - 1; // back to before '_'
                }
                return self.scan_ident(c, Tok::Txxx);
            }
            b'd' => {
                if self.peekch() == Some(b'_') {
                    self.getch(); // consume '_'
                    let start = self.pos;
                    while let Some(c2) = self.peekch() {
                        if c2.is_ascii_digit()
                            || c2 == b'.'
                            || c2 == b'-'
                            || c2 == b'+'
                            || c2 == b'e'
                            || c2 == b'E'
                        {
                            self.getch();
                        } else {
                            break;
                        }
                    }
                    let s = std::str::from_utf8(&self.input[start..self.pos]).unwrap_or("0");
                    if let Ok(v) = s.parse::<f64>() {
                        self.tokval.fltd = v;
                        return Ok(Tok::Tfltd);
                    }
                    self.pos = start - 1;
                }
                return self.scan_ident(c, Tok::Txxx);
            }
            b'%' => {
                let c2 = self.getch().unwrap_or(0);
                return self.scan_ident(c2, Tok::Ttmp);
            }
            b'@' => {
                let c2 = self.getch().unwrap_or(0);
                return self.scan_ident(c2, Tok::Tlbl);
            }
            b'$' => {
                let c2 = match self.getch() {
                    Some(b'"') => return self.scan_quoted_string(Tok::Tglo),
                    Some(c2) => c2,
                    None => return self.err("unexpected EOF after $"),
                };
                return self.scan_ident(c2, Tok::Tglo);
            }
            b':' => {
                let c2 = self.getch().unwrap_or(0);
                return self.scan_ident(c2, Tok::Ttyp);
            }
            b'#' => {
                loop {
                    match self.getch() {
                        Some(b'\n') | None => break,
                        _ => {}
                    }
                }
                self.lnum += 1;
                return Ok(Tok::Tnl);
            }
            b'\n' => {
                self.lnum += 1;
                return Ok(Tok::Tnl);
            }
            b'"' => {
                return self.scan_quoted_string(Tok::Tstr);
            }
            _ => {}
        }

        if c.is_ascii_digit() || c == b'-' {
            self.ungetch();
            self.tokval.num = self.getint();
            return Ok(Tok::Tint);
        }

        if c.is_ascii_alphabetic() || c == b'.' || c == b'_' {
            return self.scan_ident(c, Tok::Txxx);
        }

        self.errf(format!("invalid character '{}' ({})", c as char, c))
    }

    /// Scan a quoted string. The opening quote has already been consumed
    /// (or was the leading char for $ globals).
    fn scan_quoted_string(&mut self, t: Tok) -> PResult<Tok> {
        let mut s = String::new();
        let mut esc = false;
        loop {
            let c = match self.getch() {
                None => return self.err("unterminated string"),
                Some(c) => c,
            };
            if c == b'"' && !esc {
                break;
            }
            s.push(c as char);
            esc = c == b'\\' && !esc;
        }
        self.tokval.str_val = s;
        Ok(t)
    }

    /// Scan an identifier token. `first` is the first character already consumed.
    /// If `t` is not Txxx, the token type is forced (for sigil tokens: %, @, $, :).
    fn scan_ident(&mut self, first: u8, t: Tok) -> PResult<Tok> {
        if !first.is_ascii_alphanumeric() && first != b'.' && first != b'_' {
            return self.errf(format!("invalid character '{}' ({})", first as char, first));
        }
        let mut tok = vec![first];
        loop {
            match self.peekch() {
                Some(c) if c.is_ascii_alphanumeric() || c == b'$' || c == b'.' || c == b'_' => {
                    self.getch();
                    tok.push(c);
                }
                _ => break,
            }
        }
        let ident = String::from_utf8(tok).unwrap_or_default();
        self.tokval.str_val = ident.clone();

        if t != Tok::Txxx {
            return Ok(t);
        }

        if let Some(&kw) = self.kwmap.get(&ident) {
            return Ok(kw);
        }
        self.errf(format!("unknown keyword '{}'", ident))
    }

    fn peek(&mut self) -> PResult<Tok> {
        if self.thead.is_none() {
            self.thead = Some(self.lex()?);
        }
        Ok(self.thead.expect("token lookahead was just initialized"))
    }

    fn next(&mut self) -> PResult<Tok> {
        let t = self.peek()?;
        self.thead = None;
        Ok(t)
    }

    fn nextnl(&mut self) -> PResult<Tok> {
        loop {
            let t = self.next()?;
            if t != Tok::Tnl {
                return Ok(t);
            }
        }
    }

    fn expect(&mut self, t: Tok) -> PResult<()> {
        let t1 = self.next()?;
        if t == t1 {
            return Ok(());
        }
        let s1 = tok_name(t);
        let s2 = tok_name(t1);
        self.errf(format!("{} expected, got {} instead", s1, s2))
    }
}

fn tok_name(t: Tok) -> &'static str {
    match t {
        Tok::Tlbl => "label",
        Tok::Tcomma => ",",
        Tok::Teq => "=",
        Tok::Tnl => "newline",
        Tok::Tlparen => "(",
        Tok::Trparen => ")",
        Tok::Tlbrace => "{",
        Tok::Trbrace => "}",
        Tok::Teof => "end of file",
        _ => "??",
    }
}

struct Parser<'a> {
    lex: Lexer<'a>,
    /// Accumulated type definitions.
    typs: Vec<Typ>,
    /// Current function being parsed.
    curf: Fn,
    /// Current block.
    curb: Option<usize>, // index into curf.blks
    /// Block link list (indices of blocks in order).
    blk_order: Vec<usize>,
    /// Block name → index in curf.blks.
    blk_map: HashMap<String, usize>,
    /// Temp name → TmpId for current function.
    tmp_map: HashMap<String, TmpId>,
    /// Current instruction buffer for current block.
    insb: Vec<Ins>,
    /// Return class for current function.
    rcls: i32,
    /// Phi link: index of next phi to add to current block.
    nblk: u32,
    /// Debug file names.
    dbgfiles: Vec<String>,
    /// Symbol name table (global across all functions).
    sym_strs: Vec<String>,
    /// Symbol name dedup map.
    sym_map: HashMap<String, u32>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            lex: Lexer::new(input),
            typs: Vec::new(),
            curf: Fn::default(),
            curb: None,
            blk_order: Vec::new(),
            blk_map: HashMap::new(),
            tmp_map: HashMap::new(),
            insb: Vec::new(),
            rcls: K0,
            nblk: 0,
            dbgfiles: Vec::new(),
            sym_strs: Vec::new(),
            sym_map: HashMap::new(),
        }
    }

    fn err<T>(&self, msg: &str) -> PResult<T> {
        self.lex.err(msg)
    }

    fn errf<T>(&self, msg: String) -> PResult<T> {
        self.lex.errf(msg)
    }

    /// Find or create a temporary by name.
    fn tmpref(&mut self, name: &str) -> Ref {
        if let Some(&id) = self.tmp_map.get(name) {
            return Ref::Tmp(id);
        }
        let t = self.curf.tmps.len() as u32;
        let id = TmpId(t);
        self.tmp_map.insert(name.to_string(), id);
        let tmp = Tmp {
            name: name.to_string(),
            cls: Cls::Kx,
            ..Tmp::default()
        };
        self.curf.tmps.push(tmp);
        Ref::Tmp(id)
    }

    /// Parse a reference (tmp, int, float, global).
    fn parseref(&mut self) -> PResult<Ref> {
        let mut c = Con::default();
        match self.lex.next()? {
            Tok::Ttmp => {
                let s = self.lex.tokval.str_val.clone();
                return Ok(self.tmpref(&s));
            }
            Tok::Tint => {
                c.typ = ConType::Bits;
                c.bits = ConBits::from_i64(self.lex.tokval.num);
            }
            Tok::Tflts => {
                c.typ = ConType::Bits;
                c.bits = ConBits::from_f32(self.lex.tokval.flts);
                c.flt = 1;
            }
            Tok::Tfltd => {
                c.typ = ConType::Bits;
                c.bits = ConBits::from_f64(self.lex.tokval.fltd);
                c.flt = 2;
            }
            Tok::Tthread => {
                c.sym.typ = SymType::Thr;
                self.lex.expect(Tok::Tglo)?;
                c.typ = ConType::Addr;
                c.sym.id = self.intern_sym(&self.lex.tokval.str_val.clone());
            }
            Tok::Tglo => {
                c.typ = ConType::Addr;
                c.sym.id = self.intern_sym(&self.lex.tokval.str_val.clone());
            }
            _ => return Ok(Ref::R),
        }
        Ok(newcon(&c, &mut self.curf))
    }

    /// Intern a global symbol name. Returns an id that can be used to look up
    /// the name in `Fn.strs`.
    fn intern_sym(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.sym_map.get(name) {
            return id;
        }
        let id = self.sym_strs.len() as u32;
        self.sym_strs.push(name.to_owned());
        self.sym_map.insert(name.to_owned(), id);
        id
    }

    /// Find type by name among parsed types, searching backwards.
    fn findtyp(&self, limit: usize) -> PResult<usize> {
        let name = self.lex.tokval.str_val.clone();
        for i in (0..limit).rev() {
            if self.typs[i].name == name {
                return Ok(i);
            }
        }
        self.errf(format!("undefined type :{}", name))
    }

    /// Parse a class specifier (w, l, s, d, sb, ub, sh, uh, :type).
    /// Returns (class_int, type_index). The type_index is only meaningful
    /// when class == KC.
    fn parsecls(&mut self) -> PResult<(i32, i32)> {
        let class = match self.lex.next()? {
            Tok::Ttyp => {
                let tyn = self.findtyp(self.typs.len())? as i32;
                (KC, tyn)
            }
            Tok::Tsb => (KSB, -1),
            Tok::Tub => (KUB, -1),
            Tok::Tsh => (KSH, -1),
            Tok::Tuh => (KUH, -1),
            Tok::Tw => (Cls::Kw as i32, -1),
            Tok::Tl => (Cls::Kl as i32, -1),
            Tok::Ts => (Cls::Ks as i32, -1),
            Tok::Td => (Cls::Kd as i32, -1),
            _ => return self.err("invalid class specifier"),
        };
        Ok(class)
    }

    /// Parse a reference list (function params or call args).
    /// If `arg` is true, we're parsing call arguments.
    /// If `arg` is false, we're parsing function parameters.
    /// Returns true if the function is variadic.
    fn parserefl(&mut self, arg: bool) -> PResult<bool> {
        let mut hasenv = false;
        let mut vararg = false;
        self.lex.expect(Tok::Tlparen)?;

        while self.lex.peek()? != Tok::Trparen {
            if self.insb.len() >= N_INS {
                return self.err("too many instructions");
            }
            if !arg && vararg {
                return self.err("no parameters allowed after '...'");
            }

            match self.lex.peek()? {
                Tok::Tdots => {
                    if vararg {
                        return self.err("only one '...' allowed");
                    }
                    vararg = true;
                    if arg {
                        self.insb.push(Ins {
                            op: Op::Argv,
                            cls: Cls::Kx,
                            to: Ref::R,
                            arg: [Ref::R, Ref::R],
                        });
                    }
                    self.lex.next()?;
                    if self.lex.peek()? == Tok::Trparen {
                        break;
                    }
                    self.lex.expect(Tok::Tcomma)?;
                    continue;
                }
                Tok::Tenv => {
                    if hasenv {
                        return self.err("only one environment allowed");
                    }
                    hasenv = true;
                    self.lex.next()?;
                    let k = Cls::Kl as i32;
                    let r = self.parseref()?;
                    if r == Ref::R {
                        return self.err("invalid argument");
                    }
                    if !arg && !matches!(r, Ref::Tmp(_)) {
                        return self.err("invalid function parameter");
                    }
                    let ins = if arg {
                        Ins {
                            op: Op::Arge,
                            cls: Cls::from_i8(k as i8),
                            to: Ref::R,
                            arg: [r, Ref::R],
                        }
                    } else {
                        Ins {
                            op: Op::Pare,
                            cls: Cls::from_i8(k as i8),
                            to: r,
                            arg: [Ref::R, Ref::R],
                        }
                    };
                    self.insb.push(ins);
                }
                _ => {
                    let (k, ty) = self.parsecls()?;
                    let r = self.parseref()?;
                    if r == Ref::R {
                        return self.err("invalid argument");
                    }
                    if !arg && !matches!(r, Ref::Tmp(_)) {
                        return self.err("invalid function parameter");
                    }
                    let ins = if k == KC {
                        if arg {
                            Ins {
                                op: Op::Argc,
                                cls: Cls::Kl,
                                to: Ref::R,
                                arg: [Ref::Typ(TypId(ty as u32)), r],
                            }
                        } else {
                            Ins {
                                op: Op::Parc,
                                cls: Cls::Kl,
                                to: r,
                                arg: [Ref::Typ(TypId(ty as u32)), Ref::R],
                            }
                        }
                    } else if k >= KSB {
                        if arg {
                            Ins {
                                op: subword_parameter_op(true, k),
                                cls: Cls::Kw,
                                to: Ref::R,
                                arg: [r, Ref::R],
                            }
                        } else {
                            Ins {
                                op: subword_parameter_op(false, k),
                                cls: Cls::Kw,
                                to: r,
                                arg: [Ref::R, Ref::R],
                            }
                        }
                    } else {
                        let cls = Cls::from_i8(k as i8);
                        if arg {
                            Ins {
                                op: Op::Arg,
                                cls,
                                to: Ref::R,
                                arg: [r, Ref::R],
                            }
                        } else {
                            Ins {
                                op: Op::Par,
                                cls,
                                to: r,
                                arg: [Ref::R, Ref::R],
                            }
                        }
                    };
                    self.insb.push(ins);
                }
            }

            if self.lex.peek()? == Tok::Trparen {
                break;
            }
            self.lex.expect(Tok::Tcomma)?;
        }
        self.lex.expect(Tok::Trparen)?;
        Ok(vararg)
    }

    /// Find or create a block by name.
    fn findblk(&mut self, name: &str) -> usize {
        if let Some(&idx) = self.blk_map.get(name) {
            return idx;
        }
        let idx = self.curf.blks.len();
        let blk = Blk {
            id: self.nblk,
            name: name.to_string(),
            ..Blk::default()
        };
        self.nblk += 1;
        self.curf.blks.push(blk);
        self.blk_map.insert(name.to_string(), idx);
        idx
    }

    /// Close the current block: flush instruction buffer.
    fn closeblk(&mut self) {
        if let Some(idx) = self.curb {
            self.curf.blks[idx].ins = self.insb.drain(..).collect();
            self.blk_order.push(idx);
        }
    }

    /// Parse a single line inside a function body.
    fn parseline(&mut self, ps: PState) -> PResult<PState> {
        let t = self.lex.nextnl()?;

        if ps == PState::Label && t != Tok::Tlbl && t != Tok::Trbrace {
            return self.err("label or } expected");
        }

        if t == Tok::Tlbl {
            let name = self.lex.tokval.str_val.clone();
            let b = self.findblk(&name);

            if let Some(cur_idx) = self.curb
                && self.curf.blks[cur_idx].jmp.typ == Jmp::Jxxx
            {
                self.closeblk();
                self.curf.blks[cur_idx].jmp.typ = Jmp::Jmp_;
                self.curf.blks[cur_idx].s1 = Some(BlkId(b as u32));
            }
            if self.curf.blks[b].jmp.typ != Jmp::Jxxx {
                return self.errf(format!("multiple definitions of block @{}", name));
            }
            self.curb = Some(b);
            self.lex.expect(Tok::Tnl)?;
            return Ok(PState::Phi);
        }

        if t == Tok::Trbrace {
            return Ok(PState::End);
        }

        if t == Tok::Tret {
            let Some(cur) = self.curb else {
                return self.err("instruction outside block");
            };
            self.curf.blks[cur].jmp.typ = ret_jmp(self.rcls);
            if self.lex.peek()? == Tok::Tnl {
                self.curf.blks[cur].jmp.typ = Jmp::Ret0;
            } else if self.rcls != K0 {
                let r = self.parseref()?;
                if r == Ref::R {
                    return self.err("invalid return value");
                }
                self.curf.blks[cur].jmp.arg = r;
            }
            self.lex.expect(Tok::Tnl)?;
            self.closeblk();
            return Ok(PState::Label);
        }

        if t == Tok::Tjmp {
            let Some(cur) = self.curb else {
                return self.err("instruction outside block");
            };
            self.curf.blks[cur].jmp.typ = Jmp::Jmp_;
            self.lex.expect(Tok::Tlbl)?;
            let name = self.lex.tokval.str_val.clone();
            let s1 = self.findblk(&name);
            self.curf.blks[cur].s1 = Some(BlkId(s1 as u32));
            self.lex.expect(Tok::Tnl)?;
            self.closeblk();
            return Ok(PState::Label);
        }

        if t == Tok::Tjnz {
            let Some(cur) = self.curb else {
                return self.err("instruction outside block");
            };
            self.curf.blks[cur].jmp.typ = Jmp::Jnz;
            let r = self.parseref()?;
            if r == Ref::R {
                return self.err("invalid argument for jnz jump");
            }
            self.curf.blks[cur].jmp.arg = r;
            self.lex.expect(Tok::Tcomma)?;
            self.lex.expect(Tok::Tlbl)?;
            let name1 = self.lex.tokval.str_val.clone();
            let s1 = self.findblk(&name1);
            self.curf.blks[cur].s1 = Some(BlkId(s1 as u32));
            self.lex.expect(Tok::Tcomma)?;
            self.lex.expect(Tok::Tlbl)?;
            let name2 = self.lex.tokval.str_val.clone();
            let s2 = self.findblk(&name2);
            self.curf.blks[cur].s2 = Some(BlkId(s2 as u32));
            if s1 == 0 || s2 == 0 {
                let start_id = self.curf.start;
                if self.curf.blks[s1].id == start_id.0 || self.curf.blks[s2].id == start_id.0 {
                    return self.err("invalid jump to the start block");
                }
            }
            self.lex.expect(Tok::Tnl)?;
            self.closeblk();
            return Ok(PState::Label);
        }

        if t == Tok::Thlt {
            let Some(cur) = self.curb else {
                return self.err("instruction outside block");
            };
            self.curf.blks[cur].jmp.typ = Jmp::Hlt;
            self.lex.expect(Tok::Tnl)?;
            self.closeblk();
            return Ok(PState::Label);
        }

        if t == Tok::Top(Op::Dbgloc) {
            if self.curb.is_none() {
                return self.err("instruction outside block");
            }
            self.lex.expect(Tok::Tint)?;
            let line = self.lex.tokval.num;
            let arg0 = Ref::Int(line as i32);
            let arg1 = if self.lex.peek()? == Tok::Tcomma {
                self.lex.next()?;
                self.lex.expect(Tok::Tint)?;
                Ref::Int(self.lex.tokval.num as i32)
            } else {
                Ref::Int(0)
            };
            self.insb.push(Ins {
                op: Op::Dbgloc,
                cls: Cls::Kw,
                to: Ref::R,
                arg: [arg0, arg1],
            });
            self.lex.expect(Tok::Tnl)?;
            return Ok(PState::Instruction);
        }

        let (r, k, ty, op) = if t == Tok::Ttmp {
            let name = self.lex.tokval.str_val.clone();
            let r = self.tmpref(&name);
            self.lex.expect(Tok::Teq)?;
            let (k, ty) = self.parsecls()?;
            let op = self.lex.next()?;
            (r, k, ty, op)
        } else {
            let is_store_tok = is_store_token(t);
            if is_store_tok || t == Tok::Tblit || t == Tok::Tcall || t == Tok::Top(Op::Vastart) {
                (Ref::R, Cls::Kw as i32, -1i32, t)
            } else {
                return self.err("label, instruction or jump expected");
            }
        };

        if self.curb.is_none() {
            return self.err("instruction outside block");
        }

        if op == Tok::Tcall {
            let arg0 = self.parseref()?;
            self.parserefl(true)?;
            let (call_k, arg1) = if k == KC {
                (Cls::Kl, Ref::Typ(TypId(ty as u32)))
            } else if k >= KSB {
                (Cls::Kw, Ref::R)
            } else {
                (Cls::from_i8(k as i8), Ref::R)
            };
            self.lex.expect(Tok::Tnl)?;
            self.insb.push(Ins {
                op: Op::Call,
                cls: call_k,
                to: r,
                arg: [arg0, arg1],
            });
            return Ok(PState::Instruction);
        }

        let mut real_op = match op {
            Tok::Top(o) => o,
            Tok::Tloadw => Op::Loadsw,
            Tok::Tloadl | Tok::Tloads | Tok::Tloadd => Op::Load,
            Tok::Talloc1 | Tok::Talloc2 => Op::Alloc4,
            Tok::Tblit => Op::Blit0, // handled specially below
            Tok::Tphi => Op::Oxxx,   // handled specially below
            _ => return self.err("invalid instruction"),
        };

        if real_op == Op::Vastart && !self.curf.vararg {
            return self.err("cannot use vastart in non-variadic function");
        }

        if k >= KSB && op != Tok::Tphi && op != Tok::Tblit {
            return self.err("size class must be w, l, s, or d");
        }

        let mut args: Vec<Ref> = Vec::new();
        let mut blks: Vec<String> = Vec::new();

        if self.lex.peek()? != Tok::Tnl {
            loop {
                if args.len() >= N_PRED {
                    return self.err("too many arguments");
                }
                if op == Tok::Tphi {
                    self.lex.expect(Tok::Tlbl)?;
                    blks.push(self.lex.tokval.str_val.clone());
                }
                let a = self.parseref()?;
                if a == Ref::R {
                    return self.err("invalid instruction argument");
                }
                args.push(a);
                let t2 = self.lex.peek()?;
                if t2 == Tok::Tnl {
                    break;
                }
                if t2 != Tok::Tcomma {
                    return self.err(", or end of line expected");
                }
                self.lex.next()?;
            }
        }
        self.lex.next()?; // consume the newline

        if op == Tok::Tphi {
            if ps != PState::Phi {
                return self.err("unexpected phi instruction");
            }
            let cur = self.curb.unwrap();
            if self.curf.blks[cur].id == self.curf.start.0 {
                return self.err("unexpected phi instruction");
            }
            let phi_blks: Vec<BlkId> = blks
                .iter()
                .map(|name| {
                    let idx = self.findblk(name);
                    BlkId(self.curf.blks[idx].id)
                })
                .collect();
            let phi = Phi {
                to: r,
                cls: Cls::from_i8(k as i8),
                args,
                blks: phi_blks,
            };
            self.curf.blks[cur].phi.push(phi);
            return Ok(PState::Phi);
        }

        if op == Tok::Tblit {
            if args.len() < 3 {
                return self.err("blit requires 3 arguments");
            }
            self.insb.push(Ins {
                op: Op::Blit0,
                cls: Cls::Kx,
                to: Ref::R,
                arg: [args[0], args[1]],
            });
            let size_ref = args[2];
            match size_ref {
                Ref::Con(cid) => {
                    let c = &self.curf.cons[cid.0 as usize];
                    let sz = c.bits.i();
                    if c.typ != ConType::Bits || sz < 0 {
                        return self.err("invalid blit size");
                    }
                    self.insb.push(Ins {
                        op: Op::Blit1,
                        cls: Cls::Kx,
                        to: Ref::R,
                        arg: [Ref::Int(sz as i32), Ref::R],
                    });
                }
                _ => return self.err("blit size must be constant"),
            }
            return Ok(PState::Instruction);
        }

        if let Tok::Top(o) = op {
            real_op = o;
        }

        let cls = Cls::from_i8(k as i8);
        self.insb.push(Ins {
            op: real_op,
            cls,
            to: r,
            arg: [
                args.first().copied().unwrap_or(Ref::R),
                args.get(1).copied().unwrap_or(Ref::R),
            ],
        });
        Ok(PState::Instruction)
    }

    fn typecheck(&self) -> PResult<()> {
        let f = &self.curf;

        let mut tmp_cls: Vec<Cls> = vec![Cls::Kx; f.tmps.len()];

        for b in &f.blks {
            for p in &b.phi {
                if let Ref::Tmp(id) = p.to {
                    tmp_cls[id.0 as usize] = p.cls;
                }
            }
            for i in &b.ins {
                if let Ref::Tmp(id) = i.to {
                    let t = &mut tmp_cls[id.0 as usize];
                    if *t != Cls::Kx && *t != i.cls {
                        return self.errf(format!(
                            "temporary %{} is assigned with multiple types",
                            f.tmps[id.0 as usize].name
                        ));
                    }
                    *t = i.cls;
                }
            }
        }
        Ok(())
    }

    fn parsefn(&mut self, lnk: Lnk) -> PResult<()> {
        self.curb = None;
        self.nblk = 0;
        self.insb.clear();
        self.blk_map.clear();
        self.blk_order.clear();
        self.tmp_map.clear();

        self.curf = Fn::default();

        self.curf.cons.push(Con {
            typ: ConType::Bits,
            bits: ConBits::from_i64(0xdeaddead),
            ..Con::default()
        });
        self.curf.cons.push(Con {
            typ: ConType::Bits,
            bits: ConBits::from_i64(0),
            ..Con::default()
        });

        for _i in 0..TMP0 {
            self.curf.tmps.push(Tmp {
                cls: Cls::Kl,
                ..Tmp::default()
            });
        }

        self.curf.lnk = lnk;
        self.curf.retty = -1;

        if self.lex.peek()? != Tok::Tglo {
            let (cls, ty) = self.parsecls()?;
            self.rcls = cls;
            self.curf.retty = ty;
        } else {
            self.rcls = K0;
        }

        if self.lex.next()? != Tok::Tglo {
            return self.err("function name expected");
        }
        self.curf.name = self.lex.tokval.str_val.clone();

        self.curf.vararg = self.parserefl(false)?;

        if self.lex.nextnl()? != Tok::Tlbrace {
            return self.err("function body must start with {");
        }

        let mut ps = PState::Label;
        loop {
            ps = self.parseline(ps)?;
            if ps == PState::End {
                break;
            }
        }

        if self.curb.is_none() {
            return self.err("empty function");
        }
        if let Some(cur) = self.curb
            && self.curf.blks[cur].jmp.typ == Jmp::Jxxx
        {
            return self.err("last block misses jump");
        }

        if !self.blk_order.is_empty() {
            self.curf.start = BlkId(self.curf.blks[self.blk_order[0]].id);
        }

        self.typecheck()?;
        Ok(())
    }

    fn parsefields(
        &mut self,
        fields: &mut Vec<Field>,
        ty: &mut TypStub,
        t_init: Tok,
    ) -> PResult<()> {
        let mut sz: u64 = 0;
        let mut al = ty.align;
        let mut t = t_init;

        while t != Tok::Trbrace {
            let (ftype, s, a) = match t {
                Tok::Td => (FieldType::Fd, 8u64, 3i32),
                Tok::Tl => (FieldType::Fl, 8, 3),
                Tok::Ts => (FieldType::Fs, 4, 2),
                Tok::Tw => (FieldType::Fw, 4, 2),
                Tok::Th => (FieldType::Fh, 2, 1),
                Tok::Tb => (FieldType::Fb, 1, 0),
                Tok::Ttyp => {
                    let idx = self.findtyp(self.typs.len())?;
                    let ty1 = &self.typs[idx];
                    let s = ty1.size;
                    let a = ty1.align;
                    (FieldType::FTyp, s, a)
                }
                _ => return self.err("invalid type member specifier"),
            };

            let typ_len = if ftype == FieldType::FTyp {
                let idx = self.findtyp(self.typs.len())?;
                idx as u32
            } else {
                s as u32
            };

            if a > al {
                al = a;
            }
            let align_mask = (1u64 << a) - 1;
            let padding = ((sz + align_mask) & !align_mask) - sz;
            if padding > 0 && fields.len() < N_FIELD {
                fields.push(Field {
                    typ: FieldType::FPad,
                    len: padding as u32,
                });
            }

            t = self.lex.nextnl()?;
            let count = if t == Tok::Tint {
                let c = self.lex.tokval.num;
                t = self.lex.nextnl()?;
                c as u64
            } else {
                1
            };

            sz += padding + count * s;

            let fld_len = if ftype == FieldType::FTyp {
                typ_len
            } else {
                s as u32
            };
            for _ in 0..count {
                if fields.len() < N_FIELD {
                    fields.push(Field {
                        typ: ftype,
                        len: fld_len,
                    });
                }
            }

            if t != Tok::Tcomma {
                break;
            }
            t = self.lex.nextnl()?;
        }

        if t != Tok::Trbrace {
            return self.err(", or } expected");
        }

        fields.push(Field {
            typ: FieldType::End,
            len: 0,
        });

        let a_val = 1u64 << al;
        if sz < ty.size {
            sz = ty.size;
        }
        ty.size = (sz + a_val - 1) & !(a_val - 1);
        ty.align = al;
        Ok(())
    }

    fn parsetyp(&mut self) -> PResult<()> {
        if self.lex.nextnl()? != Tok::Ttyp {
            return self.err("type name expected");
        }
        let name = self.lex.tokval.str_val.clone();
        if self.lex.nextnl()? != Tok::Teq {
            return self.err("= expected after type name");
        }

        let mut stub = TypStub {
            is_dark: false,
            is_union: false,
            align: -1,
            size: 0,
        };

        let mut t = self.lex.nextnl()?;
        if t == Tok::Talign {
            if self.lex.nextnl()? != Tok::Tint {
                return self.err("alignment expected");
            }
            let mut val = self.lex.tokval.num;
            let mut al = 0i32;
            while val > 1 {
                val /= 2;
                al += 1;
            }
            stub.align = al;
            t = self.lex.nextnl()?;
        }

        if t != Tok::Tlbrace {
            return self.err("type body must start with {");
        }

        t = self.lex.nextnl()?;
        if t == Tok::Tint {
            stub.is_dark = true;
            stub.size = self.lex.tokval.num as u64;
            if stub.align == -1 {
                return self.err("dark types need alignment");
            }
            if self.lex.nextnl()? != Tok::Trbrace {
                return self.err("} expected");
            }

            self.typs.push(Typ {
                name,
                is_dark: true,
                is_union: false,
                align: stub.align,
                size: stub.size,
                nunion: 0,
                fields: Vec::new(),
            });
            return Ok(());
        }

        let mut all_fields: Vec<Vec<Field>> = Vec::new();

        if t == Tok::Tlbrace {
            stub.is_union = true;
            loop {
                if t != Tok::Tlbrace {
                    return self.err("invalid union member");
                }
                let mut flds = Vec::new();
                let inner_t = self.lex.nextnl()?;
                self.parsefields(&mut flds, &mut stub, inner_t)?;
                all_fields.push(flds);
                t = self.lex.nextnl()?;
                if t == Tok::Trbrace {
                    break;
                }
            }
        } else {
            let mut flds = Vec::new();
            self.parsefields(&mut flds, &mut stub, t)?;
            all_fields.push(flds);
        }

        self.typs.push(Typ {
            name,
            is_dark: stub.is_dark,
            is_union: stub.is_union,
            align: stub.align,
            size: stub.size,
            nunion: all_fields.len() as u32,
            fields: all_fields,
        });
        Ok(())
    }

    fn parsedat(&mut self, lnk: Lnk) -> PResult<Vec<Dat>> {
        let mut items: Vec<Dat> = Vec::new();

        if self.lex.nextnl()? != Tok::Tglo {
            return self.err("data name expected");
        }
        let name = self.lex.tokval.str_val.clone();
        if self.lex.nextnl()? != Tok::Teq {
            return self.err("= expected after data name");
        }

        let mut data_lnk = lnk;
        let mut t = self.lex.nextnl()?;
        data_lnk.align = 8;
        if t == Tok::Talign {
            if self.lex.nextnl()? != Tok::Tint {
                return self.err("alignment expected");
            }
            data_lnk.align = self.lex.tokval.num as u8;
            t = self.lex.nextnl()?;
        }

        items.push(Dat {
            item: DatItem::Start,
            name: Some(name),
            lnk: Some(data_lnk),
        });

        if t != Tok::Tlbrace {
            return self.err("expected data contents in { .. }");
        }

        loop {
            let dt = self.lex.nextnl()?;
            if dt == Tok::Trbrace {
                break;
            }

            let is_zero = dt == Tok::Tz;

            t = self.lex.nextnl()?;
            loop {
                let item = match t {
                    Tok::Tflts => DatItem::FltS(self.lex.tokval.flts),
                    Tok::Tfltd => DatItem::FltD(self.lex.tokval.fltd),
                    Tok::Tint => {
                        if is_zero {
                            DatItem::Zero(self.lex.tokval.num as u64)
                        } else {
                            match dt {
                                Tok::Tb => DatItem::Byte(self.lex.tokval.num),
                                Tok::Th => DatItem::Half(self.lex.tokval.num),
                                Tok::Tw | Tok::Ts => DatItem::Word(self.lex.tokval.num),
                                Tok::Tl | Tok::Td => DatItem::Long(self.lex.tokval.num),
                                _ => DatItem::Long(self.lex.tokval.num),
                            }
                        }
                    }
                    Tok::Tglo => {
                        let ref_name = self.lex.tokval.str_val.clone();
                        let mut off = 0i64;
                        if self.lex.peek()? == Tok::Tplus {
                            self.lex.next()?;
                            if self.lex.next()? != Tok::Tint {
                                return self.err("invalid token after offset in ref");
                            }
                            off = self.lex.tokval.num;
                        }
                        DatItem::Ref {
                            name: ref_name,
                            off,
                        }
                    }
                    Tok::Tstr => DatItem::Str(self.lex.tokval.str_val.clone()),
                    _ => return self.err("constant literal expected"),
                };

                items.push(Dat {
                    item,
                    name: None,
                    lnk: None,
                });

                t = self.lex.nextnl()?;
                if t == Tok::Tint
                    || t == Tok::Tflts
                    || t == Tok::Tfltd
                    || t == Tok::Tstr
                    || t == Tok::Tglo
                {
                    continue;
                }
                break;
            }

            if t == Tok::Trbrace {
                break;
            }
            if t != Tok::Tcomma {
                return self.err(", or } expected");
            }
        }

        items.push(Dat {
            item: DatItem::End,
            name: None,
            lnk: None,
        });

        Ok(items)
    }

    fn parselnk(&mut self) -> PResult<(Lnk, Tok)> {
        let mut lnk = Lnk::default();
        let mut haslnk = false;

        loop {
            let t = self.lex.nextnl()?;
            match t {
                Tok::Texport => {
                    lnk.export = true;
                    haslnk = true;
                }
                Tok::Tthread => {
                    lnk.thread = true;
                    haslnk = true;
                }
                Tok::Tsection => {
                    if lnk.sec.is_some() {
                        return self.err("only one section allowed");
                    }
                    if self.lex.next()? != Tok::Tstr {
                        return self.err("section \"name\" expected");
                    }
                    lnk.sec = Some(self.lex.tokval.str_val.clone());
                    if self.lex.peek()? == Tok::Tstr {
                        self.lex.next()?;
                        lnk.secf = Some(self.lex.tokval.str_val.clone());
                    }
                    haslnk = true;
                }
                _ => {
                    if t == Tok::Tfunc && lnk.thread {
                        return self.err("only data may have thread linkage");
                    }
                    if haslnk && t != Tok::Tdata && t != Tok::Tfunc {
                        return self.err("only data and function have linkage");
                    }
                    return Ok((lnk, t));
                }
            }
        }
    }

    fn parse_all(&mut self) -> PResult<ParseResult> {
        let mut fns = Vec::new();
        let mut dats = Vec::new();

        loop {
            let (lnk, t) = self.parselnk()?;
            match t {
                Tok::Tdbgfile => {
                    self.lex.expect(Tok::Tstr)?;
                    self.dbgfiles.push(self.lex.tokval.str_val.clone());
                }
                Tok::Tfunc => {
                    self.parsefn(lnk)?;
                    self.curf.strs = self.sym_strs.clone();
                    fns.push(self.curf.clone());
                }
                Tok::Tdata => {
                    let dat = self.parsedat(lnk)?;
                    dats.push(dat);
                }
                Tok::Ttype => {
                    self.parsetyp()?;
                }
                Tok::Teof => {
                    break;
                }
                _ => {
                    return self.err("top-level definition expected");
                }
            }
        }

        Ok(ParseResult {
            types: self.typs.clone(),
            data: dats,
            functions: fns,
        })
    }
}

/// Helper struct for building Typ during parsing.
struct TypStub {
    is_dark: bool,
    is_union: bool,
    align: i32,
    size: u64,
}

fn is_store_token(t: Tok) -> bool {
    match t {
        Tok::Top(op) => op.is_store(),
        _ => false,
    }
}

/// Map rcls to return Jmp variant.
fn ret_jmp(rcls: i32) -> Jmp {
    match rcls {
        x if x == Cls::Kw as i32 => Jmp::Retw,
        x if x == Cls::Kl as i32 => Jmp::Retl,
        x if x == Cls::Ks as i32 => Jmp::Rets,
        x if x == Cls::Kd as i32 => Jmp::Retd,
        x if x == KSB => Jmp::Retsb,
        x if x == KUB => Jmp::Retub,
        x if x == KSH => Jmp::Retsh,
        x if x == KUH => Jmp::Retuh,
        x if x == KC => Jmp::Retc,
        x if x == K0 => Jmp::Ret0,
        _ => Jmp::Ret0,
    }
}

/// Print a constant in IR syntax.
fn printcon(c: &Con) -> String {
    match c.typ {
        ConType::Undef => String::new(),
        ConType::Addr => {
            let mut s = String::new();
            if c.sym.typ == SymType::Thr {
                s.push_str("thread ");
            }
            write!(s, "${}", c.sym.id).unwrap();
            if c.bits.i() != 0 {
                write!(s, "{:+}", c.bits.i()).unwrap();
            }
            s
        }
        ConType::Bits => {
            if c.flt == 1 {
                format!("s_{}", c.bits.s())
            } else if c.flt == 2 {
                format!("d_{}", c.bits.d())
            } else {
                format!("{}", c.bits.i())
            }
        }
    }
}

/// Print a reference in IR syntax.
pub fn printref(r: Ref, f: &Fn, typs: &[Typ]) -> String {
    match r {
        Ref::Tmp(id) => {
            if id.0 < TMP0 {
                format!("R{}", id.0)
            } else {
                format!("%{}", f.tmps[id.0 as usize].name)
            }
        }
        Ref::Con(id) => {
            if r == Ref::UNDEF {
                "UNDEF".to_string()
            } else {
                printcon(&f.cons[id.0 as usize])
            }
        }
        Ref::Slot(_v) => {
            format!("S{}", r.sval())
        }
        Ref::Call(v) => {
            format!("{:04x}", v)
        }
        Ref::Typ(id) => {
            if (id.0 as usize) < typs.len() {
                format!(":{}", typs[id.0 as usize].name)
            } else {
                format!(":T{}", id.0)
            }
        }
        Ref::Mem(id) => {
            let m = &f.mems[id.0 as usize];
            let mut s = String::from("[");
            let mut has_part = false;
            if m.offset.typ != ConType::Undef {
                s.push_str(&printcon(&m.offset));
                has_part = true;
            }
            if m.base != Ref::R {
                if has_part {
                    s.push_str(" + ");
                }
                s.push_str(&printref(m.base, f, typs));
                has_part = true;
            }
            if m.index != Ref::R {
                if has_part {
                    s.push_str(" + ");
                }
                write!(s, "{} * ", m.scale).unwrap();
                s.push_str(&printref(m.index, f, typs));
            }
            s.push(']');
            s
        }
        Ref::Int(_v) => {
            format!("{}", r.sval())
        }
        Ref::R => String::new(),
    }
}

/// Print a function in IR syntax for debugging.
pub fn printfn(f: &Fn, typs: &[Typ]) -> String {
    let ktoc = ['w', 'l', 's', 'd'];
    let mut out = String::new();

    writeln!(out, "function ${}() {{", f.name).unwrap();

    for b in &f.blks {
        writeln!(out, "@{}", b.name).unwrap();

        for p in &b.phi {
            write!(out, "\t{}", printref(p.to, f, typs)).unwrap();
            let cls_idx = p.cls as i8;
            let cls_ch = if cls_idx >= 0 && (cls_idx as usize) < ktoc.len() {
                ktoc[cls_idx as usize]
            } else {
                '?'
            };
            write!(out, " ={} phi ", cls_ch).unwrap();
            for (n, (arg, blk_id)) in p.args.iter().zip(p.blks.iter()).enumerate() {
                let blk_name = f
                    .blks
                    .iter()
                    .find(|blk| blk.id == blk_id.0)
                    .map(|blk| blk.name.as_str())
                    .unwrap_or("??");
                write!(out, "@{} {}", blk_name, printref(*arg, f, typs)).unwrap();
                if n < p.args.len() - 1 {
                    write!(out, ", ").unwrap();
                }
            }
            writeln!(out).unwrap();
        }

        for i in &b.ins {
            write!(out, "\t").unwrap();
            if i.to != Ref::R {
                let cls_idx = i.cls as i8;
                let cls_ch = if cls_idx >= 0 && (cls_idx as usize) < ktoc.len() {
                    ktoc[cls_idx as usize]
                } else {
                    '?'
                };
                write!(out, "{} ={} ", printref(i.to, f, typs), cls_ch).unwrap();
            }
            let opname = OP_TABLE[i.op as usize].name;
            write!(out, "{}", opname).unwrap();

            if i.to == Ref::R {
                match i.op {
                    Op::Arg
                    | Op::Swap
                    | Op::Xcmp
                    | Op::Acmp
                    | Op::Acmn
                    | Op::Afcmp
                    | Op::Xtest
                    | Op::Xdiv
                    | Op::Xidiv => {
                        let cls_idx = i.cls as i8;
                        if cls_idx >= 0 && (cls_idx as usize) < ktoc.len() {
                            write!(out, "{}", ktoc[cls_idx as usize]).unwrap();
                        }
                    }
                    _ => {}
                }
            }

            if i.arg[0] != Ref::R {
                write!(out, " {}", printref(i.arg[0], f, typs)).unwrap();
            }
            if i.arg[1] != Ref::R {
                write!(out, ", {}", printref(i.arg[1], f, typs)).unwrap();
            }
            writeln!(out).unwrap();
        }

        match b.jmp.typ {
            Jmp::Ret0
            | Jmp::Retw
            | Jmp::Retl
            | Jmp::Rets
            | Jmp::Retd
            | Jmp::Retsb
            | Jmp::Retub
            | Jmp::Retsh
            | Jmp::Retuh
            | Jmp::Retc => {
                let jname = jmp_name(b.jmp.typ);
                write!(out, "\t{}", jname).unwrap();
                if b.jmp.typ != Jmp::Ret0 || b.jmp.arg != Ref::R {
                    write!(out, " {}", printref(b.jmp.arg, f, typs)).unwrap();
                }
                if b.jmp.typ == Jmp::Retc && f.retty >= 0 && (f.retty as usize) < typs.len() {
                    write!(out, ", :{}", typs[f.retty as usize].name).unwrap();
                }
                writeln!(out).unwrap();
            }
            Jmp::Hlt => {
                writeln!(out, "\thlt").unwrap();
            }
            Jmp::Jmp_ => {
                if let Some(s1_id) = b.s1 {
                    let s1_name = f
                        .blks
                        .iter()
                        .find(|blk| blk.id == s1_id.0)
                        .map(|blk| blk.name.as_str())
                        .unwrap_or("??");
                    writeln!(out, "\tjmp @{}", s1_name).unwrap();
                }
            }
            Jmp::Jnz => {
                write!(out, "\tjnz {}", printref(b.jmp.arg, f, typs)).unwrap();
                let s1_name =
                    b.s1.and_then(|id| {
                        f.blks
                            .iter()
                            .find(|blk| blk.id == id.0)
                            .map(|blk| blk.name.as_str())
                    })
                    .unwrap_or("??");
                let s2_name =
                    b.s2.and_then(|id| {
                        f.blks
                            .iter()
                            .find(|blk| blk.id == id.0)
                            .map(|blk| blk.name.as_str())
                    })
                    .unwrap_or("??");
                writeln!(out, ", @{}, @{}", s1_name, s2_name).unwrap();
            }
            Jmp::Jxxx => {}
            _ => {
                let jname = jmp_name(b.jmp.typ);
                write!(out, "\t{} ", jname).unwrap();
                let s1_name =
                    b.s1.and_then(|id| {
                        f.blks
                            .iter()
                            .find(|blk| blk.id == id.0)
                            .map(|blk| blk.name.as_str())
                    })
                    .unwrap_or("??");
                let s2_name =
                    b.s2.and_then(|id| {
                        f.blks
                            .iter()
                            .find(|blk| blk.id == id.0)
                            .map(|blk| blk.name.as_str())
                    })
                    .unwrap_or("??");
                writeln!(out, "@{}, @{}", s1_name, s2_name).unwrap();
            }
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

/// Get the string name for a Jmp variant.
fn jmp_name(j: Jmp) -> &'static str {
    match j {
        Jmp::Jxxx => "xxx",
        Jmp::Retw => "retw",
        Jmp::Retl => "retl",
        Jmp::Rets => "rets",
        Jmp::Retd => "retd",
        Jmp::Retsb => "retsb",
        Jmp::Retub => "retub",
        Jmp::Retsh => "retsh",
        Jmp::Retuh => "retuh",
        Jmp::Retc => "retc",
        Jmp::Ret0 => "ret",
        Jmp::Jmp_ => "jmp",
        Jmp::Jnz => "jnz",
        Jmp::Jfieq => "jfieq",
        Jmp::Jfine => "jfine",
        Jmp::Jfisge => "jfisge",
        Jmp::Jfisgt => "jfisgt",
        Jmp::Jfisle => "jfisle",
        Jmp::Jfislt => "jfislt",
        Jmp::Jfiuge => "jfiuge",
        Jmp::Jfiugt => "jfiugt",
        Jmp::Jfiule => "jfiule",
        Jmp::Jfiult => "jfiult",
        Jmp::Jffeq => "jffeq",
        Jmp::Jffge => "jffge",
        Jmp::Jffgt => "jffgt",
        Jmp::Jffle => "jffle",
        Jmp::Jfflt => "jfflt",
        Jmp::Jffne => "jffne",
        Jmp::Jffo => "jffo",
        Jmp::Jffuo => "jffuo",
        Jmp::Hlt => "hlt",
    }
}

/// Parse IR text into types, data blocks, and functions.
pub fn parse(input: &str) -> PResult<ParseResult> {
    let mut parser = Parser::new(input);
    parser.parse_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty() {
        let result = parse("").expect("empty input");
        assert!(result.types.is_empty());
        assert!(result.data.is_empty());
        assert!(result.functions.is_empty());
    }

    #[test]
    fn test_parse_type() {
        let input = "type :pair = { w, l }\n";
        let result = parse(input).expect("type");
        assert_eq!(result.types.len(), 1);
        assert_eq!(result.types[0].name, "pair");
        assert!(!result.types[0].is_dark);
        assert!(!result.types[0].is_union);
    }

    #[test]
    fn test_parse_data() {
        let input = "data $str = { b \"hello\", b 0 }\n";
        let result = parse(input).expect("data");
        assert_eq!(result.data.len(), 1);
        assert!(result.data[0].len() >= 3);
    }

    #[test]
    fn test_parse_function() {
        let input = "\
function w $add(w %a, w %b) {
@start
    %c =w add %a, %b
    ret %c
}
";
        let result = parse(input).expect("function");
        assert_eq!(result.functions.len(), 1);
        assert_eq!(result.functions[0].name, "add");
    }

    #[test]
    fn test_parse_void_function() {
        let input = "\
function $nop() {
@start
    ret
}
";
        let result = parse(input).expect("void function");
        assert_eq!(result.functions.len(), 1);
        assert_eq!(result.functions[0].name, "nop");
    }

    #[test]
    fn test_printfn_roundtrip() {
        let input = "\
function w $add(w %a, w %b) {
@start
    %c =w add %a, %b
    ret %c
}
";
        let result = parse(input).expect("round trip input");
        let f = &result.functions[0];
        let text = printfn(f, &result.types);
        assert!(text.contains("add"));
        assert!(text.contains("ret"));
    }
}
