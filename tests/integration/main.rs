//! Integration tests entry point, following https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html

mod common;
pub use common::*;

mod blocker_project_resolution;
mod sync;
