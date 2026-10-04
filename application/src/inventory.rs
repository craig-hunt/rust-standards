//! Answering a stock query.

use crate::ports::{InventoryStore, StoreResult};
use domain::inventory::{InventoryQuery, InventoryResult};

/// The store reads; the query decides what to show and in what order.
pub struct InventoryService<S> {
    store: S,
}

impl<S: InventoryStore> InventoryService<S> {
    /// Wires the service to a store.
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    /// The rows the query matches, in the order it asked for.
    ///
    /// # Errors
    ///
    /// Returns whatever the store reported.
    pub fn query(&self, query: &InventoryQuery) -> StoreResult<InventoryResult> {
        Ok(query.apply(&self.store.items()?))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::InventoryService;
    use crate::ports::{InventoryStore, StoreResult};
    use domain::inventory::{InventoryItem, InventoryQuery, Status, constants, seed};

    const ALPHA: &str = "Alpha";
    const BRAVO: &str = "Bravo";
    const FEW: i32 = 2;
    const MANY: i32 = 9;
    const TWO_ROWS: usize = 2;
    const ONE_MATCH: usize = 1;
    const SEARCH: &str = "badge";

    /// Answers with rows in an order the domain is expected to fix.
    struct Unordered;

    impl InventoryStore for Unordered {
        fn items(&self) -> StoreResult<Vec<InventoryItem>> {
            Ok(vec![
                InventoryItem {
                    name: BRAVO.to_owned(),
                    quantity: MANY,
                    status: Status::new(constants::STATUS_IN_STOCK),
                },
                InventoryItem {
                    name: ALPHA.to_owned(),
                    quantity: FEW,
                    status: Status::new(constants::STATUS_LOW),
                },
            ])
        }
    }

    /// Answers with the domain's own seed.
    struct Seeded;

    impl InventoryStore for Seeded {
        fn items(&self) -> StoreResult<Vec<InventoryItem>> {
            Ok(seed())
        }
    }

    #[test]
    fn the_query_orders_the_store_rows_rather_than_trusting_their_order() {
        let query = InventoryQuery::parse(None, None, None).unwrap();

        let answered = InventoryService::new(Unordered).query(&query).unwrap();

        let names: Vec<&str> = answered.items.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, vec![ALPHA, BRAVO]);
        assert_eq!(answered.total, TWO_ROWS);
    }

    #[test]
    fn a_search_narrows_to_the_rows_that_match() {
        let query = InventoryQuery::parse(Some(SEARCH), None, None).unwrap();

        let answered = InventoryService::new(Seeded).query(&query).unwrap();

        assert_eq!(answered.shown, ONE_MATCH);
        assert_eq!(answered.total, seed().len());
    }
}
