#![feature(type_ascription)]

//! A collection of types and migrations for use with diesel and postgresql specifically for my
//! website.

#[cfg(feature = "diesel")]
#[macro_use]
extern crate diesel;

pub mod models;
#[cfg(not(feature = "diesel"))]
pub use models::{credentials, permissions, post_tag_junctions, posts, tags, users};

#[cfg(feature = "server")]
pub mod query;
#[cfg(feature = "rocket")]
pub mod rocket;
/// Auto generated by diesel. Reflects the database schema after applying all migrations in the
/// `migrations` folder.
#[cfg(feature = "diesel")]
pub mod schema;
