//! Signups: a validated form, field by field.

use crate::errors::{DomainError, FieldError, FieldProblems};
use crate::events::DomainEvent;
use crate::text;
use std::time::SystemTime;
use uuid::Uuid;

/// The at-sign separating an address into its two halves.
const AT_SIGN: char = '@';

/// The dot separating a domain into labels.
const LABEL_SEPARATOR: char = '.';

/// Placeholders [`Signup::summary`] substitutes into [`constants::SUMMARY_FORMAT`].
const NAME_TOKEN: &str = "{name}";
const PLAN_TOKEN: &str = "{plan}";
const SEATS_TOKEN: &str = "{seats}";

/// Every literal the signup feature carries, named once.
pub mod constants {
    /// The entry plan.
    pub const PLAN_STARTER: &str = "Starter";
    /// The middle plan.
    pub const PLAN_GROWTH: &str = "Growth";
    /// The plan with a salesperson attached.
    pub const PLAN_ENTERPRISE: &str = "Enterprise";

    /// Member name a client sends for a name.
    pub const FIELD_FULL_NAME: &str = "fullName";
    /// Member name a client sends for an address.
    pub const FIELD_EMAIL: &str = "email";
    /// Member name a client sends for a plan.
    pub const FIELD_PLAN: &str = "plan";
    /// Member name a client sends for a seat count.
    pub const FIELD_SEATS: &str = "seats";
    /// Member name a client sends for notes.
    pub const FIELD_NOTES: &str = "notes";
    /// Member name a client sends for the terms flag.
    pub const FIELD_ACCEPT_TERMS: &str = "acceptTerms";

    /// Shown beside a name input left empty.
    pub const MSG_NAME_REQUIRED: &str = "Enter your full name.";
    /// Shown beside an address input left empty.
    pub const MSG_EMAIL_REQUIRED: &str = "Enter your work email.";
    /// Shown beside an address that is not shaped like one.
    pub const MSG_EMAIL_INVALID: &str = "Enter a valid email address.";
    /// Shown when no plan was chosen.
    pub const MSG_PLAN_REQUIRED: &str = "Choose a plan.";
    /// Shown when the plan chosen is not on offer.
    pub const MSG_PLAN_UNKNOWN: &str = "Choose the Starter, Growth, or Enterprise plan.";
    /// Shown beside a seat count below the minimum.
    pub const MSG_SEATS_INVALID: &str = "Enter at least one seat.";
    /// Shown beside notes that ran long.
    pub const MSG_NOTES_TOO_LONG: &str = "Keep the notes within the length limit.";
    /// Shown when the terms were not accepted.
    pub const MSG_TERMS_REQUIRED: &str = "Accept the terms to continue.";

    /// Code for an identifier that is not one.
    pub const CODE_INVALID_ID: &str = "invalid_id";
    /// Detail for [`CODE_INVALID_ID`].
    pub const MSG_INVALID_ID: &str = "signup id must be a positive whole number";

    /// Seats a form that mentioned none is taken to want.
    pub const DEFAULT_SEATS: i32 = 1;
    /// The fewest seats the rules accept.
    pub const MIN_SEATS: i32 = 1;
    /// The longest notes the rules accept, in characters.
    pub const MAX_NOTES_LENGTH: usize = 1000;

    /// How a confirmation reads back a signup.
    pub const SUMMARY_FORMAT: &str = "{name} on the {plan} plan, {seats} seat(s).";
}

/// The failures the signup feature raises.
pub mod errors {
    use super::constants;
    use crate::errors::{DomainError, FieldError};

    /// A name arrived empty.
    #[must_use]
    pub const fn name_required() -> FieldError {
        FieldError::new(constants::FIELD_FULL_NAME, constants::MSG_NAME_REQUIRED)
    }

    /// An address arrived empty.
    #[must_use]
    pub const fn email_required() -> FieldError {
        FieldError::new(constants::FIELD_EMAIL, constants::MSG_EMAIL_REQUIRED)
    }

