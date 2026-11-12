//! The C types and `errno` values the decoder names. `libc` defines them wherever there is
//! a C library; `wasm32-unknown-unknown` has none, so there they are the types of the
//! `wasm32` C ABI and the values of Linux's generic `errno.h`.

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use libc::{
    intptr_t, off_t, ptrdiff_t, uintptr_t, EAGAIN, EINVAL, EIO, ENOENT, ENOMEM, ENOPROTOOPT,
    ERANGE,
};

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use freestanding::*;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
mod freestanding {
    #![allow(non_camel_case_types)]

    use std::ffi::c_int;

    pub type intptr_t = isize;
    pub type uintptr_t = usize;
    pub type ptrdiff_t = isize;
    pub type off_t = i64;

    pub const ENOENT: c_int = 2;
    pub const EIO: c_int = 5;
    pub const EAGAIN: c_int = 11;
    pub const ENOMEM: c_int = 12;
    pub const EINVAL: c_int = 22;
    pub const ERANGE: c_int = 34;
    pub const ENOPROTOOPT: c_int = 92;
}
