//! Opportunities module data layer.
//!
//! Split into focused submodules to satisfy the module-size gate:
//! - [`runtime`]: signals, filters, runtime handles and store factories.
//! - [`list`]: list polling resource and first-page cache lifecycle.
//! - [`search`]: symbol search hook, row filtering and summaries.
//! - [`detail`]/[`detail_assembly`]/[`detail_sections`]/[`detail_format`]:
//!   opportunity detail fetch, segment assembly, section capture, formatting.
//! - [`detail_seed`]: detail seed value object.

mod detail;
mod detail_assembly;
mod detail_evidence;
mod detail_format;
mod detail_sections;
mod detail_seed;
mod list;
mod runtime;
mod search;
mod webhook;

#[cfg(test)]
mod tests;

pub(in crate::panels::modules::opportunities) use detail::*;
pub(in crate::panels::modules::opportunities) use detail_assembly::*;
pub(in crate::panels::modules::opportunities) use detail_evidence::*;
pub(in crate::panels::modules::opportunities) use detail_format::*;
pub(in crate::panels::modules::opportunities) use detail_sections::*;
pub(in crate::panels::modules::opportunities) use detail_seed::*;
pub(in crate::panels::modules::opportunities) use list::*;
pub(in crate::panels::modules::opportunities) use runtime::*;
pub(in crate::panels::modules::opportunities) use search::*;
pub(in crate::panels::modules::opportunities) use webhook::*;

pub(in crate::panels) use runtime::create_opportunities_runtime;
pub(in crate::panels) use runtime::OpportunitiesRuntime;
