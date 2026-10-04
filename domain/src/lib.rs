//! The rules, and nothing that talks to the outside world.
//!
//! Every type here either holds a value the rules accept or cannot be built at
//! all. That is the whole design: a function taking a [`tasks::TaskTitle`] needs
//! no guard clause, because an untrimmed or empty title has no representation.

pub mod errors;
pub mod events;
pub mod health;
pub mod inventory;
pub mod signups;
pub mod tasks;
pub mod text;
