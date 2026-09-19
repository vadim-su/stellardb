#[cfg(feature = "auth")]
pub mod api_key;
pub mod authorization;
#[cfg(feature = "auth")]
pub mod error;
#[cfg(feature = "auth")]
pub mod jwt;
#[cfg(feature = "server")]
pub mod middleware;
#[cfg(feature = "auth")]
pub mod password;
#[cfg(feature = "auth")]
pub mod policy_engine;
#[cfg(feature = "auth")]
pub mod service;
pub mod types;

pub use authorization::{AuthorizationError, AuthorizationProvider, authorize};
#[cfg(feature = "auth")]
pub use error::AuthError;
#[cfg(feature = "auth")]
pub use service::AuthService;
pub use types::*;
