#![deny(unused_crate_dependencies, clippy::pedantic)]

mod config;

use config::Config;
use directories::ProjectDirs;
use itertools::Itertools;
use std::{
    collections::HashMap,
    ffi::{CStr, OsString},
    io::Write,
    path::PathBuf,
    process::ExitCode,
};

use color_eyre::eyre::{Context, OptionExt, Result};
use systemd::{JournalWaitResult, id128::Id128, journal::JournalRef};

use crate::config::{NotifyResult, NotifyResultAllowed};

/// Maximum number of message lines included in a notification summary.
const MAX_SUMMARY_LINES: usize = 3;

/// Parsed command-line invocation.
#[derive(Debug)]
struct Cli {
    debug: bool,
    config_path: PathBuf,
}

/// Parses command-line arguments, accepting flags in any position.
fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Cli, String> {
    let mut debug = false;
    let mut config_path = None;

    for arg in args {
        if arg == "--debug" || arg == "-d" {
            debug = true;
            continue;
        }

        if arg.to_string_lossy().starts_with('-') {
            return Err(format!("Unknown option: {}", arg.to_string_lossy()));
        }

        if config_path.is_some() {
            return Err(format!("Unexpected argument: {}", arg.to_string_lossy()));
        }

        config_path = Some(PathBuf::from(arg));
    }

    let Some(config_path) = config_path else {
        return Err("Missing CONFIG argument".to_owned());
    };

    Ok(Cli { debug, config_path })
}

/// Prints the command-line usage text to stderr.
fn print_usage() {
    eprintln!(concat!(
        "Usage: ",
        env!("CARGO_PKG_NAME"),
        " [--debug] CONFIG\n\n",
        "Options:\n",
        "  -d, --debug  Print entries of the current boot that match the configuration,\n",
        "               then exit\n\n",
        "Report issues at ",
        env!("CARGO_PKG_REPOSITORY"),
        "/issues\n",
        "View possible fields using `journalctl -o verbose`\n"
    ));
}

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

/// Scans every entry of the current boot and prints matching notifications.
fn debug_journal(config: &Config, journal: &mut JournalRef) -> Result<()> {
    let message_box = c"MESSAGE".into();
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();

    journal
        .seek_head()
        .wrap_err("Failed to seek to the start of the journal")?;

    while next_entry(journal)?.is_some() {
        let mut field_cache = HashMap::new();

        let NotifyResult::NotifyAllowed(allowed) = match_entry(config, &mut field_cache, journal)?
        else {
            continue;
        };

        let Some(msg) = config::get_journal_field(&message_box, &mut field_cache, journal)? else {
            continue;
        };

        let NotificationContent { appname, summary } =
            notification_content(config, &allowed, msg, journal)?;

        writeln!(stdout, "{appname}: {summary}")?;
    }

    Ok(())
}

fn main() -> Result<ExitCode> {
    color_eyre::install()?;

    let cli = match parse_args(std::env::args_os().skip(1)) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{message}\n");
            print_usage();
            return Ok(ExitCode::FAILURE);
        }
    };

    let config = Config::read(&cli.config_path)?;

    let mut journal = systemd::journal::OpenOptions::default().open()?;
    journal.match_add("_BOOT_ID", Id128::from_boot()?.to_string())?;

    if cli.debug {
        debug_journal(&config, &mut journal)?;
        return Ok(ExitCode::SUCCESS);
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses a config path alone and checks that debug mode stays off.
    #[test]
    fn parse_args_plain_config() {
        let cli = parse_args(["config.toml"].map(OsString::from)).unwrap();
        assert!(!cli.debug);
        assert_eq!(cli.config_path, PathBuf::from("config.toml"));
    }

    /// Parses both debug flag spellings before and after the config path and
    /// checks that each selects debug mode.
    #[test]
    fn parse_args_debug_variants() {
        for args in [
            ["--debug", "config.toml"],
            ["-d", "config.toml"],
            ["config.toml", "--debug"],
        ] {
            let cli = parse_args(args.map(OsString::from)).unwrap();
            assert!(cli.debug, "{args:?}");
            assert_eq!(cli.config_path, PathBuf::from("config.toml"), "{args:?}");
        }
    }

    /// Passes no arguments and checks that parsing reports the missing config path.
    #[test]
    fn parse_args_missing_config() {
        let error = parse_args(std::iter::empty::<OsString>()).unwrap_err();
        assert_eq!(error, "Missing CONFIG argument");
    }

    /// Passes two positional paths and checks that the second is rejected.
    #[test]
    fn parse_args_extra_positional() {
        let error = parse_args(["one.toml", "two.toml"].map(OsString::from)).unwrap_err();
        assert_eq!(error, "Unexpected argument: two.toml");
    }

    /// Passes an unknown flag and checks that it is rejected by name.
    #[test]
    fn parse_args_unknown_option() {
        let error = parse_args(["--bogus"].map(OsString::from)).unwrap_err();
        assert_eq!(error, "Unknown option: --bogus");
    }
}
