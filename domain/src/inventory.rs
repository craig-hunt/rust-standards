//! Stock: a validated query, and the rows it answers with.

use crate::errors::DomainError;

/// Every literal the inventory feature carries, named once.
pub mod constants {
    /// Query value selecting the name column.
    pub const COLUMN_NAME: &str = "name";
    /// Query value selecting the quantity column.
    pub const COLUMN_QUANTITY: &str = "quantity";
    /// Query value selecting the status column.
    pub const COLUMN_STATUS: &str = "status";

    /// Query value for an ascending sort.
    pub const DIRECTION_ASCENDING: &str = "ascending";
    /// Query value for a descending sort.
    pub const DIRECTION_DESCENDING: &str = "descending";

    /// Plenty on the shelf.
    pub const STATUS_IN_STOCK: &str = "In stock";
    /// Running out.
    pub const STATUS_LOW: &str = "Low";
    /// None left.
    pub const STATUS_OUT_OF_STOCK: &str = "Out of stock";

    /// Code for a query naming something the table cannot do.
    pub const CODE_INVALID_QUERY: &str = "invalid_query";
    /// Detail for a column the table cannot sort on.
    pub const MSG_INVALID_SORT: &str = "sort must be name, quantity, or status";
    /// Detail for an order that does not exist.
    pub const MSG_INVALID_DIRECTION: &str = "direction must be ascending or descending";
}

/// The failures the inventory feature raises.
pub mod errors {
    use super::constants;
    use crate::errors::DomainError;

    /// A column the table cannot sort on.
    #[must_use]
    pub fn invalid_sort() -> DomainError {
        DomainError::invalid(constants::CODE_INVALID_QUERY, constants::MSG_INVALID_SORT)
    }

    /// An order that does not exist.
    #[must_use]
    pub fn invalid_direction() -> DomainError {
        DomainError::invalid(
            constants::CODE_INVALID_QUERY,
            constants::MSG_INVALID_DIRECTION,
        )
    }
}

/// A column the stock table sorts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Column {
    /// Order by name.
    #[default]
    Name,
    /// Order by how many are left.
    Quantity,
    /// Order by the words the table shows.
    Status,
}

impl Column {
    const ALL: [Self; 3] = [Self::Name, Self::Quantity, Self::Status];

    /// The query value that names this column.
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::Name => constants::COLUMN_NAME,
            Self::Quantity => constants::COLUMN_QUANTITY,
            Self::Status => constants::COLUMN_STATUS,
        }
    }

    /// Reads a requested column, defaulting an absent one to the name.
    ///
    /// # Errors
    ///
    /// Returns [`errors::invalid_sort`] for a column the table cannot sort on.
    pub fn parse(raw: Option<&str>) -> Result<Self, DomainError> {
        match raw {
            None | Some("") => Ok(Self::default()),
            Some(value) => Self::ALL
                .into_iter()
                .find(|column| column.value() == value)
                .ok_or_else(errors::invalid_sort),
        }
    }
}

/// Which way a sort runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Smallest first.
    #[default]
    Ascending,
    /// Largest first.
    Descending,
}

impl Direction {
    const ALL: [Self; 2] = [Self::Ascending, Self::Descending];

    /// The query value that names this order.
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::Ascending => constants::DIRECTION_ASCENDING,
            Self::Descending => constants::DIRECTION_DESCENDING,
        }
    }

    /// Reads a requested order, defaulting an absent one to ascending.
    ///
    /// # Errors
    ///
    /// Returns [`errors::invalid_direction`] for an order that does not exist.
    pub fn parse(raw: Option<&str>) -> Result<Self, DomainError> {
        match raw {
            None | Some("") => Ok(Self::default()),
            Some(value) => Self::ALL
                .into_iter()
                .find(|direction| direction.value() == value)
                .ok_or_else(errors::invalid_direction),
        }
    }
}

/// The stock status a row reports, in the words the table shows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Status(String);

impl Status {
    /// Keeps whatever the row said.
    #[must_use]
    pub fn new(raw: &str) -> Self {
        Self(raw.to_owned())
    }

    /// The words the table shows.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.0
    }
}

