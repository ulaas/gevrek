//! Test support: a backend in memory, for driving the real core without a
//! desktop. Available to other crates with the `test-support` feature.

pub mod fake_backend;

pub use fake_backend::FakeBackend;
