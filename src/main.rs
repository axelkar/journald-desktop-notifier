#![deny(unused_crate_dependencies, clippy::pedantic)]

mod config;

use config::Config;
use directories::ProjectDirs;
use itertools::Itertools;
use std::{collections::HashMap, process::ExitCode};

use color_eyre::eyre::{Context, OptionExt, Result};
use systemd::{JournalWaitResult, id128::Id128, journal::JournalRef};

use crate::config::{NotifyResult, NotifyResultAllowed};

#[expect(clippy::too_many_lines, reason = "TODO: shorten")] // TODO: shorten
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

    let proj_dirs = ProjectDirs::from("fi.axka", "axka", "journald-desktop-notifier")
        .ok_or_eyre("Failed to get state directory")?;
    let state_dir = proj_dirs
        .state_dir()
        .ok_or_eyre("Failed to get state directory")?;
    std::fs::create_dir_all(state_dir).wrap_err("Failed to create state directory")?;
    let cursor_path = state_dir.join("journal-cursor");

    let mut last_written_cursor = None;

    if cursor_path.exists() {
        let cursor = std::fs::read_to_string(&cursor_path)
            .wrap_err("Failed to read journal cursor from disk")?;
        journal
            .seek_cursor(&cursor)
            .wrap_err("Failed to seek to cursor in journal")?;
        last_written_cursor = Some(cursor);
    }

    let message_box = c"MESSAGE".into();

    loop {
        if next_entry(&mut journal)?.is_none() {
            // No journal entry available

            // Write the cursor to disk.
            let cursor = journal
                .cursor()
                .wrap_err("Failed to get cursor from journal")?;
            if Some(&cursor) != last_written_cursor.as_ref() {
                std::fs::write(&cursor_path, &cursor)
                    .wrap_err("Failed to write journal cursor to disk")?;
                last_written_cursor = Some(cursor);
            }

            // Let's wait
            loop {
                match journal.wait(None)? {
                    JournalWaitResult::Nop => {}
                    JournalWaitResult::Append | JournalWaitResult::Invalidate => {
                        if next_entry(&mut journal)?.is_some() {
                            break;
                        }
                    }
                }
            }
        }

        let mut field_cache = HashMap::new();

        let mut res = NotifyResult::Ignored;
        for matcher in &config.matchers {
            res = matcher
                .allows_notify(&mut field_cache, &mut journal)
                .wrap_err("Failed to determine whether message should be matched")?;
            if matches!(res, NotifyResult::NotifyAllowed(_)) {
                break;
            }
        }

        if let NotifyResult::NotifyAllowed(NotifyResultAllowed {
            prefix,
            content_regex,
        }) = res
        {
            let mut id = None;
            for field in &config.id_fields {
                id = journal
                    .get_data(field)?
                    .and_then(|field| Some(String::from_utf8_lossy(field.value()?).into_owned()));
                if id.is_some() {
                    break;
                }
            }

            let Some(msg) =
                config::get_journal_field(&message_box, &mut field_cache, &mut journal)?
            else {
                continue;
            };

            let msg = content_regex
                .and_then(|regex| Some(regex.captures(msg)?.get(1)?.as_bytes()))
                .unwrap_or(msg);

            let msg = String::from_utf8_lossy(msg);
            let prefix = prefix.unwrap_or_default();

            let msg = format!("{prefix}{msg}");

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
