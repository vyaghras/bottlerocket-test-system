/*!

This library provides the Kubernetes custom resource definitions and their API clients.

!*/

pub mod model;
mod test_client;

pub use test_client::Error;
pub use test_client::TestClient;
