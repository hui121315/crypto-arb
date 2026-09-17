//! Futures mapper: DTO -> view-row conversion.
//!
//! Split to satisfy the module-size gate: production conversion in
//! [`conversion`]; test fixtures/helpers/cases under [`tests`].

mod conversion;

pub(in crate::panels::modules::futures) use conversion::to_futures_opps_from_list_views;
