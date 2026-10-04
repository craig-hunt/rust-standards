//! Tasks: the identifiers, the title, the filter, and what a list request answers.

use crate::errors::{DomainError, FieldError};
use crate::text;

/// Every literal the task feature carries, named once.
///
/// Tests reference these rather than repeating the values, so a reworded message
/// fails to compile instead of drifting.
pub mod constants {
    /// The longest title the rules accept, in characters.
    pub const MAX_TITLE_LENGTH: usize = 200;

    /// Query value selecting every task.
    pub const FILTER_ALL: &str = "all";
    /// Query value selecting the incomplete tasks.
    pub const FILTER_ACTIVE: &str = "active";
    /// Query value selecting the completed tasks.
    pub const FILTER_COMPLETED: &str = "completed";

    /// Member name a client sends for a title.
    pub const FIELD_TITLE: &str = "title";
    /// Member name a client sends for a completion flag.
    pub const FIELD_COMPLETED: &str = "completed";

    /// Code for a filter the service does not publish.
    pub const CODE_INVALID_FILTER: &str = "invalid_filter";
    /// Code for an identifier that is not one.
    pub const CODE_INVALID_ID: &str = "invalid_id";
    /// Code for a task that does not exist.
    pub const CODE_NOT_FOUND: &str = "not_found";

    /// Detail for [`CODE_INVALID_FILTER`].
    pub const MSG_INVALID_FILTER: &str = "filter must be all, active, or completed";
    /// Detail for [`CODE_INVALID_ID`].
    pub const MSG_INVALID_ID: &str = "task id must be a positive whole number";
    /// Detail for [`CODE_NOT_FOUND`].
    pub const MSG_NOT_FOUND: &str = "no task has that id";
    /// Shown beside a title input left empty.
    pub const MSG_TITLE_REQUIRED: &str = "Enter a task title.";
    /// Shown beside a title input that ran long.
    pub const MSG_TITLE_TOO_LONG: &str = "Keep the task title within the length limit.";
    /// Shown when an update omits the completion flag.
    pub const MSG_COMPLETED_REQUIRED: &str = "Say whether the task is completed.";
}

/// The failures the task feature raises.
pub mod errors {
    use super::constants;
    use crate::errors::{DomainError, FieldError};

    /// A title arrived empty.
    #[must_use]
    pub const fn title_required() -> FieldError {
        FieldError::new(constants::FIELD_TITLE, constants::MSG_TITLE_REQUIRED)
    }

    /// A title ran past the length limit.
    #[must_use]
    pub const fn title_too_long() -> FieldError {
        FieldError::new(constants::FIELD_TITLE, constants::MSG_TITLE_TOO_LONG)
    }

    /// An update omitted the completion flag.
    #[must_use]
    pub const fn completed_required() -> FieldError {
        FieldError::new(
            constants::FIELD_COMPLETED,
            constants::MSG_COMPLETED_REQUIRED,
        )
    }

    /// A filter the service does not publish.
    #[must_use]
    pub fn invalid_filter() -> DomainError {
        DomainError::invalid(
            constants::CODE_INVALID_FILTER,
            constants::MSG_INVALID_FILTER,
        )
    }

    /// An identifier below the first valid value.
    #[must_use]
    pub fn invalid_id() -> DomainError {
        DomainError::invalid(constants::CODE_INVALID_ID, constants::MSG_INVALID_ID)
    }

    /// No task has that identifier.
    #[must_use]
    pub const fn not_found() -> DomainError {
        DomainError::not_found(constants::CODE_NOT_FOUND, constants::MSG_NOT_FOUND)
    }
}

/// A task identifier, distinct from every other identifier in the system.
///
/// A bare `i64` would compile in every position an identifier appears, which is
/// exactly the problem: passing a signup id where a task id belongs would
/// type-check. The field is private and the only constructor validates, so an
/// invalid identifier has no representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(i64);

impl TaskId {
    const FIRST_VALID: i64 = 1;

    /// Wraps a value the rules accept.
    ///
    /// # Errors
    ///
    /// Returns [`errors::invalid_id`] when the value falls below the first valid
    /// identifier.
    pub fn new(value: i64) -> Result<Self, DomainError> {
        if value < Self::FIRST_VALID {
            return Err(errors::invalid_id());
        }
        Ok(Self(value))
    }

