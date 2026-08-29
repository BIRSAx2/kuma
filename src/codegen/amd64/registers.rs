//! AMD64 register definitions and targets.

use crate::ir::internal::{Architecture, Op, Target};

#[inline]
pub const fn bit(n: u32) -> u64 {
    1u64 << n
}

pub const RXX: u32 = 0;

pub const RAX: u32 = RXX + 1;
pub const RCX: u32 = RAX + 1;
pub const RDX: u32 = RCX + 1;
pub const RSI: u32 = RDX + 1;
pub const RDI: u32 = RSI + 1;
pub const R8: u32 = RDI + 1;
pub const R9: u32 = R8 + 1;
pub const R10: u32 = R9 + 1;
pub const R11: u32 = R10 + 1;
pub const RBX: u32 = R11 + 1;
pub const R12: u32 = RBX + 1;
pub const R13: u32 = R12 + 1;
pub const R14: u32 = R13 + 1;
pub const R15: u32 = R14 + 1;
pub const RBP: u32 = R15 + 1;
pub const RSP: u32 = RBP + 1;

pub const XMM0: u32 = RSP + 1;
pub const XMM1: u32 = XMM0 + 1;
pub const XMM2: u32 = XMM1 + 1;
pub const XMM3: u32 = XMM2 + 1;
pub const XMM4: u32 = XMM3 + 1;
pub const XMM5: u32 = XMM4 + 1;
pub const XMM6: u32 = XMM5 + 1;
pub const XMM7: u32 = XMM6 + 1;
pub const XMM8: u32 = XMM7 + 1;
pub const XMM9: u32 = XMM8 + 1;
pub const XMM10: u32 = XMM9 + 1;
pub const XMM11: u32 = XMM10 + 1;
pub const XMM12: u32 = XMM11 + 1;
pub const XMM13: u32 = XMM12 + 1;
pub const XMM14: u32 = XMM13 + 1;
pub const XMM15: u32 = XMM14 + 1;

pub const NFPR: i32 = (XMM14 - XMM0 + 1) as i32;
pub const NGPR: i32 = (RSP - RAX + 1) as i32;
pub const NGPS: i32 = (R11 - RAX + 1) as i32;
pub const NFPS: i32 = NFPR;

pub const RGLOB: u64 = bit(RBP) | bit(RSP) | bit(R11);

pub static ARG_GPRS: [u32; 6] = [RDI, RSI, RDX, RCX, R8, R9];
pub static RET_GPRS: [u32; 2] = [RAX, RDX];
pub static RET_FPRS: [u32; 2] = [XMM0, XMM1];

pub static AMD64_SYSV_RSAVE: &[i32] = &[
    RDI as i32,
    RSI as i32,
    RDX as i32,
    RCX as i32,
    R8 as i32,
    R9 as i32,
    R10 as i32,
    R11 as i32,
    RAX as i32,
    XMM0 as i32,
    XMM1 as i32,
    XMM2 as i32,
    XMM3 as i32,
    XMM4 as i32,
    XMM5 as i32,
    XMM6 as i32,
    XMM7 as i32,
    XMM8 as i32,
    XMM9 as i32,
    XMM10 as i32,
    XMM11 as i32,
    XMM12 as i32,
    XMM13 as i32,
    XMM14 as i32,
];

pub static AMD64_SYSV_RCLOB: &[i32] = &[RBX as i32, R12 as i32, R13 as i32, R14 as i32, R15 as i32];

pub static T_AMD64_SYSV: Target = Target {
    architecture: Architecture::Amd64,
    apple: false,
    gpr0: RAX as i32,
    ngpr: NGPR,
    fpr0: XMM0 as i32,
    nfpr: NFPR,
    rglob: RGLOB,
    nrglob: 3,
    rsave: AMD64_SYSV_RSAVE,
    nrsave: [NGPS, NFPS],
};

pub static T_AMD64_APPLE: Target = Target {
    architecture: Architecture::Amd64,
    apple: true,
    gpr0: RAX as i32,
    ngpr: NGPR,
    fpr0: XMM0 as i32,
    nfpr: NFPR,
    rglob: RGLOB,
    nrglob: 3,
    rsave: AMD64_SYSV_RSAVE,
    nrsave: [NGPS, NFPS],
};

pub fn memargs(_op: Op) -> i32 {
    0
}