    /// An address arrived malformed.
    #[must_use]
    pub const fn email_invalid() -> FieldError {
        FieldError::new(constants::FIELD_EMAIL, constants::MSG_EMAIL_INVALID)
    }

    /// No plan was chosen.
    #[must_use]
    pub const fn plan_required() -> FieldError {
        FieldError::new(constants::FIELD_PLAN, constants::MSG_PLAN_REQUIRED)
    }

    /// The plan chosen is not on offer.
    #[must_use]
    pub const fn plan_unknown() -> FieldError {
        FieldError::new(constants::FIELD_PLAN, constants::MSG_PLAN_UNKNOWN)
    }

    /// A seat count below the minimum.
    #[must_use]
    pub const fn seats_invalid() -> FieldError {
        FieldError::new(constants::FIELD_SEATS, constants::MSG_SEATS_INVALID)
    }

    /// Notes past the length limit.
    #[must_use]
    pub const fn notes_too_long() -> FieldError {
        FieldError::new(constants::FIELD_NOTES, constants::MSG_NOTES_TOO_LONG)
    }

    /// The terms were not accepted.
    #[must_use]
    pub const fn terms_required() -> FieldError {
        FieldError::new(constants::FIELD_ACCEPT_TERMS, constants::MSG_TERMS_REQUIRED)
    }

    /// An identifier below the first valid value.
    #[must_use]
    pub fn invalid_id() -> DomainError {
        DomainError::invalid(constants::CODE_INVALID_ID, constants::MSG_INVALID_ID)
    }
}

/// A signup identifier, distinct from every other identifier in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignupId(i64);

impl SignupId {
    const FIRST_VALID: i64 = 1;

    /// Wraps a value the rules accept.
    ///
    /// # Errors
    ///
    /// Returns [`errors::invalid_id`] below the first valid identifier.
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
}

impl std::fmt::Display for SignupId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A signup's name: trimmed and present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullName(String);

impl FullName {
    /// Trims and validates a submitted name.
    ///
    /// # Errors
    ///
    /// Returns [`errors::name_required`] for empty or whitespace-only input.
    pub fn new(raw: &str) -> Result<Self, FieldError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(errors::name_required());
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The validated value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.0
    }
}

/// A signup's email: trimmed, present, and shaped like an address.
///
/// The check is written out rather than expressed as a regular expression. The
/// Go, C# and Java siblings use one because their standard libraries ship a
/// regex engine; Rust's does not, and taking a regex crate into the domain to
/// answer three questions would cost a dependency to say less clearly what is
/// accepted.
///
/// Deliberately coarse. An expression accepting exactly RFC 5322 would reject
/// addresses that work and accept ones that do not; the only test that settles
/// it is sending a message. This rejects what is obviously wrong and leaves the
/// rest to delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailAddress(String);

impl EmailAddress {
    /// Trims and validates a submitted address.
    ///
    /// # Errors
    ///
    /// Returns [`errors::email_required`] for empty input and
    /// [`errors::email_invalid`] for input that is not shaped like an address.
    pub fn new(raw: &str) -> Result<Self, FieldError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(errors::email_required());
        }
        if !Self::is_shaped_like_an_address(trimmed) {
            return Err(errors::email_invalid());
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// One at-sign, something either side of it, and a dot after it.
    fn is_shaped_like_an_address(candidate: &str) -> bool {
        if candidate.chars().any(char::is_whitespace) {
            return false;
        }
        let mut halves = candidate.split(AT_SIGN);
        let (Some(local), Some(domain), None) = (halves.next(), halves.next(), halves.next())
        else {
            return false;
        };
        !local.is_empty() && Self::has_a_dotted_label(domain)
    }

    /// A domain with a dot that separates two non-empty labels.
    fn has_a_dotted_label(domain: &str) -> bool {
        match domain.rsplit_once(LABEL_SEPARATOR) {
            Some((before, after)) => !before.is_empty() && !after.is_empty(),
            None => false,
        }
    }

    /// The validated value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.0
    }
}