    /// The underlying value, for a store that has to write it.
    #[must_use]
    pub const fn value(self) -> i64 {
        self.0
    }

    /// Reads an identifier from a path segment, answering `None` when the text
    /// does not name one.
    ///
    /// The scan accepts ASCII digits and nothing else. `str::parse` would accept
    /// a leading sign and surrounding text varies by type, so either on its own
    /// would let a request address a task by a spelling the Go, C# and Java
    /// siblings reject. All four agree on which requests reach a handler.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        raw.parse::<i64>()
            .ok()
            .and_then(|value| Self::new(value).ok())
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A validated task title: trimmed, present, and within the length limit.
///
/// The constructor trims the value it stores, so the trimming and the checks
/// cannot disagree and no caller can hold an untrimmed title.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskTitle(String);

impl TaskTitle {
    /// Trims and validates a submitted title.
    ///
    /// # Errors
    ///
    /// Returns [`errors::title_required`] for a title that is empty or only
    /// whitespace, and [`errors::title_too_long`] for one past the limit.
    pub fn new(raw: &str) -> Result<Self, FieldError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(errors::title_required());
        }
        if text::count_characters(trimmed) > constants::MAX_TITLE_LENGTH {
            return Err(errors::title_too_long());
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The validated value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TaskTitle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Which tasks a list request shows.
///
/// The siblings need a switch with no default arm, or an extension method, to
/// stop a new member falling silently into a catch-all. Rust matches
/// exhaustively by default, so adding a variant here stops the build in
/// [`TaskFilter::includes`] until somebody says which tasks it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskFilter {
    /// Every task.
    #[default]
    All,
    /// The tasks still to do.
    Active,
    /// The tasks already done.
    Completed,
}

impl TaskFilter {
    /// Every filter, for a parse that cannot forget one.
    const ALL: [Self; 3] = [Self::All, Self::Active, Self::Completed];

    /// The query value that names this filter.
    #[must_use]
    pub const fn query_value(self) -> &'static str {
        match self {
            Self::All => constants::FILTER_ALL,
            Self::Active => constants::FILTER_ACTIVE,
            Self::Completed => constants::FILTER_COMPLETED,
        }
    }

    /// Parses the query value, treating an absent value as [`TaskFilter::All`].
    ///
    /// Walks the variants rather than listing the strings again, so adding a
    /// filter cannot leave the parse behind.
    ///
    /// # Errors
    ///
    /// Returns [`errors::invalid_filter`] for a value no variant publishes.
    pub fn parse(raw: Option<&str>) -> Result<Self, DomainError> {
        match raw {
            None | Some("") => Ok(Self::All),
            Some(value) => Self::ALL
                .into_iter()
                .find(|filter| filter.query_value() == value)
                .ok_or_else(errors::invalid_filter),
        }
    }

    /// Whether this filter shows the given task.
    #[must_use]
    pub const fn includes(self, task: &TaskItem) -> bool {
        match self {
            Self::All => true,
            Self::Active => !task.completed,
            Self::Completed => task.completed,
        }
    }
}

/// One task, as the domain understands it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItem {
    /// The identifier the database assigned.
    pub id: TaskId,
    /// The validated title.
    pub title: TaskTitle,
    /// Whether the task is done.
    pub completed: bool,
}

/// What a list request answers.
///
/// `remaining` and `total` count the whole set rather than the filtered view, so
/// the counts stay steady while a reader switches filters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskView {
    /// The tasks the filter shows.
    pub tasks: Vec<TaskItem>,
    /// How many tasks across the whole set are still incomplete.
    pub remaining: usize,
    /// How many tasks exist.
    pub total: usize,
}

