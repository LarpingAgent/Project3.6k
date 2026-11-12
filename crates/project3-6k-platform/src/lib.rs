//! Native application lifecycle and input handling.

mod application;
mod input;

pub use application::{ApplicationError, RunOptions, run, run_with_options};