/// The plan a signup names.
///
/// An enum, unlike the siblings, which hold the submitted text so they can tell
/// an absent plan from an unrecognized one. Rust needs no such trick: the
/// constructor returns which of the two failures occurred, and the type that
/// results can only be a plan on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// The entry plan.
    Starter,
    /// The middle plan.
    Growth,
    /// The plan with a salesperson attached.
    Enterprise,
}

impl Plan {
    /// Every plan, for a parse that cannot forget one.
    const ALL: [Self; 3] = [Self::Starter, Self::Growth, Self::Enterprise];

    /// The text a form submits for this plan.
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::Starter => constants::PLAN_STARTER,
            Self::Growth => constants::PLAN_GROWTH,
            Self::Enterprise => constants::PLAN_ENTERPRISE,
        }
    }

    /// Reads a submitted plan.
    ///
    /// # Errors
    ///
    /// Returns [`errors::plan_required`] for empty input and
    /// [`errors::plan_unknown`] for a plan not on offer, because a form shows a
    /// different sentence for each.
    pub fn new(raw: &str) -> Result<Self, FieldError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(errors::plan_required());
        }
        Self::ALL
            .into_iter()
            .find(|plan| plan.value() == trimmed)
            .ok_or_else(errors::plan_unknown)
    }
}

/// How many seats a signup takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seats(i32);

impl Seats {
    /// Validates a submitted count, defaulting an absent one.
    ///
    /// An absent count takes the default while an explicit zero fails.
    /// Collapsing both to zero would reject a form that never mentioned seats,
    /// and defaulting both would accept one asking for none.
    ///
    /// # Errors
    ///
    /// Returns [`errors::seats_invalid`] below the minimum.
    pub fn new(raw: Option<i32>) -> Result<Self, FieldError> {
        let count = raw.unwrap_or(constants::DEFAULT_SEATS);
        if count < constants::MIN_SEATS {
            return Err(errors::seats_invalid());
        }
        Ok(Self(count))
    }

    /// The validated count.
    #[must_use]
    pub const fn value(self) -> i32 {
        self.0
    }
}

/// Whatever a signup wanted to add, within the length limit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Notes(String);

impl Notes {
    /// Trims and validates submitted notes. Empty is valid: notes are optional.
    ///
    /// # Errors
    ///
    /// Returns [`errors::notes_too_long`] past the length limit.
    pub fn new(raw: &str) -> Result<Self, FieldError> {
        let trimmed = raw.trim();
        if text::count_characters(trimmed) > constants::MAX_NOTES_LENGTH {
            return Err(errors::notes_too_long());
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The validated value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.0
    }
}

/// What the form submitted, before validation.
#[derive(Debug, Clone, Default)]
pub struct SignupRequest {
    /// What was typed in the name box.
    pub full_name: String,
    /// What was typed in the email box.
    pub email: String,
    /// What was chosen from the plan list.
    pub plan: String,
    /// What was typed in the seats box, absent when it was left alone.
    pub seats: Option<i32>,
    /// What was typed in the notes box.
    pub notes: String,
    /// Whether the terms box was ticked.
    pub accept_terms: bool,
}

/// A signup whose every field already holds a usable value.
///
/// The guarantee is in the types, not in a convention: each field is a type that
/// cannot hold a value the rules reject, so this struct cannot be built out of a
/// blank name or a plan nobody offers. [`validate`] still earns its place,
/// because a constructor can only report the first problem it finds and a form
/// needs all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signup {
    /// The validated name.
    pub full_name: FullName,
    /// The validated address.
    pub email: EmailAddress,
    /// The chosen plan.
    pub plan: Plan,
    /// The validated seat count.
    pub seats: Seats,
    /// The validated notes.
    pub notes: Notes,
}