impl TaskView {
    /// Filters a set of tasks and counts the whole of it.
    #[must_use]
    pub fn summarize(all: &[TaskItem], filter: TaskFilter) -> Self {
        Self {
            tasks: all
                .iter()
                .filter(|task| filter.includes(task))
                .cloned()
                .collect(),
            remaining: all.iter().filter(|task| !task.completed).count(),
            total: all.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here. Scoped to the test module, so production code still
    // cannot reach for any of them.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{TaskFilter, TaskId, TaskItem, TaskTitle, TaskView, constants, errors};

    const FIRST: &str = "Read the ADR";
    const SECOND: &str = "Answer the questionnaire";
    const THIRD: &str = "Book the review";
    const PADDED: &str = "   Read the ADR   ";
    const EMOJI: &str = "\u{1F600}";
    const LETTER: &str = "a";

    const FIRST_VALID_ID: i64 = 1;
    const BELOW_FIRST_VALID_ID: i64 = 0;
    const NEGATIVE_ID: i64 = -1;
    const LATER_ID: i64 = 42;
    const LATER_ID_TEXT: &str = "42";
    const FIRST_ID_TEXT: &str = "1";

    const ONE_SHOWN: usize = 1;
    const TWO_REMAIN: usize = 2;
    const THREE_EXIST: usize = 3;
    const NONE: usize = 0;
    const SECOND_ID: i64 = 2;
    const THIRD_ID: i64 = 3;

    /// Spellings that name no task: whitespace, a sign, a decimal, a word, a
    /// non-ASCII digit, and the value below the first valid identifier.
    const REFUSED_IDENTIFIERS: [&str; 10] = [
        " 1", "1 ", "+1", "-1", "1.0", "", "abc", "1a", "\u{661}", "0",
    ];

    /// Spellings that do, including both ends of the ASCII digit range.
    const ACCEPTED_IDENTIFIERS: [&str; 5] = ["10", "19", "90", "109", "1234567890"];

    /// Larger than the column can hold.
    const BEYOND_THE_COLUMN: &str = "9223372036854775808";

    /// A title that is absent, empty, or only whitespace.
    const BLANK_TITLES: [&str; 4] = ["", " ", "\t", "\n"];

    /// A filter the service does not publish.
    const UNKNOWN_FILTER: &str = "archived";

    fn task(id: i64, title: &str, completed: bool) -> TaskItem {
        TaskItem {
            id: TaskId::new(id).unwrap(),
            title: TaskTitle::new(title).unwrap(),
            completed,
        }
    }

    fn three_tasks() -> Vec<TaskItem> {
        vec![
            task(1, FIRST, false),
            task(2, SECOND, true),
            task(THIRD_ID, THIRD, false),
        ]
    }

    #[test]
    fn an_identifier_accepts_the_first_valid_value_and_refuses_the_one_below() {
        assert_eq!(TaskId::new(FIRST_VALID_ID).unwrap().value(), FIRST_VALID_ID);
        assert_eq!(
            TaskId::new(BELOW_FIRST_VALID_ID).unwrap_err().code(),
            constants::CODE_INVALID_ID,
            "zero is not an identifier"
        );
        assert!(TaskId::new(NEGATIVE_ID).is_err());
    }

    #[test]
    fn an_identifier_parses_only_plain_ascii_digits() {
        assert_eq!(TaskId::parse(LATER_ID_TEXT).unwrap().value(), LATER_ID);
        assert_eq!(
            TaskId::parse(FIRST_ID_TEXT).unwrap().value(),
            FIRST_VALID_ID
        );

        for refused in REFUSED_IDENTIFIERS {
            assert!(
                TaskId::parse(refused).is_none(),
                "{refused} must not name a task"
            );
        }
    }

    #[test]
    fn an_identifier_parses_every_ascii_digit() {
        for accepted in ACCEPTED_IDENTIFIERS {
            assert!(TaskId::parse(accepted).is_some(), "{accepted} names a task");
        }
    }

    #[test]
    fn an_identifier_too_large_for_the_column_names_no_task() {
        assert!(TaskId::parse(BEYOND_THE_COLUMN).is_none());
    }

    #[test]
    fn an_identifier_prints_the_bare_number() {
        assert_eq!(TaskId::new(LATER_ID).unwrap().to_string(), LATER_ID_TEXT);
    }

    #[test]
    fn a_title_stores_itself_trimmed() {
        assert_eq!(TaskTitle::new(PADDED).unwrap().value(), FIRST);
        assert_eq!(TaskTitle::new(FIRST).unwrap().to_string(), FIRST);
    }

    #[test]
    fn a_title_cannot_be_absent_empty_or_whitespace() {
        for refused in BLANK_TITLES {
            let failure = TaskTitle::new(refused).unwrap_err();
            assert_eq!(failure.field, constants::FIELD_TITLE);
            assert_eq!(failure.problem, constants::MSG_TITLE_REQUIRED);
        }
    }

    #[test]
    fn a_title_accepts_the_limit_and_refuses_one_character_past_it() {
        let at_limit = LETTER.repeat(constants::MAX_TITLE_LENGTH);
        let one_too_many = LETTER.repeat(constants::MAX_TITLE_LENGTH + 1);

        assert!(TaskTitle::new(&at_limit).is_ok());
        assert_eq!(
            TaskTitle::new(&one_too_many).unwrap_err().problem,
            constants::MSG_TITLE_TOO_LONG
        );
    }

    #[test]
    fn a_title_measures_the_limit_in_characters_not_bytes() {
        let at_limit = EMOJI.repeat(constants::MAX_TITLE_LENGTH);

        assert!(
            TaskTitle::new(&at_limit).is_ok(),
            "a limit counted in bytes would reject this at a quarter of the length"
        );
    }

    #[test]
    fn an_absent_filter_shows_everything() {
        assert_eq!(TaskFilter::parse(None).unwrap(), TaskFilter::All);
        assert_eq!(TaskFilter::parse(Some("")).unwrap(), TaskFilter::All);
    }

    #[test]
    fn every_filter_parses_the_query_value_it_publishes() {
        for filter in [TaskFilter::All, TaskFilter::Active, TaskFilter::Completed] {
            assert_eq!(
                TaskFilter::parse(Some(filter.query_value())).unwrap(),
                filter
            );
        }
    }

    #[test]
    fn a_filter_the_service_does_not_publish_is_refused() {
        assert_eq!(
            TaskFilter::parse(Some(UNKNOWN_FILTER)).unwrap_err().code(),
            constants::CODE_INVALID_FILTER
        );
    }

    #[test]
    fn each_filter_shows_the_tasks_it_names_and_hides_the_others() {
        let active = task(FIRST_VALID_ID, FIRST, false);
        let done = task(SECOND_ID, SECOND, true);

        assert!(TaskFilter::All.includes(&active) && TaskFilter::All.includes(&done));
        assert!(TaskFilter::Active.includes(&active) && !TaskFilter::Active.includes(&done));
        assert!(TaskFilter::Completed.includes(&done) && !TaskFilter::Completed.includes(&active));
    }

    #[test]
    fn a_view_counts_across_every_task_not_the_filtered_set() {
        let view = TaskView::summarize(&three_tasks(), TaskFilter::Completed);

        assert_eq!(view.tasks.len(), ONE_SHOWN, "one completed task is shown");
        assert_eq!(
            view.remaining, TWO_REMAIN,
            "two remain across the whole set"
        );
        assert_eq!(view.total, THREE_EXIST, "three tasks exist");
    }

    #[test]
    fn a_view_keeps_the_order_the_store_returned() {
        let view = TaskView::summarize(&three_tasks(), TaskFilter::Active);

        let titles: Vec<&str> = view.tasks.iter().map(|task| task.title.value()).collect();
        assert_eq!(titles, vec![FIRST, THIRD]);
    }

    #[test]
    fn a_view_of_nothing_reports_nothing() {
        let view = TaskView::summarize(&[], TaskFilter::All);

        assert_eq!(view.tasks.len(), 0);
        assert_eq!(view.remaining, NONE);
        assert_eq!(view.total, NONE);
    }

    #[test]
    fn each_failure_names_the_field_it_concerns() {
        assert_eq!(errors::title_required().field, constants::FIELD_TITLE);
        assert_eq!(errors::title_too_long().field, constants::FIELD_TITLE);
        assert_eq!(
            errors::completed_required().field,
            constants::FIELD_COMPLETED
        );
        assert_eq!(
            errors::completed_required().problem,
            constants::MSG_COMPLETED_REQUIRED
        );
    }

    #[test]
    fn a_missing_task_is_not_found_rather_than_invalid() {
        let failure = errors::not_found();

        assert_eq!(failure.code(), constants::CODE_NOT_FOUND);
        assert_eq!(
            failure.fields().len(),
            0,
            "nothing to render beside an input"
        );
    }
}
