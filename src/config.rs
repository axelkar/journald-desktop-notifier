use std::collections::hash_map::Entry;
use std::ffi::CStr;
use std::ops::Deref;
use std::path::Path;
use std::{collections::HashMap, hash::Hash};

use color_eyre::eyre::{Context, Result, eyre};
use serde::{Deserialize, Deserializer};
use systemd::journal::JournalRef;

fn deserialize_regex<'de, D>(d: D) -> std::result::Result<regex::bytes::Regex, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(d).map_err(serde::de::Error::custom)?;
    s.parse().map_err(serde::de::Error::custom)
}

#[derive(Deserialize)]
struct Regex(#[serde(deserialize_with = "deserialize_regex")] regex::bytes::Regex);
impl Deref for Regex {
    type Target = regex::bytes::Regex;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::fmt::Debug for Regex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}
impl PartialEq for Regex {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

/// Note: This strips ANSI escapes
#[expect(
    clippy::borrowed_box,
    reason = "Impossible due to the current borrow checker"
)]
pub fn get_journal_field<'config: 'field_name, 'field_name, 'cache>(
    field_name: &'config Box<CStr>,
    field_cache: &'cache mut HashMap<&'field_name Box<CStr>, Vec<u8>>,
    journal: &mut JournalRef,
) -> Result<Option<&'cache [u8]>> {
    match field_cache.entry(field_name) {
        Entry::Occupied(o) => Ok(Some(o.into_mut())),
        Entry::Vacant(v) => Ok(Some(
            v.insert(
                match journal
                    .get_data(field_name.as_ref())
                    .wrap_err_with(|| {
                        eyre!(
                            "Failed to get field {} of a journal entry",
                            field_name.to_string_lossy()
                        )
                    })?
                    .and_then(|field| Some(field.value()?.to_owned()))
                {
                    None => {
                        return Ok(None);
                    }
                    Some(data) => strip_ansi_escapes::strip(data),
                },
            ),
        )),
    }
}

fn default_id_fields() -> Vec<String> {
    vec![
        "SYSLOG_IDENTIFIER".to_owned(),
        "_SYSTEMD_USER_UNIT".to_owned(),
    ]
}

#[derive(serde::Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    /// Used for deciding the appname field of the notification.
    #[serde(default = "default_id_fields")]
    pub id_fields: Vec<String>,
    /// Specifies the rules to notify messages.
    #[serde(default, rename = "match")]
    pub matchers: Vec<Notify>,
}
impl Config {
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let buf = fs_err::read_to_string(path)?;

        // Heuristic for good error messages
        if buf.bytes().next() == Some(b'{') {
            // JSON
            toml::from_str(&buf)
                .or_else(|_| serde_json::from_str(&buf).map_err(Into::<color_eyre::Report>::into))
        } else {
            serde_json::from_str(&buf)
                .or_else(|_| toml::from_str(&buf).map_err(Into::<color_eyre::Report>::into))
        }
        .wrap_err("Failed to parse config")
    }
}

/// All regexes and zero ignores must match for this level to notify.
#[derive(serde::Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Notify {
    #[serde(default, rename = "ignore")]
    pub ignore_rules: Vec<Ignore>,

    /// Add a prefix.
    #[serde(default)]
    prefix: Option<String>,

    /// Regex for capturing the content of the message (group 1 is used). Default is equivalent to `(?s:.)*`.
    #[serde(default)]
    content_regex: Option<Regex>,

    #[serde(flatten, deserialize_with = "deserialize_matcher_hashmap")]
    field_regexes: HashMap<Box<CStr>, Regex>,
}

#[derive(Default)]
pub struct NotifyResultAllowed {
    pub prefix: Option<String>,
    pub content_regex: Option<regex::bytes::Regex>,
}
pub enum NotifyResult {
    NotifyAllowed(NotifyResultAllowed),
    Ignored,
}

