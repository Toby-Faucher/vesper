#![forbid(unsafe_code)]
//! Audio lab for the Vesper capture path: a seeded corpus of noisy mixtures, DNSMOS and intrusive
//! metrics, and the `audiolab` CLI that scores `vesper_dsp::Pipeline` on it.

pub mod audio;
pub mod corpus;
pub mod dnsmos;
pub mod eval;
pub mod metrics;