impl Signup {
    /// Reads this signup back in one sentence.
    #[must_use]
    pub fn summary(&self) -> String {
        constants::SUMMARY_FORMAT
            .replace(NAME_TOKEN, self.full_name.value())
            .replace(PLAN_TOKEN, self.plan.value())
            .replace(SEATS_TOKEN, &self.seats.value().to_string())
    }

    /// Describes this signup as the event a consumer receives.
    ///
    /// The signup composes its own event, so the fact travels with the rules
    /// that produced it rather than being assembled by whichever layer happens
    /// to write the row. The identifier arrives from the store because the
    /// database assigns it, and the clock from the caller because the domain
    /// owns none.
    #[must_use]
    pub fn recorded(&self, id: SignupId, event_id: Uuid, occurred_at: SystemTime) -> DomainEvent {
        DomainEvent::SignupRecorded {
            event_id,
            occurred_at,
            signup_id: id.value(),
            email: self.email.value().to_owned(),
            plan: self.plan.value().to_owned(),
            seats: self.seats.value(),
        }
    }
}

/// What a signup confirmation returns to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignupConfirmation {
    /// The identifier the database assigned.
    pub id: SignupId,
    /// The sentence reading the signup back.
    pub summary: String,
}

/// Validates a submitted signup, reporting every field problem at once.
///
/// Returning on the first problem would make a form correct one field per round
/// trip. Every check runs, so a caller marks all of them together.
///
/// Each check is the value type's own constructor rather than a copy of its
/// rule, so a sentence shown to a user is written in exactly one place.
///
/// # Errors
///
/// Returns a [`DomainError::Invalid`] carrying every field problem found.
pub fn validate(request: &SignupRequest) -> Result<Signup, DomainError> {
    let mut problems = FieldProblems::new();

    let full_name = collect(FullName::new(&request.full_name), &mut problems);
    let email = collect(EmailAddress::new(&request.email), &mut problems);
    let plan = collect(Plan::new(&request.plan), &mut problems);
    let seats = collect(Seats::new(request.seats), &mut problems);
    let notes = collect(Notes::new(&request.notes), &mut problems);

    if !request.accept_terms {
        let refused = errors::terms_required();
        problems.insert(refused.field.to_owned(), refused.problem.to_owned());
    }

    match (full_name, email, plan, seats, notes) {
        (Some(full_name), Some(email), Some(plan), Some(seats), Some(notes))
            if problems.is_empty() =>
        {
            Ok(Signup {
                full_name,
                email,
                plan,
                seats,
                notes,
            })
        }
        _ => Err(DomainError::with_fields(problems)),
    }
}