/// One row of the stock table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryItem {
    /// What it is.
    pub name: String,
    /// How many are left.
    pub quantity: i32,
    /// What the table says about that number.
    pub status: Status,
}

/// What a stock query answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryResult {
    /// The rows that matched, in the requested order.
    pub items: Vec<InventoryItem>,
    /// How many matched.
    pub shown: usize,
    /// How many rows exist.
    pub total: usize,
}

/// A validated stock query.
///
/// The Go sibling parses this straight from the query string. This takes three
/// values instead, so the domain states the rule without knowing that a query
/// string exists or what its keys are called.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InventoryQuery {
    /// The text a name must contain, empty to match everything.
    pub search: String,
    /// Which column orders the rows.
    pub sort: Column,
    /// Which way that order runs.
    pub direction: Direction,
}

impl InventoryQuery {
    /// Validates a requested query.
    ///
    /// # Errors
    ///
    /// Returns [`errors::invalid_sort`] or [`errors::invalid_direction`] when
    /// the request names something the table cannot do.
    pub fn parse(
        search: Option<&str>,
        sort: Option<&str>,
        direction: Option<&str>,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            search: search.unwrap_or_default().trim().to_owned(),
            sort: Column::parse(sort)?,
            direction: Direction::parse(direction)?,
        })
    }

    /// Filters and orders a set of rows, leaving the caller's slice untouched.
    ///
    /// The sort is stable, so rows that tie on the sort column keep the order
    /// the store returned rather than shuffling between requests.
    #[must_use]
    pub fn apply(&self, items: &[InventoryItem]) -> InventoryResult {
        let needle = self.search.to_lowercase();
        let mut matched: Vec<InventoryItem> = items
            .iter()
            .filter(|item| item.name.to_lowercase().contains(&needle))
            .cloned()
            .collect();

        // The comparator carries the direction rather than the sorted vector
        // being reversed afterwards. Reversing undoes the stability the sort just
        // provided: rows tied on the sort column come back in the opposite of the
        // order the store returned them, so a descending sort on status shuffles
        // every item sharing a status.
        matched.sort_by(|left, right| {
            let ordered = match self.sort {
                Column::Name => left.name.cmp(&right.name),
                Column::Quantity => left.quantity.cmp(&right.quantity),
                Column::Status => left.status.cmp(&right.status),
            };
            match self.direction {
                Direction::Ascending => ordered,
                Direction::Descending => ordered.reverse(),
            }
        });

        InventoryResult {
            shown: matched.len(),
            total: items.len(),
            items: matched,
        }
    }
}

/// What a fresh database starts with, named rather than written into the list.
///
/// The rule covers a seed row as it covers anything else. These were literals
/// inside a `vec!` until the literal rule learned to read macro tokens, which is
/// exactly the hiding place a rule about source text leaves open when it trusts
/// the syntax tree alone.
mod seeded {
    pub(super) const ACCESS_BADGE: &str = "Access badge";
    pub(super) const ACCESS_BADGES_ON_HAND: i32 = 240;
    pub(super) const DOCKING_STATION: &str = "Docking station";
    pub(super) const DOCKING_STATIONS_ON_HAND: i32 = 12;
    pub(super) const LAPTOP_SLEEVE: &str = "Laptop sleeve";
    pub(super) const LAPTOP_SLEEVES_ON_HAND: i32 = 0;
    pub(super) const MONITOR_ARM: &str = "Monitor arm";
    pub(super) const MONITOR_ARMS_ON_HAND: i32 = 58;
    pub(super) const HEADSET: &str = "Noise-cancelling headset";
    pub(super) const HEADSETS_ON_HAND: i32 = 4;
    pub(super) const WEBCAM: &str = "Webcam";
    pub(super) const WEBCAMS_ON_HAND: i32 = 31;
}

