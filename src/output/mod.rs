//! Output storage and presentation.
mod emitter;
mod inspect;
mod sink;

pub use emitter::Emitter;
pub use inspect::pp_line;
pub use sink::{merge_batch, Batch, FinalSink, SinkTarget};