/// Keeps a validated value, or records why it was refused.
fn collect<T>(outcome: Result<T, FieldError>, problems: &mut FieldProblems) -> Option<T> {
    match outcome {
        Ok(value) => Some(value),
        Err(refused) => {
            problems.insert(refused.field.to_owned(), refused.problem.to_owned());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    // A test asserts by panicking, so the lints that forbid a panic in a service
    // have to be lifted here. Scoped to the test module, so production code still
    // cannot reach for any of them.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{
        EmailAddress, FullName, Notes, Plan, Seats, Signup, SignupId, SignupRequest, constants,
        validate,
    };
    use crate::events::DomainEvent;
    use std::time::SystemTime;
    use uuid::Uuid;

    const NAME: &str = "Ada Lovelace";
    const PADDED_NAME: &str = "  Ada Lovelace  ";
    const EMAIL: &str = "ada@example.com";
    const NOTE: &str = "Platform team first.";
    const EXPECTED_SUMMARY: &str = "Ada Lovelace on the Growth plan, 3 seat(s).";
    const SEATS: i32 = 3;
    const MANY_SEATS: i32 = 25;
    const NO_SEATS: i32 = 0;
    const NEGATIVE_SEATS: i32 = -4;
    const ASSIGNED_ID: i64 = 12;
    const FIRST_VALID_ID: i64 = 1;
    const BELOW_FIRST_VALID_ID: i64 = 0;
    const LATER_ID: i64 = 7;
    const UNKNOWN_PLAN: &str = "Platinum";
    const PADDED_EMAIL: &str = "  ada@example.com  ";
    const PADDED_NOTE: &str = "  a note  ";
    const TRIMMED_NOTE: &str = "a note";
    const EVERY_FIELD: usize = 6;
    const ONE_FIELD: usize = 1;
    const LETTER: &str = "a";

    /// Values a name, an address or a plan must refuse as absent.
    const BLANK: [&str; 3] = ["", " ", "\t"];
    const BLANK_OR_SPACES: [&str; 2] = ["", "   "];

    /// Spellings that are not shaped like an address.
    const MALFORMED_ADDRESSES: [&str; 9] = [
        "ada",
        "ada@",
        "@example.com",
        "ada@example",
        "ada example.com",
        "ada@@example.com",
        "ada@.com",
        "ada@example.",
        "a d a@example.com",
    ];

    /// Spellings that are.
    const WELL_FORMED_ADDRESSES: [&str; 3] = [
        "a@b.c",
        "first.last@sub.example.co.uk",
        "ada+tag@example.com",
    ];

    fn valid_request() -> SignupRequest {
        SignupRequest {
            full_name: NAME.to_owned(),
            email: EMAIL.to_owned(),
            plan: constants::PLAN_GROWTH.to_owned(),
            seats: Some(SEATS),
            notes: NOTE.to_owned(),
            accept_terms: true,
        }
    }

    fn signup() -> Signup {
        validate(&valid_request()).unwrap()
    }

    #[test]
    fn a_name_cannot_be_absent_and_stores_itself_trimmed() {
        assert_eq!(FullName::new(PADDED_NAME).unwrap().value(), NAME);
        for refused in BLANK {
            assert_eq!(
                FullName::new(refused).unwrap_err().problem,
                constants::MSG_NAME_REQUIRED
            );
        }
    }

    #[test]
    fn an_absent_address_reads_as_absent_not_as_malformed() {
        for refused in BLANK_OR_SPACES {
            assert_eq!(
                EmailAddress::new(refused).unwrap_err().problem,
                constants::MSG_EMAIL_REQUIRED
            );
        }
    }

    #[test]
    fn a_malformed_address_reads_as_malformed() {
        for refused in MALFORMED_ADDRESSES {
            assert_eq!(
                EmailAddress::new(refused).unwrap_err().problem,
                constants::MSG_EMAIL_INVALID,
                "{refused} is not shaped like an address"
            );
        }
    }

    #[test]
    fn an_address_shaped_like_one_is_accepted_and_trimmed() {
        assert_eq!(EmailAddress::new(PADDED_EMAIL).unwrap().value(), EMAIL);
        for accepted in WELL_FORMED_ADDRESSES {
            assert!(
                EmailAddress::new(accepted).is_ok(),
                "{accepted} is an address"
            );
        }
    }

    #[test]
    fn an_absent_plan_reads_as_absent_and_an_unknown_one_as_unknown() {
        for absent in BLANK_OR_SPACES {
            assert_eq!(
                Plan::new(absent).unwrap_err().problem,
                constants::MSG_PLAN_REQUIRED
            );
        }
        assert_eq!(
            Plan::new(UNKNOWN_PLAN).unwrap_err().problem,
            constants::MSG_PLAN_UNKNOWN
        );
    }

    #[test]
    fn every_plan_on_offer_is_accepted() {
        for plan in [Plan::Starter, Plan::Growth, Plan::Enterprise] {
            assert_eq!(Plan::new(plan.value()).unwrap(), plan);
        }
    }

    #[test]
    fn an_omitted_seat_count_defaults_and_an_explicit_zero_does_not() {
        assert_eq!(Seats::new(None).unwrap().value(), constants::DEFAULT_SEATS);
        assert_eq!(Seats::new(Some(MANY_SEATS)).unwrap().value(), MANY_SEATS);
        assert_eq!(
            Seats::new(Some(NO_SEATS)).unwrap_err().problem,
            constants::MSG_SEATS_INVALID,
            "a form asking for none is not a form that forgot to ask"
        );
        assert!(Seats::new(Some(NEGATIVE_SEATS)).is_err());
    }

    #[test]
    fn notes_are_optional_trimmed_and_capped() {
        assert_eq!(Notes::new("").unwrap().value(), "");
        assert_eq!(Notes::new(PADDED_NOTE).unwrap().value(), TRIMMED_NOTE);
        assert!(Notes::new(&LETTER.repeat(constants::MAX_NOTES_LENGTH)).is_ok());
        assert_eq!(
            Notes::new(&LETTER.repeat(constants::MAX_NOTES_LENGTH + 1))
                .unwrap_err()
                .problem,
            constants::MSG_NOTES_TOO_LONG
        );
    }

    #[test]
    fn an_identifier_refuses_the_value_below_the_first_valid_one() {
        assert_eq!(
            SignupId::new(FIRST_VALID_ID).unwrap().value(),
            FIRST_VALID_ID
        );
        assert_eq!(
            SignupId::new(BELOW_FIRST_VALID_ID).unwrap_err().code(),
            constants::CODE_INVALID_ID
        );
        assert_eq!(
            SignupId::new(LATER_ID).unwrap().to_string(),
            LATER_ID.to_string()
        );
    }

    #[test]
    fn a_complete_form_is_accepted() {
        let accepted = validate(&valid_request()).unwrap();

        assert_eq!(accepted.full_name.value(), NAME);
        assert_eq!(accepted.seats.value(), SEATS);
    }

    #[test]
    fn every_problem_is_reported_at_once_so_a_form_corrects_them_in_one_pass() {
        let empty = SignupRequest {
            seats: Some(NO_SEATS),
            notes: LETTER.repeat(constants::MAX_NOTES_LENGTH + 1),
            ..SignupRequest::default()
        };

        let problems = validate(&empty).unwrap_err().fields();

        assert_eq!(
            problems.len(),
            EVERY_FIELD,
            "all six, not the first one found"
        );
        for field in [
            constants::FIELD_FULL_NAME,
            constants::FIELD_EMAIL,
            constants::FIELD_PLAN,
            constants::FIELD_SEATS,
            constants::FIELD_NOTES,
            constants::FIELD_ACCEPT_TERMS,
        ] {
            assert!(problems.contains_key(field), "{field} is reported");
        }
    }

    #[test]
    fn refused_terms_alone_withhold_the_signup() {
        let refused = SignupRequest {
            accept_terms: false,
            ..valid_request()
        };

        let problems = validate(&refused).unwrap_err().fields();

        assert_eq!(problems.len(), ONE_FIELD);
        assert_eq!(
            problems.get(constants::FIELD_ACCEPT_TERMS).unwrap(),
            constants::MSG_TERMS_REQUIRED
        );
    }

    #[test]
    fn a_signup_reads_itself_back_in_one_sentence() {
        assert_eq!(signup().summary(), EXPECTED_SUMMARY);
    }

    #[test]
    fn a_signup_composes_its_own_event_carrying_primitives() {
        let event_id = Uuid::new_v4();
        let occurred_at = SystemTime::now();

        let recorded =
            signup().recorded(SignupId::new(ASSIGNED_ID).unwrap(), event_id, occurred_at);

        match recorded {
            DomainEvent::SignupRecorded {
                event_id: carried,
                occurred_at: when,
                signup_id,
                email,
                plan,
                seats,
            } => {
                assert_eq!(carried, event_id);
                assert_eq!(when, occurred_at);
                assert_eq!(signup_id, ASSIGNED_ID);
                assert_eq!(email, EMAIL);
                assert_eq!(plan, constants::PLAN_GROWTH);
                assert_eq!(seats, SEATS);
            }
            other @ DomainEvent::TaskCompleted { .. } => {
                panic!("a signup records a signup, not {other:?}")
            }
        }
    }
}
