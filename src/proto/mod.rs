pub mod pb {
    #![allow(clippy::empty_docs)]
    include!("gen/rafn/v1/rafn.v1.rs");
}

pub mod benchmark;
pub mod error;

pub use error::{Error, Result};
