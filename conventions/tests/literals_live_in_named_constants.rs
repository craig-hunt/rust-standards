//! Asserts the rule that a literal with a meaning carries a name.
//!
//! Written as a test walking the syntax tree, which is the mechanism the ticket
//! asked to have named. The alternatives were weighed and rejected:
//!
//! - **A clippy lint.** Clippy has no lint for this. `restriction` offers
//!   nothing that distinguishes a literal in a constant from one in a function
//!   body, and a lint that cannot make that distinction would fire on every
//!   constant in the repository.
//! - **A dylint lint.** A custom lint runs inside a compiler driver pinned to one
//!   toolchain, so the repository would carry a second toolchain pin that has to
//!   move in step with `rust-toolchain.toml`. The Java sibling's round taught the
//!   same lesson from the other direction: an enforcer wired into the build costs
//!   more than it returns when a test says the same thing.
//!
//! A failing test names the file and the line, runs under the same command as
//! everything else, and can be read by somebody who has never written a lint.
//! The Go sibling reaches the same conclusion and walks its AST from a test.

mod support;

use proc_macro2::LineColumn;
use syn::visit::Visit;

/// Values that carry no meaning wherever they appear.
///
/// Zero and one are counting, not facts: an index starts at zero and a step is
/// one. The empty string is the absence of a string, and so is a string holding
/// nothing but whitespace, which is a line break or a separator. Naming these
/// would produce `ZERO` and `EMPTY`, which tell a reader nothing the literal did
/// not, and the noise would teach people to ignore the rule.
const MEANINGLESS_INTEGERS: [&str; 2] = ["0", "1"];

/// Macros carrying a template or a diagnostic sentence in a known argument.
///
/// A format template carries placeholders and punctuation, and an assertion
/// message exists to be read by whoever the assertion fails in front of. Neither
/// states a value another line could duplicate or branch on. Every other literal
/// a macro carries gets the rule, which is the point of looking inside macros at
/// all.
const MACROS_CARRYING_A_MESSAGE: [&str; 23] = [
    "assert",
    "assert_eq",
    "assert_ne",
    "debug",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "eprint",
    "eprintln",
    "error",
    "format",
    "format_args",
    "info",
    "panic",
    "print",
    "println",
    "todo",
    "trace",
    "unimplemented",
    "unreachable",
    "warn",
    "write",
    "writeln",
];

/// Macros comparing two values before any message.
///
/// Their message cannot be the first string argument, because the first two
/// arguments are the values under comparison and either may be a string. Reading
/// the message position from the macro's own shape is what keeps
/// `assert_eq!(answer, "Enterprise")` under the rule.
const MACROS_COMPARING_BEFORE_THE_MESSAGE: [&str; 4] = [
    "assert_eq",
    "assert_ne",
    "debug_assert_eq",
    "debug_assert_ne",
];

/// Where a message can first appear in a comparison assertion.
const AFTER_THE_COMPARED_VALUES: usize = 2;

/// Macros whose argument has to be a literal, because the language says so.
///
/// `env!` reads its name at compile time and a `const` cannot stand in for it.
/// Flagging these would state a rule the language makes impossible to follow,
/// and a rule nobody can satisfy is a rule everybody learns to suppress.
const MACROS_TAKING_A_COMPILE_TIME_LITERAL: [&str; 8] = [
    "cfg",
    "concat",
    "env",
    "include",
    "include_bytes",
    "include_str",
    "option_env",
    "stringify",
];

/// Macros building source rather than running it.
///
/// `parse_quote!` holds a specimen: the offenders these tests construct to prove
/// the rule fails have to contain bare literals, because a constant interpolated
/// into the specimen would build a path node and the rule would have nothing to
/// find. The repository publishes no procedural macro, so this excuses test
/// fixtures and nothing that ships.
const MACROS_HOLDING_SOURCE_SPECIMENS: [&str; 4] = [
    "parse_quote",
    "parse_quote_spanned",
    "quote",
    "quote_spanned",
];

const VIOLATION: &str = "{path}:{line} holds the literal {literal} outside a named constant";
const FIX: &str = "move each of these into a const whose name says what it means";

