//! What the service does, and the ports it needs somebody else to implement.
//!
//! Every port here is a trait, and every service takes those traits rather than
//! a concrete type. That is what lets a test exercise a rule without a database
//! and what keeps `infrastructure` depending on this crate rather than the
//! reverse.

pub mod events;
pub mod health;
pub mod inventory;
pub mod ports;
pub mod signups;
pub mod tasks;