/// The rows a fresh database starts with, so the reference API answers with
/// something recognizable before anyone adds stock.
#[must_use]
pub fn seed() -> Vec<InventoryItem> {
    vec![
        row(
            seeded::ACCESS_BADGE,
            seeded::ACCESS_BADGES_ON_HAND,
            constants::STATUS_IN_STOCK,
        ),
        row(
            seeded::DOCKING_STATION,
            seeded::DOCKING_STATIONS_ON_HAND,
            constants::STATUS_LOW,
        ),
        row(
            seeded::LAPTOP_SLEEVE,
            seeded::LAPTOP_SLEEVES_ON_HAND,
            constants::STATUS_OUT_OF_STOCK,
        ),
        row(
            seeded::MONITOR_ARM,
            seeded::MONITOR_ARMS_ON_HAND,
            constants::STATUS_IN_STOCK,
        ),
        row(
            seeded::HEADSET,
            seeded::HEADSETS_ON_HAND,
            constants::STATUS_LOW,
        ),
        row(
            seeded::WEBCAM,
            seeded::WEBCAMS_ON_HAND,
            constants::STATUS_IN_STOCK,
        ),
    ]
}

fn row(name: &str, quantity: i32, status: &str) -> InventoryItem {
    InventoryItem {
        name: name.to_owned(),
        quantity,
        status: Status::new(status),
    }
}

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here. Scoped to the test module, so production code still
    // cannot reach for any of them.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{
        Column, Direction, InventoryItem, InventoryQuery, Status, constants, seed, seeded,
    };

    const ALPHA: &str = "Alpha";
    const BRAVO: &str = "Bravo";
    const CHARLIE: &str = "Charlie";
    const SHARED_QUANTITY: i32 = 5;
    const FEW: i32 = 2;
    const MANY: i32 = 9;
    const ONE_MATCH: usize = 1;
    const THREE_ROWS: usize = 3;
    const SEED_ROWS: usize = 6;
    const UNKNOWN_COLUMN: &str = "colour";
    const UNKNOWN_DIRECTION: &str = "sideways";
    const SEARCH_IN_UPPER_CASE: &str = "BADGE";
    const PADDED_SEARCH: &str = "  badge  ";
    const TRIMMED_SEARCH: &str = "badge";

    fn rows() -> Vec<InventoryItem> {
        vec![
            item(CHARLIE, FEW, constants::STATUS_LOW),
            item(ALPHA, MANY, constants::STATUS_OUT_OF_STOCK),
            item(BRAVO, SHARED_QUANTITY, constants::STATUS_IN_STOCK),
        ]
    }

    /// Two rows the sort cannot separate, in the order a store returned them.
    fn tied_rows() -> Vec<InventoryItem> {
        vec![
            item(CHARLIE, SHARED_QUANTITY, constants::STATUS_LOW),
            item(ALPHA, SHARED_QUANTITY, constants::STATUS_LOW),
        ]
    }

    fn item(name: &str, quantity: i32, status: &str) -> InventoryItem {
        InventoryItem {
            name: name.to_owned(),
            quantity,
            status: Status::new(status),
        }
    }

    fn names(sort: Option<&str>, direction: Option<&str>) -> Vec<String> {
        InventoryQuery::parse(None, sort, direction)
            .unwrap()
            .apply(&rows())
            .items
            .into_iter()
            .map(|row| row.name)
            .collect()
    }

    #[test]
    fn an_absent_query_orders_by_name_ascending() {
        let query = InventoryQuery::parse(None, None, None).unwrap();

        assert_eq!(query.sort, Column::Name);
        assert_eq!(query.direction, Direction::Ascending);
        assert_eq!(query.search, "");
    }

    #[test]
    fn an_empty_query_value_reads_as_absent() {
        let query = InventoryQuery::parse(Some(""), Some(""), Some("")).unwrap();

        assert_eq!(query.sort, Column::Name);
        assert_eq!(query.direction, Direction::Ascending);
    }

    #[test]
    fn a_column_the_table_cannot_sort_on_is_refused() {
        let failure = InventoryQuery::parse(None, Some(UNKNOWN_COLUMN), None).unwrap_err();

        assert_eq!(failure.code(), constants::CODE_INVALID_QUERY);
        assert_eq!(failure.to_string(), constants::MSG_INVALID_SORT);
    }

    #[test]
    fn an_order_that_does_not_exist_is_refused() {
        let failure = InventoryQuery::parse(None, None, Some(UNKNOWN_DIRECTION)).unwrap_err();

        assert_eq!(failure.to_string(), constants::MSG_INVALID_DIRECTION);
    }

    #[test]
    fn the_search_term_is_trimmed() {
        assert_eq!(
            InventoryQuery::parse(Some(PADDED_SEARCH), None, None)
                .unwrap()
                .search,
            TRIMMED_SEARCH
        );
    }

    #[test]
    fn a_search_matches_regardless_of_case() {
        let matched = InventoryQuery::parse(Some(SEARCH_IN_UPPER_CASE), None, None)
            .unwrap()
            .apply(&seed());

        assert_eq!(matched.shown, ONE_MATCH);
        assert_eq!(matched.total, seed().len());
        assert_eq!(matched.items[0].name, seeded::ACCESS_BADGE);
    }

    #[test]
    fn a_descending_sort_keeps_tied_rows_in_the_order_the_store_returned_them() {
        let ordered: Vec<String> = InventoryQuery::parse(
            None,
            Some(constants::COLUMN_STATUS),
            Some(constants::DIRECTION_DESCENDING),
        )
        .unwrap()
        .apply(&tied_rows())
        .items
        .into_iter()
        .map(|row| row.name)
        .collect();

        assert_eq!(
            ordered,
            vec![CHARLIE, ALPHA],
            "reversing the sorted vector would answer these the other way round"
        );
    }

    #[test]
    fn each_column_orders_in_both_directions() {
        assert_eq!(
            names(Some(constants::COLUMN_NAME), None),
            vec![ALPHA, BRAVO, CHARLIE]
        );
        assert_eq!(
            names(
                Some(constants::COLUMN_NAME),
                Some(constants::DIRECTION_DESCENDING)
            ),
            vec![CHARLIE, BRAVO, ALPHA]
        );

        assert_eq!(
            names(Some(constants::COLUMN_QUANTITY), None),
            vec![CHARLIE, BRAVO, ALPHA]
        );
        assert_eq!(
            names(
                Some(constants::COLUMN_QUANTITY),
                Some(constants::DIRECTION_DESCENDING)
            ),
            vec![ALPHA, BRAVO, CHARLIE]
        );

        assert_eq!(
            names(Some(constants::COLUMN_STATUS), None),
            vec![BRAVO, CHARLIE, ALPHA]
        );
        assert_eq!(
            names(
                Some(constants::COLUMN_STATUS),
                Some(constants::DIRECTION_DESCENDING)
            ),
            vec![ALPHA, CHARLIE, BRAVO]
        );
    }

    #[test]
    fn counts_report_what_was_shown_against_what_exists() {
        let result = InventoryQuery::parse(Some(ALPHA), None, None)
            .unwrap()
            .apply(&rows());

        assert_eq!(result.shown, ONE_MATCH);
        assert_eq!(result.total, THREE_ROWS);
    }

    #[test]
    fn the_callers_rows_are_left_alone() {
        let caller = rows();

        let answered = InventoryQuery::parse(None, Some(constants::COLUMN_NAME), None)
            .unwrap()
            .apply(&caller);

        assert_eq!(
            answered.items.len(),
            caller.len(),
            "every row matched an empty search"
        );
        assert_eq!(
            caller,
            rows(),
            "apply takes a slice and answers with a new vector"
        );
    }

    #[test]
    fn a_status_reads_back_the_words_it_was_given() {
        for words in [
            constants::STATUS_IN_STOCK,
            constants::STATUS_LOW,
            constants::STATUS_OUT_OF_STOCK,
        ] {
            assert_eq!(
                Status::new(words).value(),
                words,
                "the table shows these words, so nothing may rewrite them"
            );
        }
    }

    #[test]
    fn the_seed_carries_the_status_words_the_table_shows() {
        let seeded = seed();
        let statuses: Vec<&str> = seeded.iter().map(|row| row.status.value()).collect();

        assert!(statuses.contains(&constants::STATUS_IN_STOCK));
        assert!(statuses.contains(&constants::STATUS_LOW));
        assert!(statuses.contains(&constants::STATUS_OUT_OF_STOCK));
    }

    #[test]
    fn the_seed_is_recognizable_and_stable() {
        assert_eq!(seed().len(), SEED_ROWS);
        assert_eq!(seed(), seed(), "two calls answer the same rows");
    }
}