const PATH_TOKEN: &str = "{path}";
const LINE_TOKEN: &str = "{line}";
const LITERAL_TOKEN: &str = "{literal}";

/// Literals found in a position the rule does not excuse.
#[derive(Default)]
struct LooseLiterals {
    found: Vec<(LineColumn, String)>,
}

impl LooseLiterals {
    fn record(&mut self, literal: &syn::Lit) {
        if excused(literal) {
            return;
        }
        self.found.push((literal.span().start(), describe(literal)));
    }

    /// Applies the rule to the tokens a macro was handed.
    ///
    /// `syn` parses a macro invocation as an unexamined token stream, so the
    /// visitor never reaches inside one. Without this, `vec!["Enterprise"]` and
    /// `format!("{}", 86_400)` state a value the rule exists to catch while the
    /// gate reports a clean sweep.
    fn record_macro(&mut self, invocation: &syn::Macro) {
        self.record_invocation(&macro_name(invocation), invocation.tokens.clone());
    }

    fn record_invocation(&mut self, name: &str, tokens: proc_macro2::TokenStream) {
        if MACROS_TAKING_A_COMPILE_TIME_LITERAL.contains(&name)
            || MACROS_HOLDING_SOURCE_SPECIMENS.contains(&name)
        {
            return;
        }

        let arguments = split_on_commas(tokens);
        let message = message_argument(name, &arguments);
        for (position, argument) in arguments.iter().enumerate() {
            if Some(position) == message {
                continue;
            }
            self.record_tokens(argument.clone());
        }
    }

    /// Walks a token stream, treating a nested invocation as an invocation.
    ///
    /// A macro inside a macro arrives as plain tokens, so `visit_macro` never
    /// sees it. Without reading the `name ! (...)` shape here, the `env!` inside
    /// a `panic!` message loses its exemption and the rule demands a constant the
    /// language will not accept.
    fn record_tokens(&mut self, tokens: proc_macro2::TokenStream) {
        const BANG: char = '!';

        let mut tokens = tokens.into_iter().peekable();
        while let Some(token) = tokens.next() {
            match token {
                proc_macro2::TokenTree::Ident(name) => {
                    let invoked = matches!(
                        tokens.peek(),
                        Some(proc_macro2::TokenTree::Punct(punctuation))
                            if punctuation.as_char() == BANG
                    );
                    if !invoked {
                        continue;
                    }
                    tokens.next();
                    if let Some(proc_macro2::TokenTree::Group(group)) = tokens.peek().cloned() {
                        tokens.next();
                        self.record_invocation(&name.to_string(), group.stream());
                    }
                }
                proc_macro2::TokenTree::Group(group) => self.record_tokens(group.stream()),
                proc_macro2::TokenTree::Literal(literal) => self.record(&syn::Lit::new(literal)),
                proc_macro2::TokenTree::Punct(_) => {}
            }
        }
    }
}

fn macro_name(invocation: &syn::Macro) -> String {
    invocation
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_default()
}

/// Splits a macro's tokens into its arguments, at the commas that separate them.
///
/// Only the commas at the top level separate arguments: one inside a call or an
/// array belongs to that expression, and a split on every comma would read
/// `vec![a, b]` passed to `assert_eq!` as two arguments and move the message
/// position.
fn split_on_commas(tokens: proc_macro2::TokenStream) -> Vec<proc_macro2::TokenStream> {
    const SEPARATOR: char = ',';

    let mut arguments = Vec::new();
    let mut current = proc_macro2::TokenStream::new();
    for token in tokens {
        match &token {
            proc_macro2::TokenTree::Punct(punctuation) if punctuation.as_char() == SEPARATOR => {
                arguments.push(std::mem::take(&mut current));
            }
            _ => current.extend(std::iter::once(token)),
        }
    }
    if !current.is_empty() {
        arguments.push(current);
    }
    arguments
}

/// Which argument holds the message, when the macro has one.
///
/// The message is the first argument that is one string literal and nothing
/// else, searched from the position the macro's shape allows. A comparison
/// assertion compares its first two arguments, so the search starts past them.
fn message_argument(name: &str, arguments: &[proc_macro2::TokenStream]) -> Option<usize> {
    if !MACROS_CARRYING_A_MESSAGE.contains(&name) {
        return None;
    }

    let first_possible = if MACROS_COMPARING_BEFORE_THE_MESSAGE.contains(&name) {
        AFTER_THE_COMPARED_VALUES
    } else {
        0
    };

    arguments
        .iter()
        .enumerate()
        .skip(first_possible)
        .find(|(_, argument)| is_one_string_literal(argument))
        .map(|(position, _)| position)
}

