//! Bounded decoders and immutable format values.

mod csf;

pub use csf::{CsfError, CsfFile, CsfHeader, CsfLabel, CsfLimits, CsfString, parse_csf};
