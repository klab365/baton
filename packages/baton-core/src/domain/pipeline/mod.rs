mod model;
mod structured_handoff;
pub(crate) mod transitions;

pub use model::{
    PipelineRun, RunEvent, RunTraceEntry, Status, StepRun, StructuredHandoffAttempt, TaskRunContext,
};
pub use structured_handoff::{
    Finding, FindingStatus, MAX_FINDINGS, MAX_STRUCTURED_HANDOFF_BYTES, StructuredHandoff,
    VerificationEvidence,
};
