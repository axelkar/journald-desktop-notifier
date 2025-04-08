#![deny(unused_crate_dependencies, clippy::pedantic)]

mod config;

use config::Config;
use itertools::Itertools;
use std::{collections::HashMap, process::ExitCode};
use try_iterator::prelude::TryIterator;

use color_eyre::eyre::{Context, Result};
use systemd::{JournalWaitResult, id128::Id128, journal::JournalRef};


fn main() -> Result<ExitCode> {
    fn next_entry(journal: &mut JournalRef) -> Result<Option<()>> {
        Ok((journal.next()? != 0).then_some(()))
    }

    color_eyre::install()?;

    let Some(config_path) = std::env::args_os().nth(1) else {
        print!(
            "{}",
            concat!(
                "Usage: ",
                env!("CARGO_PKG_NAME"),
                " CONFIG\n\nReport issues at ",
                env!("CARGO_PKG_REPOSITORY"),
                "/issues\nView possible fields using `journalctl -o verbose`\n"
            )
        );
        return Ok(ExitCode::FAILURE);
    };

    let config = Config::read(&config_path)?;

    let mut journal = systemd::journal::OpenOptions::default().open()?;
    journal.match_add("_BOOT_ID", Id128::from_boot()?.to_string())?;

    loop {
        if next_entry(&mut journal)?.is_none() {
            loop {
                match journal.wait(None)? {
                    JournalWaitResult::Nop => {
                        println!("3");
                        continue;
                    } // Should be unreachable
                    JournalWaitResult::Append | JournalWaitResult::Invalidate => {
                        if next_entry(&mut journal)?.is_some() {
                            break;
                        }
                    }
                }
            }
        }

        let mut field_cache = HashMap::new();
        if config
            .matchers
            .iter()
            .try_any(|matcher| matcher.denies(&mut field_cache, &mut journal))
            .wrap_err("Failed to determine whether message should be matched")?
        {
            println!("Matches!");
            //let fields = journal.collect_entry();
            let id = journal
                .get_data("SYSLOG_IDENTIFIER")?
                .and_then(|field| Some(String::from_utf8_lossy(field.value()?).into_owned()));
            let Some(msg) = journal
                .get_data("MESSAGE")?
                .and_then(|field| Some(String::from_utf8_lossy(field.value()?).into_owned()))
            else {
                continue;
            };

            notify_rust::Notification::new()
                .appname(id.as_deref().unwrap_or("journald-desktop-notifier"))
                .summary(&msg.lines().take(3).join("\n")) // coredumps have lots of lines
                .urgency(notify_rust::Urgency::Critical)
                //.icon(icon)
                // devices/computer, devices/drive-harddisk, application-x-firmware,
                // dialog-warning-symbolic
                .show()?;
        }
    }
}
