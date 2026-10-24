//! Outbound adapters for Baton's domain ports.
mod chatgpt;
mod chatgpt_provider;
mod fake;
mod openai;
mod rig_support;

pub use chatgpt::ChatGptRigRuntime;
pub use chatgpt_provider::{ChatGptSession, DeviceCode};
pub use fake::FakeRuntime;
pub use openai::OpenAiRigRuntime;
