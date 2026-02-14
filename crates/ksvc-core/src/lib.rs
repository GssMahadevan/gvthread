//! # ksvc-core — Trait definitions for KSVC
//!
//! This crate defines the trait boundaries for every axis of variability
//! in the KSVC system. Each trait models one capability dimension.
//! Default (safe) implementations exist for every trait.
//! Optimized implementations are behind feature flags or runtime detection.
//!
//! ## Design principle
//!
//! > "Program to the interface. Start safe. Optimize with a new impl,
//! >  not by modifying the existing one."
//!
//! Every component of KSVC depends on traits from this crate, never on
//! concrete types. Swapping implementations is a one-line type alias change
//! or a runtime `Box<dyn Trait>` swap.

pub mod entry;
pub mod completion;
pub mod tier;
pub mod router;
pub mod io_backend;
pub mod worker;
pub mod notifier;
pub mod buffer;
pub mod shared_page;
pub mod error;
