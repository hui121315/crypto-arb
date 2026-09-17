//! Futures module data layer.
//!
//! Split into focused submodules to satisfy the module-size gate:
//! - [`model`]: `FuturesOpportunity` value object and its derivations.
//! - [`runtime`]: constants, filters, runtime handles and store factories.
//! - [`list`]: list polling resource + stream patching helpers.
//! - [`search`]: symbol search hook, chips, row filtering and summaries.

mod list;
mod model;
mod runtime;
mod search;

#[cfg(test)]
mod tests;

pub(in crate::panels::modules::futures) use list::*;
pub(in crate::panels::modules::futures) use model::*;
pub(in crate::panels::modules::futures) use runtime::*;
pub(in crate::panels::modules::futures) use search::*;

pub(in crate::panels) use runtime::create_futures_runtime;
pub(in crate::panels) use runtime::FuturesRuntime;
