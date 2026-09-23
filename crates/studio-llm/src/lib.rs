//! Outbound inference infrastructure. It never reads project datasets or writes artifacts.
pub mod credentials;
mod protocols;
mod providers;
mod runtime;
mod transport;
pub use runtime::RemoteLlm;
mod recorded;
