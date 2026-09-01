//! Native Windows PowerShell and PowerShell Core execution support.

pub mod adapters;
pub mod catalog;
pub mod manifest;
pub mod orchestrator;
pub mod parser;
pub mod transport;

#[cfg(test)]
mod tests;