impl Notify {
    #[expect(
        clippy::borrowed_box,
        reason = "Impossible due to the current borrow checker"
    )]
    pub fn allows_notify<'config: 'field_name, 'field_name>(
        &'config self,
        field_cache: &mut HashMap<&'field_name Box<CStr>, Vec<u8>>,
        journal: &mut JournalRef,
    ) -> Result<NotifyResult> {
        for (field_name, regex) in &self.field_regexes {
            let Some(data) = get_journal_field(field_name, field_cache, journal)? else {
                return Ok(NotifyResult::Ignored);
            };

            if !regex.is_match(data) {
                return Ok(NotifyResult::Ignored);
            }
        }

        let mut first_prefix = None;
        let mut first_content_regex = None;
        for ignore_rule in &self.ignore_rules {
            let res = ignore_rule.ignores(field_cache, journal)?;
            match res {
                NotifyResult::NotifyAllowed(allowed) => {
                    first_prefix = first_prefix.or(allowed.prefix);
                    first_content_regex = first_content_regex.or(allowed.content_regex);
                }
                // Short-circuit if ignored
                NotifyResult::Ignored => return Ok(NotifyResult::Ignored),
            }
        }

        Ok(NotifyResult::NotifyAllowed(NotifyResultAllowed {
            prefix: first_prefix.or_else(|| self.prefix.clone()),
            content_regex: first_content_regex
                .or_else(|| self.content_regex.as_ref().map(|a| a.0.clone())),
        }))
    }
}

/// All regexes and zero notify exceptions must match for this level to ignore.
#[derive(serde::Deserialize, Debug)]
pub struct Ignore {
    #[serde(default, rename = "except-notify")]
    pub notify_exceptions: Vec<Notify>,
    #[serde(flatten, deserialize_with = "deserialize_matcher_hashmap")]
    field_regexes: HashMap<Box<CStr>, Regex>,
}

impl PartialEq for Ignore {
    fn eq(&self, other: &Self) -> bool {
        self.notify_exceptions == other.notify_exceptions
            && self.field_regexes == other.field_regexes
    }
}

impl Ignore {
    #[expect(
        clippy::borrowed_box,
        reason = "Impossible due to the current borrow checker"
    )]
    pub fn ignores<'config: 'field_name, 'field_name>(
        &'config self,
        field_cache: &mut HashMap<&'field_name Box<CStr>, Vec<u8>>,
        journal: &mut JournalRef,
    ) -> Result<NotifyResult> {
        for (field_name, regex) in &self.field_regexes {
            let Some(data) = get_journal_field(field_name, field_cache, journal)? else {
                return Ok(NotifyResult::NotifyAllowed(NotifyResultAllowed::default()));
            };

            if !regex.is_match(data) {
                return Ok(NotifyResult::NotifyAllowed(NotifyResultAllowed::default()));
            }
        }

        for notify in &self.notify_exceptions {
            let res = notify.allows_notify(field_cache, journal)?;
            match res {
                NotifyResult::Ignored => {}
                // Short-circuit if allowed
                res @ NotifyResult::NotifyAllowed(_) => return Ok(res),
            }
        }

        Ok(NotifyResult::Ignored)
    }
}

fn deserialize_matcher_hashmap<'de, D>(
    d: D,
) -> std::result::Result<HashMap<Box<CStr>, Regex>, D::Error>
where
    D: Deserializer<'de>,
{
    fn deserialize_key<'de, D>(d: D) -> std::result::Result<Box<CStr>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Box<CStr> = Deserialize::deserialize(d).map_err(serde::de::Error::custom)?;
        if !s
            .to_bytes()
            .iter()
            .all(|b| matches!(b, b'A'..=b'Z' | b'0'..=b'9' | b'_'))
        {
            return Err(serde::de::Error::custom(format!(
                "Must be only uppercase ASCII and underscores: {}",
                s.to_string_lossy()
            )));
        }
        Ok(s)
    }

    #[derive(Deserialize, Hash, Eq, PartialEq)]
    struct WrapperKey(#[serde(deserialize_with = "deserialize_key")] Box<CStr>);

    let dict: HashMap<WrapperKey, Regex> = Deserialize::deserialize(d)?;
    Ok(dict.into_iter().map(|(WrapperKey(k), v)| (k, v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deser_json() {
        Config::read("test_data/config.json").unwrap();
    }
    #[test]
    fn test_deser_toml() {
        Config::read("test_data/config.toml").unwrap();
    }
    #[test]
    fn test_deser_json_toml_eq() {
        let config_json = Config::read("test_data/config.json").unwrap();
        let config_toml = Config::read("test_data/config.toml").unwrap();
        assert_eq!(config_json, config_toml);
    }
    #[test]
    fn test_deser_not_found() {
        let _ = Config::read("test_data/notfound").unwrap_err();
    }
}
