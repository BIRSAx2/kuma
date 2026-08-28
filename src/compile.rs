//! Compilation pipeline.

use std::fmt;

use crate::ir::{Fn, Target, Typ};
use crate::parse::{self, ParseResult};
use crate::{
    alias, amd64, arm64, cfg, copy, emit, fold, live, load, mem, regalloc, simpl, spill, ssa,
};

fn is_amd64_target(target: &Target) -> bool {
    target.name.starts_with("amd64")
}

/// Compilation error.
#[derive(Debug)]
pub enum Error {
    /// Error during parsing.
    Parse(String),
    /// Error during compilation.
    Compile(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(msg) => write!(f, "parse error: {msg}"),
            Error::Compile(msg) => write!(f, "compile error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

/// Compile IR text to assembly for the given target.
pub fn compile(input: &str, target: &Target) -> Result<String, Error> {
    let mut out = String::new();

    let ParseResult {
        types,
        data,
        functions,
    } = parse::parse(input);

    for data_group in &data {
        let mut dat_state = emit::DatState::new();
        for dat in data_group {
            emit::emitdat(dat, &mut dat_state, Some(target), &mut out);
        }
    }

    for mut f in functions {
        compile_fn(&mut f, target, &types, &mut out);
    }

    emit::with_fp_stash(|stash| emit::emitfin(stash, target, &mut out));

    Ok(out)
}

fn compile_fn(f: &mut Fn, target: &Target, typs: &[Typ], out: &mut String) {
    if is_amd64_target(target) {
        amd64::abi0(f, target);
    } else {
        arm64::abi0(f, target);
    }

    cfg::fillrpo(f);
    cfg::fillpreds(f);
    ssa::filluse(f);

    mem::promote(f);
    ssa::filluse(f);

    ssa::ssa(f);
    ssa::filluse(f);
    ssa::ssacheck(f);

    alias::fillalias(f);
    load::loadopt(f);
    ssa::filluse(f);

    alias::fillalias(f);
    mem::coalesce(f);
    ssa::filluse(f);
    ssa::ssacheck(f);

    copy::copy(f);
    ssa::filluse(f);

    fold::fold(f);

    if is_amd64_target(target) {
        amd64::abi1(f, target, typs);
    } else {
        arm64::abi1(f, target, typs);
    }

    simpl::simpl(f);

    cfg::fillpreds(f);
    ssa::filluse(f);

    if is_amd64_target(target) {
        amd64::isel(f, target);
    } else {
        arm64::isel(f, target);
    }

    cfg::fillrpo(f);
    live::filllive(f, target);
    cfg::fillloop(f);
    spill::fillcost(f);

    spill::spill(f, target);
    regalloc::rega(f, target);

    cfg::fillrpo(f);
    cfg::simpljmp(f);
    cfg::fillpreds(f);
    cfg::fillrpo(f);

    assert!(!f.rpo.is_empty(), "function must have at least one block");
    debug_assert!(
        f.rpo[0] == f.start,
        "first RPO block must be the entry block"
    );
    if is_amd64_target(target) {
        amd64::emitfn(f, target, out);
    } else {
        arm64::emitfn(f, target, out);
    }
}
