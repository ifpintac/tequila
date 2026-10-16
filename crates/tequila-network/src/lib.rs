//! Network rendering (`TEQUILA_SPEC.md` §1.15): stills and configurator
//! combinations shared out across machines.
//!
//! Each render node runs `tequila --worker` with the farm's token. A
//! dispatcher - the application's Network window, or `tequila --farm` -
//! packs the project with every file it reads into a bundle
//! ([`bundle`]), queues a job per picture ([`farm`]), and each worker takes
//! the next job when it is free, renders it exactly as the command-line
//! renderer would, and sends the picture back ([`worker`]). Both ends prove
//! they hold the token and every message is signed ([`wire`]).
//!
//! The renderer itself is not in this crate: a worker is handed a function
//! that renders a job, which keeps this crate to the farm and the farm
//! testable without one.

pub mod bundle;
pub mod farm;
pub mod job;
pub mod wire;
pub mod worker;

pub use farm::{Farm, Job, JobState, JobStatus, WorkerState, WorkerStatus, DEFAULT_PORT};
pub use job::JobSpec;
