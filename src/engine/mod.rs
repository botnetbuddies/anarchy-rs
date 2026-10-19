//! Candidate generation and work scheduling.
mod estimate;
mod generate;
mod pipeline;
mod preview;

pub use estimate::{estimate, estimate_auto};
pub use generate::{gen_auto, gen_people, AutoNames};
pub use pipeline::{run_parallel, run_serial, Work};
pub use preview::{all_outputs, GenRet};

#[cfg(test)]
mod tests;

pub struct GenOpts {
    pub substitute: bool,
    pub max_sub: i64,
    pub verbose: bool,
    pub case_insensitive: bool,
    pub suffix: Option<Vec<u8>>,
}

impl GenOpts {
    #[inline]
    fn limit(&self, total: u64) -> u64 {
        if self.max_sub < 0 {
            total
        } else {
            total.min(self.max_sub as u64 + 1)
        }
    }
}

/// Scratch buffers reused across people and plugins.
pub struct Scratch {
    pub resolved: Vec<u8>,
    pub tmp: Vec<u8>,
    pub chars: Vec<u8>,
}

impl Scratch {
    pub fn new() -> Scratch {
        Scratch {
            resolved: Vec::with_capacity(64),
            tmp: Vec::with_capacity(256),
            chars: Vec::with_capacity(256),
        }
    }
}