fn is_one_string_literal(argument: &proc_macro2::TokenStream) -> bool {
    let mut tokens = argument.clone().into_iter();
    let Some(proc_macro2::TokenTree::Literal(only)) = tokens.next() else {
        return false;
    };
    tokens.next().is_none() && matches!(syn::Lit::new(only), syn::Lit::Str(_))
}

impl<'ast> Visit<'ast> for LooseLiterals {
    /// A `const` or `static` item is the named constant this rule asks for, so
    /// nothing inside one is a violation. Not descending is what makes the rule
    /// expressible at all: every constant in the repository is a literal.
    fn visit_item_const(&mut self, _item: &'ast syn::ItemConst) {}

    fn visit_item_static(&mut self, _item: &'ast syn::ItemStatic) {}

    /// An associated const is a different syntax node from a free-standing one,
    /// and skipping only the latter left this rule reporting the length of every
    /// `const ALL: [Self; 3]` in the repository. That is the shape of mistake a
    /// rule about source text makes quietly, which is why these tests construct
    /// offenders and compliant files by hand rather than trusting the sweep.
    fn visit_impl_item_const(&mut self, _item: &'ast syn::ImplItemConst) {}

    fn visit_trait_item_const(&mut self, _item: &'ast syn::TraitItemConst) {}

    /// Likewise an attribute. `#[allow(...)]`, a doc comment and a test's
    /// parameter list all carry literals that have nowhere else to live.
    fn visit_attribute(&mut self, _attribute: &'ast syn::Attribute) {}

    /// A macro invocation reaches the visitor as a token stream nothing descends
    /// into, so the rule has to walk the tokens itself.
    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        self.record_macro(invocation);
    }

    fn visit_lit(&mut self, literal: &'ast syn::Lit) {
        self.record(literal);
    }
}

/// Whether a literal is one the rule does not ask to be named.
fn excused(literal: &syn::Lit) -> bool {
    match literal {
        syn::Lit::Str(text) => text.value().trim().is_empty(),
        syn::Lit::ByteStr(bytes) => bytes.value().is_empty(),
        syn::Lit::Int(number) => MEANINGLESS_INTEGERS.contains(&number.base10_digits()),
        // A boolean is its own name. A character or byte carrying meaning is
        // caught as the string case is.
        syn::Lit::Bool(_) => true,
        _ => false,
    }
}

/// How a violation names the literal it found.
///
/// Every kind renders as the source wrote it. An earlier version fell through to
/// the span for anything but a string, an integer, a float or a character, so a
/// byte literal was reported as a line and a column twice over and a reader had
/// no idea what to look for.
fn describe(literal: &syn::Lit) -> String {
    match literal {
        syn::Lit::Str(text) => format!("{:?}", text.value()),
        syn::Lit::Int(number) => number.base10_digits().to_owned(),
        syn::Lit::Float(number) => number.base10_digits().to_owned(),
        syn::Lit::Char(character) => format!("{:?}", character.value()),
        syn::Lit::Byte(byte) => format!("{:?}", char::from(byte.value())),
        syn::Lit::ByteStr(bytes) => format!("{:?}", String::from_utf8_lossy(&bytes.value())),
        syn::Lit::Verbatim(raw) => raw.to_string(),
        other => format!("{other:?}"),
    }
}

fn loose_literals_in(file: &syn::File) -> Vec<(LineColumn, String)> {
    let mut visitor = LooseLiterals::default();
    visitor.visit_file(file);
    visitor.found
}

#[test]
fn every_literal_that_means_something_sits_in_a_named_constant() {
    let mut violations = Vec::new();

    for path in support::rust_sources() {
        let parsed = support::parse(&path);
        for (position, literal) in loose_literals_in(&parsed) {
            violations.push(
                VIOLATION
                    .replace(PATH_TOKEN, &support::relative(&path))
                    .replace(LINE_TOKEN, &position.line.to_string())
                    .replace(LITERAL_TOKEN, &literal),
            );
        }
    }

    assert!(violations.is_empty(), "{FIX}:\n{}", violations.join("\n"));
}

#[test]
fn the_rule_finds_a_literal_a_function_body_introduces() {
    let offender: syn::File = syn::parse_quote! {
        fn plan() -> &'static str {
            "Enterprise"
        }
    };

    assert_eq!(
        loose_literals_in(&offender).len(),
        1,
        "a gate nobody has seen fail is a gate nobody knows works"
    );
}

#[test]
fn the_rule_accepts_a_literal_that_already_is_a_named_constant() {
    let compliant: syn::File = syn::parse_quote! {
        const PLAN: &str = "Enterprise";

        fn plan() -> &'static str {
            PLAN
        }
    };

    assert_eq!(loose_literals_in(&compliant).len(), 0);
}

#[test]
fn the_rule_excuses_what_carries_no_meaning() {
    let trivial: syn::File = syn::parse_quote! {
        fn counting(values: &[u8]) -> usize {
            let mut total = 0;
            for _ in values {
                total += 1;
            }
            if total == 0 { String::new().len() } else { total }
        }
    };

    assert_eq!(loose_literals_in(&trivial).len(), 0);
}

#[test]
fn the_rule_finds_a_literal_a_macro_carried_past_the_syntax_tree() {
    const THE_PLAN_NAME_AND_THE_SEAT_COUNT: usize = 2;

    let offender: syn::File = syn::parse_quote! {
        fn plans() -> Vec<(&'static str, u32)> {
            vec![("Enterprise", 250)]
        }
    };

    assert_eq!(
        loose_literals_in(&offender).len(),
        THE_PLAN_NAME_AND_THE_SEAT_COUNT,
        "syn hands a macro over as tokens, so a rule trusting the tree alone sees none of this"
    );
}

#[test]
fn the_rule_reads_the_value_a_comparison_assertion_compares_against() {
    const THE_EXPECTED_PLAN_NAME: usize = 1;

    let offender: syn::File = syn::parse_quote! {
        fn checked(answer: &str) {
            assert_eq!(answer, "Enterprise", "the plan the account carries");
        }
    };

    assert_eq!(
        loose_literals_in(&offender).len(),
        THE_EXPECTED_PLAN_NAME,
        "the message is excused and the compared value is not"
    );
}

#[test]
fn the_rule_excuses_a_template_and_a_message_because_neither_states_a_value() {
    let diagnostics: syn::File = syn::parse_quote! {
        fn reported(seats: u32, limit: u32) {
            tracing::warn!(seats, "more seats than the plan allows");
            assert!(seats <= limit, "a plan cannot be oversold");
            let _ = format!("{seats} of {limit}");
        }
    };

    assert_eq!(loose_literals_in(&diagnostics).len(), 0);
}

#[test]
fn the_rule_excuses_what_the_language_demands_be_a_literal() {
    let compiled: syn::File = syn::parse_quote! {
        fn where_this_was_built() -> &'static str {
            env!("CARGO_MANIFEST_DIR")
        }
    };

    assert_eq!(
        loose_literals_in(&compiled).len(),
        0,
        "env! reads its name at compile time and no const can stand in for it"
    );
}

#[test]
fn the_rule_excuses_an_attribute_because_a_literal_there_has_nowhere_else_to_live() {
    let annotated: syn::File = syn::parse_quote! {
        #[doc = "a sentence"]
        #[allow(clippy::redundant_clone)]
        fn annotated() {}
    };

    assert_eq!(loose_literals_in(&annotated).len(), 0);
}

#[test]
fn the_rule_finds_a_literal_a_static_item_hides_behind() {
    const THE_ARM_VALUE_AND_BOTH_PLAN_NAMES: usize = 3;

    let offender: syn::File = syn::parse_quote! {
        fn plan(choice: u8) -> &'static str {
            match choice {
                7 => "Enterprise",
                _ => "Starter",
            }
        }
    };

    assert_eq!(
        loose_literals_in(&offender).len(),
        THE_ARM_VALUE_AND_BOTH_PLAN_NAMES
    );
}
