//! Everything the service reads from its environment, checked once at startup.

use crate::constants::environment;

/// A setting the service cannot run without, and did not get.
#[derive(Debug, thiserror::Error)]
#[error("the service needs the {name} environment variable")]
pub struct MissingSetting {
    /// Which one.
    pub name: &'static str,
}

/// Where the database is, and nothing else.
///
/// Separate from [`Settings`] because the migrator needs exactly this. A migrator
/// reading the whole configuration would refuse to run without an API token it
/// never presents, and somebody would eventually set a placeholder token to get a
/// migration through.
#[derive(Debug, Clone)]
pub struct DatabaseSettings {
    /// A PostgreSQL connection string.
    pub url: String,
}

impl DatabaseSettings {
    /// Reads them from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`MissingSetting`] naming the variable that is absent or blank.
    pub fn from_environment() -> Result<Self, MissingSetting> {
        Self::from(|name| std::env::var(name).ok())
    }

    /// Reads them from any lookup, so a test supplies one it controls.
    ///
    /// A test cannot set process variables in Rust without racing every other
    /// test in the binary, which is why the lookup is a parameter.
    ///
    /// # Errors
    ///
    /// Returns [`MissingSetting`] naming the variable that is absent or blank.
    pub fn from<L: Fn(&str) -> Option<String>>(lookup: L) -> Result<Self, MissingSetting> {
        Ok(Self {
            url: required(&lookup, environment::DATABASE_URL)?,
        })
    }
}

/// Everything the API reads.
///
/// Checked here rather than where each value is used. A missing database URL
/// discovered on the first request is an outage; discovered at startup it is a
/// deployment that never went live.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Which port to listen on.
    pub port: u16,
    /// Where the database is.
    pub database: DatabaseSettings,
    /// The token `/api` expects.
    pub api_token: String,
}

impl Settings {
    const DEFAULT_PORT: u16 = 8080;

    /// Reads them from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`MissingSetting`] naming the variable that is absent or blank.
    pub fn from_environment() -> Result<Self, MissingSetting> {
        Self::from(|name| std::env::var(name).ok())
    }

    /// Reads them from any lookup, so a test supplies one it controls.
    ///
    /// # Errors
    ///
    /// Returns [`MissingSetting`] naming the variable that is absent or blank.
    pub fn from<L: Fn(&str) -> Option<String>>(lookup: L) -> Result<Self, MissingSetting> {
        Ok(Self {
            port: lookup(environment::PORT)
                .and_then(|configured| configured.trim().parse().ok())
                .unwrap_or(Self::DEFAULT_PORT),
            database: DatabaseSettings::from(&lookup)?,
            api_token: required(&lookup, environment::API_TOKEN)?,
        })
    }
}

/// Reads a setting that has no sensible default.
///
/// A blank value counts as absent. An empty string in a deployment file is
/// somebody's placeholder, and accepting one as a token means starting a service
/// whose token check compares against nothing.
fn required<L: Fn(&str) -> Option<String>>(
    lookup: &L,
    name: &'static str,
) -> Result<String, MissingSetting> {
    lookup(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(MissingSetting { name })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{DatabaseSettings, Settings};
    use crate::constants::environment;
    use std::collections::BTreeMap;

    const URL: &str = "postgresql://standards@localhost/standards";
    const TOKEN: &str = "a-token";
    const CONFIGURED_PORT: &str = "9090";
    const PADDED_PORT: &str = "  9090  ";
    const NOT_A_NUMBER: &str = "eight thousand";
    const EXPECTED_PORT: u16 = 9090;
    const DEFAULT_PORT: u16 = 8080;

    /// What a deployment file leaves behind when somebody means to fill it in.
    const BLANK_VALUES: [&str; 2] = ["", "   "];

    fn complete() -> BTreeMap<&'static str, String> {
        BTreeMap::from([
            (environment::DATABASE_URL, URL.to_owned()),
            (environment::API_TOKEN, TOKEN.to_owned()),
        ])
    }

    fn read(from: &BTreeMap<&'static str, String>) -> Result<Settings, super::MissingSetting> {
        Settings::from(|name| from.get(name).cloned())
    }

    #[test]
    fn a_complete_environment_is_read() {
        let settings = read(&complete()).unwrap();

        assert_eq!(settings.database.url, URL);
        assert_eq!(settings.api_token, TOKEN);
    }

    #[test]
    fn the_port_defaults_because_a_port_has_a_sensible_default_and_a_token_does_not() {
        assert_eq!(read(&complete()).unwrap().port, DEFAULT_PORT);
    }

    #[test]
    fn a_configured_port_is_read_and_whitespace_around_it_tolerated() {
        for configured in [CONFIGURED_PORT, PADDED_PORT] {
            let mut environment = complete();
            environment.insert(environment::PORT, configured.to_owned());

            assert_eq!(read(&environment).unwrap().port, EXPECTED_PORT);
        }
    }

    #[test]
    fn a_port_that_is_not_a_number_falls_back_rather_than_refusing_to_start() {
        let mut environment = complete();
        environment.insert(environment::PORT, NOT_A_NUMBER.to_owned());

        assert_eq!(
            read(&environment).unwrap().port,
            DEFAULT_PORT,
            "a port has a default; refusing to start over one would be worse"
        );
    }

    #[test]
    fn a_missing_variable_is_named_rather_than_reported_as_something_absent() {
        for missing in [environment::DATABASE_URL, environment::API_TOKEN] {
            let mut environment = complete();
            environment.remove(missing);

            let refused = read(&environment).unwrap_err();

            assert_eq!(refused.name, missing);
            assert!(refused.to_string().contains(missing));
        }
    }

    #[test]
    fn a_blank_value_counts_as_absent() {
        for blank in BLANK_VALUES {
            let mut environment = complete();
            environment.insert(environment::API_TOKEN, blank.to_owned());

            assert_eq!(read(&environment).unwrap_err().name, environment::API_TOKEN);
        }
    }

    #[test]
    fn the_migrator_reads_the_database_without_demanding_a_token() {
        let mut environment = complete();
        environment.remove(environment::API_TOKEN);

        let database = DatabaseSettings::from(|name| environment.get(name).cloned()).unwrap();

        assert_eq!(database.url, URL);
    }

    #[test]
    fn the_migrator_still_names_the_database_setting_it_needs() {
        let environment: BTreeMap<&'static str, String> = BTreeMap::new();

        let refused = DatabaseSettings::from(|name| environment.get(name).cloned()).unwrap_err();

        assert_eq!(refused.name, environment::DATABASE_URL);
    }
}
