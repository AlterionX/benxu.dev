//! Representations of auth transfer between the [`blog_client`](../blog_client) and [`server`](../server).

use serde::{Serialize, Deserialize};

/// Password authentication data. Separated from AuthenticationData to allow for impl blocks. Will
/// go away once enum variants become types.
#[derive(Serialize, Deserialize)]
pub struct Password {
    pub user_name: String,
    pub password: String,
}

/// Actual data that needs to be verified before someone can log in.
/// Currently only allows for passwords, but planning to support SSO and FIDO.
#[derive(Serialize, Deserialize)]
pub enum Authentication {
    /// Data needed to fully specify a password credential from the request.
    Password(Password),
}