//! Asserts that each crate depends on what it claims and nothing else.
//!
//! Cargo already enforces the direction: a crate cannot use a crate it does not
//! list, and a cycle is refused outright. That is stronger than the Go sibling's
//! hand-written check or the Java sibling's `ArchUnit` rules, both of which assert
//! after the fact what Cargo makes unrepresentable.
//!
//! What Cargo cannot say is which crates a layer is *allowed* to list. Nothing
//! stops somebody adding tokio to the domain, and the compiler would be content.
//! That is the question these tests answer.

mod support;

const DEPENDENCIES: &str = "dependencies";
const DEV_DEPENDENCIES: &str = "dev-dependencies";

const DOMAIN: &str = "domain";
const APPLICATION: &str = "application";
const INFRASTRUCTURE: &str = "infrastructure";
const WEB: &str = "web";
const CONVENTIONS: &str = "conventions";

/// What the domain is allowed to list.
///
/// `thiserror` derives an error's message, and `uuid` supplies a type Rust's
/// standard library does not. Both are value types that reach nothing: no
/// socket, no clock, no file. The list is short on purpose, and this test exists
/// so that it stays short by decision rather than by habit.
const DOMAIN_MAY_USE: [&str; 2] = ["thiserror", "uuid"];

/// What the application layer is allowed to add on top of the domain.
///
/// `thiserror` derives the message on the error a port returns, which is the
/// same justification the domain uses: a derive macro that writes a `Display`
/// implementation reaches nothing at runtime.
///
/// No runtime, no client, no framework. Any of those would travel into every
/// signature the domain reads and into every test, which is the whole reason
/// this list is asserted rather than assumed.
const APPLICATION_MAY_USE: [&str; 2] = [DOMAIN, "thiserror"];

const UNEXPECTED: &str = "{crate} lists {dependency}, which its layer may not use";
const CRATE_TOKEN: &str = "{crate}";
const DEPENDENCY_TOKEN: &str = "{dependency}";

const MISSING: &str = "{crate} does not list {dependency}";

const NO_TOOLS: usize = 0;

fn report(template: &str, crate_name: &str, dependency: &str) -> String {
    template
        .replace(CRATE_TOKEN, crate_name)
        .replace(DEPENDENCY_TOKEN, dependency)
}

#[test]
fn the_domain_lists_only_value_types() {
    let listed = support::dependencies(DOMAIN, DEPENDENCIES);

    let unexpected: Vec<String> = listed
        .iter()
        .filter(|dependency| !DOMAIN_MAY_USE.contains(&dependency.as_str()))
        .map(|dependency| report(UNEXPECTED, DOMAIN, dependency))
        .collect();

    assert!(
        unexpected.is_empty(),
        "a rule that can reach a database is a rule no test can pin down:\n{}",
        unexpected.join("\n")
    );
}

#[test]
fn the_domain_depends_on_no_crate_this_workspace_builds() {
    let listed = support::dependencies(DOMAIN, DEPENDENCIES);

    for inner in support::CRATES {
        assert!(
            !listed.contains(&inner.to_owned()),
            "{}",
            report(UNEXPECTED, DOMAIN, inner)
        );
    }
}

#[test]
fn the_application_layer_takes_on_no_framework() {
    let listed = support::dependencies(APPLICATION, DEPENDENCIES);

    let unexpected: Vec<String> = listed
        .iter()
        .filter(|dependency| !APPLICATION_MAY_USE.contains(&dependency.as_str()))
        .map(|dependency| report(UNEXPECTED, APPLICATION, dependency))
        .collect();

    assert!(unexpected.is_empty(), "{}", unexpected.join("\n"));
}

/// What only the two outer crates may reach for.
///
/// A database client, a connection pool, a JSON codec, a runtime or an HTTP
/// server anywhere else would put a dependency that opens a socket inside a
/// layer whose rules a test has to be able to pin down without one.
const REACHES_OUTSIDE_THE_PROCESS: [&str; 10] = [
    "postgres",
    "r2d2",
    "r2d2_postgres",
    "serde",
    "serde_json",
    "tokio",
    "hyper",
    "hyper-util",
    "http-body-util",
    "tracing-subscriber",
];

#[test]
fn only_the_outer_crates_reach_outside_the_process() {
    for crate_name in [DOMAIN, APPLICATION] {
        let listed = support::dependencies(crate_name, DEPENDENCIES);
        let unexpected: Vec<String> = listed
            .iter()
            .filter(|dependency| REACHES_OUTSIDE_THE_PROCESS.contains(&dependency.as_str()))
            .map(|dependency| report(UNEXPECTED, crate_name, dependency))
            .collect();

        assert!(unexpected.is_empty(), "{}", unexpected.join("\n"));
    }
}

#[test]
fn each_outer_layer_lists_the_layers_inside_it() {
    for (crate_name, inner) in [
        (APPLICATION, DOMAIN),
        (INFRASTRUCTURE, APPLICATION),
        (INFRASTRUCTURE, DOMAIN),
        (WEB, INFRASTRUCTURE),
        (WEB, APPLICATION),
        (WEB, DOMAIN),
    ] {
        let listed = support::dependencies(crate_name, DEPENDENCIES);
        assert!(
            listed.contains(&inner.to_owned()),
            "{}",
            report(MISSING, crate_name, inner)
        );
    }
}

#[test]
fn no_inner_layer_lists_an_outer_one() {
    for (inner, outer) in [
        (DOMAIN, APPLICATION),
        (DOMAIN, INFRASTRUCTURE),
        (DOMAIN, WEB),
        (APPLICATION, INFRASTRUCTURE),
        (APPLICATION, WEB),
        (INFRASTRUCTURE, WEB),
    ] {
        let listed = support::dependencies(inner, DEPENDENCIES);
        assert!(
            !listed.contains(&outer.to_owned()),
            "{}",
            report(UNEXPECTED, inner, outer)
        );
    }
}

#[test]
fn nothing_depends_on_the_conventions_crate() {
    for crate_name in support::CRATES {
        if crate_name == CONVENTIONS {
            continue;
        }
        for table in [DEPENDENCIES, DEV_DEPENDENCIES] {
            assert!(
                !support::dependencies(crate_name, table).contains(&CONVENTIONS.to_owned()),
                "the rules read the code; the code does not read the rules: {}",
                report(UNEXPECTED, crate_name, CONVENTIONS)
            );
        }
    }
}

#[test]
fn the_conventions_crate_carries_its_tools_as_development_dependencies() {
    let runtime = support::dependencies(CONVENTIONS, DEPENDENCIES);

    assert!(
        runtime.is_empty(),
        "nothing links against this crate, so a runtime dependency would ship a \
         parser to no purpose: {runtime:?}"
    );
    assert_ne!(
        support::dependencies(CONVENTIONS, DEV_DEPENDENCIES).len(),
        NO_TOOLS
    );
}
