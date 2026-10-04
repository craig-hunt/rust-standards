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
/// one. The empty string is the absence of a string. Naming these would produce
/// `ZERO` and `EMPTY`, which tell a reader nothing the literal did not, and the
/// noise would teach people to ignore the rule.
const MEANINGLESS_INTEGERS: [&str; 2] = ["0", "1"];
const EMPTY_STRING: &str = "";

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

    fn visit_lit(&mut self, literal: &'ast syn::Lit) {
        self.record(literal);
    }
}

/// Whether a literal is one the rule does not ask to be named.
fn excused(literal: &syn::Lit) -> bool {
    match literal {
        syn::Lit::Str(text) => text.value() == EMPTY_STRING,
        syn::Lit::Int(number) => MEANINGLESS_INTEGERS.contains(&number.base10_digits()),
        // A boolean is its own name, and a byte string or character literal
        // carrying meaning is caught as the string case would be.
        syn::Lit::Bool(_) => true,
        _ => false,
    }
}

fn describe(literal: &syn::Lit) -> String {
    match literal {
        syn::Lit::Str(text) => format!("{:?}", text.value()),
        syn::Lit::Int(number) => number.base10_digits().to_owned(),
        syn::Lit::Float(number) => number.base10_digits().to_owned(),
        syn::Lit::Char(character) => format!("{:?}", character.value()),
        other => format!("{:?}", other.span().start()),
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
        3,
        "the arm value, and both plan names"
    );
}
