use std::collections::hash_map::Entry;
use std::ffi::CStr;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::ops::Deref;
use std::path::PathBuf;
use std::{collections::HashMap, hash::Hash};

use color_eyre::eyre::{Context, Result, eyre};
use serde::{Deserialize, Deserializer};
use systemd::journal::JournalRef;
use try_iterator::prelude::TryIterator;

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

#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct Config {
    /// Specifies the rules to notify messages.
    #[serde(default, rename = "match")]
    pub matchers: Vec<Deny>,
}
impl Config {
    pub fn read(path: impl Into<PathBuf>) -> Result<Self> {
        let mut f = BufReader::new(fs_err::File::open(path)?);
        serde_json::from_reader(&mut f)
            .or_else(|_| {
                f.seek(SeekFrom::Start(0))?;
                let mut buf = String::new();
                f.read_to_string(&mut buf)?;
                toml::from_str(&buf).map_err(Into::<color_eyre::Report>::into)
            })
            .wrap_err("Failed to parse config")
    }
}

/// All regexes and zero allows must match for this level to deny.
#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct Deny {
    #[serde(default, rename = "allow")]
    pub allow_rules: Vec<Allow>,
    #[serde(flatten, deserialize_with = "deserialize_matcher_hashmap")]
    field_regexes: HashMap<Box<CStr>, Regex>,
}

impl Deny {
    #[expect(
        clippy::borrowed_box,
        reason = "Impossible due to the current borrow checker"
    )]
    pub fn denies<'config: 'field_name, 'field_name>(
        &'config self,
        field_cache: &mut HashMap<&'field_name Box<CStr>, Vec<u8>>,
        journal: &mut JournalRef,
    ) -> Result<bool> {
        for (field_name, regex) in &self.field_regexes {
            let data = match field_cache.entry(field_name) {
                Entry::Occupied(o) => o.into_mut(),
                Entry::Vacant(v) => v.insert(
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
                        None => return Ok(false),
                        Some(data) => data,
                    },
                ),
            };

            if !regex.is_match(data) {
                return Ok(false);
            }
        }
        Ok(!self
            .allow_rules
            .iter()
            .try_any(|allow| allow.allows(field_cache, journal))?)
    }
}

/// All regexes and zero denies must match for this level to allow.
#[derive(serde::Deserialize, Debug)]
pub struct Allow {
    #[serde(default, rename = "deny")]
    pub deny_rules: Vec<Deny>,
    #[serde(flatten, deserialize_with = "deserialize_matcher_hashmap")]
    field_regexes: HashMap<Box<CStr>, Regex>,
}

impl PartialEq for Allow {
    fn eq(&self, other: &Self) -> bool {
        self.deny_rules == other.deny_rules && self.field_regexes == other.field_regexes
    }
}
impl Allow {
    #[expect(
        clippy::borrowed_box,
        reason = "Impossible due to the current borrow checker"
    )]
    pub fn allows<'config: 'field_name, 'field_name>(
        &'config self,
        field_cache: &mut HashMap<&'field_name Box<CStr>, Vec<u8>>,
        journal: &mut JournalRef,
    ) -> Result<bool> {
        for (field_name, regex) in &self.field_regexes {
            let data = match field_cache.entry(field_name) {
                Entry::Occupied(o) => o.into_mut(),
                Entry::Vacant(v) => v.insert(
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
                        None => return Ok(false),
                        Some(data) => data,
                    },
                ),
            };

            if !regex.is_match(data) {
                return Ok(false);
            }
        }
        Ok(!self
            .deny_rules
            .iter()
            .try_any(|deny| deny.denies(field_cache, journal))?)
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
