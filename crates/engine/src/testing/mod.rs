//! Test support: a fake desktop for driving the real core without a
//! windowing system. Available to other crates with the `test-support`
//! feature.

pub mod fake_desktop;

pub use fake_desktop::{FakeDesktop, FakeDesktopWindow};
