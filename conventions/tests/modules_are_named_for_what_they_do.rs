//! Asserts that no module is named for a category instead of a job.
//!
//! A module called `util` tells a reader nothing, which means nobody can say
//! what belongs in it, which means everything does. The same goes for `helper`,
//! `common` and `misc`. `impl` is the other half of the same problem: it names a
//! module after a relationship to another module rather than after what the code
//! does.
//!
//! The Go sibling enforces this, and it is the one convention here that costs
//! nothing to follow and is almost never followed without a check.

mod support;

use syn::visit::Visit;

/// Names that describe a category rather than a job.
const SAYS_NOTHING: [&str; 10] = [
    "util", "utils", "helper", "helpers", "common", "misc", "shared", "base", "impl", "core",
];

const VIOLATION: &str = "{path} declares a module named {name}";
const PATH_TOKEN: &str = "{path}";
const NAME_TOKEN: &str = "{name}";

/// Module names declared in one file, at any depth.
#[derive(Default)]
struct ModuleNames {
    found: Vec<String>,
}

impl<'ast> Visit<'ast> for ModuleNames {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        self.found.push(module.ident.to_string());
        syn::visit::visit_item_mod(self, module);
    }
}

fn module_names_in(file: &syn::File) -> Vec<String> {
    let mut visitor = ModuleNames::default();
    visitor.visit_file(file);
    visitor.found
}

/// A file name is a module name too, so `src/util.rs` is caught as `mod util`
/// would be.
fn file_stem(path: &std::path::Path) -> Option<String> {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

#[test]
fn no_module_is_named_for_a_category() {
    let mut violations = Vec::new();

    for path in support::rust_sources() {
        let cited = support::relative(&path);
        let mut names = module_names_in(&support::parse(&path));
        names.extend(file_stem(&path));

        for name in names {
            if SAYS_NOTHING.contains(&name.as_str()) {
                violations.push(
                    VIOLATION
                        .replace(PATH_TOKEN, &cited)
                        .replace(NAME_TOKEN, &name),
                );
            }
        }
    }

    assert!(
        violations.is_empty(),
        "rename it after what the code in it does:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_rule_finds_a_module_named_for_a_category() {
    let offender: syn::File = syn::parse_quote! {
        mod util {
            pub fn trim(value: &str) -> &str {
                value.trim()
            }
        }
    };

    assert_eq!(
        module_names_in(&offender)
            .iter()
            .filter(|name| SAYS_NOTHING.contains(&name.as_str()))
            .count(),
        1,
        "a gate nobody has seen fail is a gate nobody knows works"
    );
}

#[test]
fn the_rule_finds_a_category_module_nested_inside_another() {
    let offender: syn::File = syn::parse_quote! {
        mod signups {
            mod helpers {}
        }
    };

    assert_eq!(
        module_names_in(&offender)
            .iter()
            .filter(|name| SAYS_NOTHING.contains(&name.as_str()))
            .count(),
        1,
        "nesting is where this name hides"
    );
}

#[test]
fn the_rule_accepts_a_module_named_for_what_it_holds() {
    let compliant: syn::File = syn::parse_quote! {
        mod signups {
            mod validation {}
        }
    };

    assert_eq!(
        module_names_in(&compliant)
            .iter()
            .filter(|name| SAYS_NOTHING.contains(&name.as_str()))
            .count(),
        0
    );
}
