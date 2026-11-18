#![deny(unused_crate_dependencies, clippy::pedantic)]

mod config;

use config::Config;
use directories::ProjectDirs;
use itertools::Itertools;
use std::{collections::HashMap, ffi::CStr, process::ExitCode};

use color_eyre::eyre::{Context, OptionExt, Result};
use systemd::{JournalWaitResult, id128::Id128, journal::JournalRef};

use crate::config::{NotifyResult, NotifyResultAllowed};

/// Maximum number of message lines included in a notification summary.
const MAX_SUMMARY_LINES: usize = 3;

/// Advances the journal to the next entry, returning `None` at the end.
fn next_entry(journal: &mut JournalRef) -> Result<Option<()>> {
    Ok((journal.next()? != 0).then_some(()))
}

/// Returns the first matcher in the config that allows notifying the current entry.
#[expect(
    clippy::borrowed_box,
    reason = "Impossible due to the current borrow checker"
)]
fn match_entry<'config: 'field_name, 'field_name>(
    config: &'config Config,
    field_cache: &mut HashMap<&'field_name Box<CStr>, Vec<u8>>,
    journal: &mut JournalRef,
) -> Result<NotifyResult> {
    let mut res = NotifyResult::Ignored;
    for matcher in &config.matchers {
        res = matcher
            .allows_notify(field_cache, journal)
            .wrap_err("Failed to determine whether message should be matched")?;
        if matches!(res, NotifyResult::NotifyAllowed(_)) {
            break;
        }
    }
    Ok(res)
}

/// The app name and summary of a notification.
struct NotificationContent {
    appname: String,
    summary: String,
}

/// Builds the notification content for an allowed match from its message.
fn notification_content(
    config: &Config,
    allowed: &NotifyResultAllowed,
    msg: &[u8],
    journal: &mut JournalRef,
) -> Result<NotificationContent> {
    let mut appname = None;
    for field in &config.id_fields {
        let Some(data) = journal.get_data(field)? else {
            continue;
        };
        let Some(value) = data.value() else {
            continue;
        };
        appname = Some(String::from_utf8_lossy(value).into_owned());
        break;
    }

    let msg = allowed
        .content_regex
        .as_ref()
        .and_then(|regex| Some(regex.captures(msg)?.get(1)?.as_bytes()))
        .unwrap_or(msg);

    let msg = String::from_utf8_lossy(msg);
    let prefix = allowed.prefix.as_deref().unwrap_or_default();
    let summary = format!("{prefix}{msg}")
        .lines()
        .take(MAX_SUMMARY_LINES) // coredumps have lots of lines
        .join("\n");

    Ok(NotificationContent {
        appname: appname.unwrap_or_else(|| env!("CARGO_PKG_NAME").to_owned()),
        summary,
    })
}

fn main() -> Result<ExitCode> {
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

        let NotifyResult::NotifyAllowed(allowed) =
            match_entry(&config, &mut field_cache, &mut journal)?
        else {
            continue;
        };

        let Some(msg) = config::get_journal_field(&message_box, &mut field_cache, &mut journal)?
        else {
            continue;
        };

        let NotificationContent { appname, summary } =
            notification_content(&config, &allowed, msg, &mut journal)?;

        notify_rust::Notification::new()
            .appname(&appname)
            .summary(&summary)
            .urgency(notify_rust::Urgency::Critical)
            //.icon(icon)
            // devices/computer, devices/drive-harddisk, application-x-firmware,
            // dialog-warning-symbolic
            .show()?;
    }
}
