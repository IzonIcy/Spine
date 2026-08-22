//! Generate launchd plists so `spn upgrade` can run on a schedule.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const LABEL: &str = "com.spine.upgrade";

/// Build a macOS launchd plist that runs `spn upgrade` on a schedule.
///
/// Weekly schedules fire Mondays at the requested time; daily schedules
/// fire every day. The plist is pure templating — installing it is left to
/// the user (`launchctl bootstrap` or copying into ~/Library/LaunchAgents).
pub fn launchd_plist(program: &Path, daily: bool, hour: u32, minute: u32) -> String {
    let program = program.display();
    let calendar = if daily {
        format!(
            "    <key>StartCalendarInterval</key>\n    <dict>\n      <key>Hour</key>\n      <integer>{hour}</integer>\n      <key>Minute</key>\n      <integer>{minute}</integer>\n    </dict>"
        )
    } else {
        format!(
            "    <key>StartCalendarInterval</key>\n    <dict>\n      <key>Weekday</key>\n      <integer>1</integer>\n      <key>Hour</key>\n      <integer>{hour}</integer>\n      <key>Minute</key>\n      <integer>{minute}</integer>\n    </dict>"
        )
    };

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n    <key>Label</key>\n    <string>{LABEL}</string>\n    <key>ProgramArguments</key>\n    <array>\n      <string>{program}</string>\n      <string>upgrade</string>\n    </array>\n{calendar}\n    <key>RunAtLoad</key>\n    <false/>\n  </dict>\n</plist>\n"
    )
}

fn parse_at(at: &str) -> Result<(u32, u32)> {
    let (hour, minute) = at
        .split_once(':')
        .context("invalid --at time, expected HH:MM")?;
    let hour: u32 = hour.parse().context("invalid hour in --at")?;
    let minute: u32 = minute.parse().context("invalid minute in --at")?;
    if hour > 23 || minute > 59 {
        anyhow::bail!("--at time out of range: {at}");
    }
    Ok((hour, minute))
}

/// Emit the plist to stdout or to `out`.
pub fn emit(daily: bool, at: &str, out: Option<&Path>) -> Result<PathBuf> {
    let (hour, minute) = parse_at(at)?;
    let program =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/spn"));
    let plist = launchd_plist(&program, daily, hour, minute);

    match out {
        Some(path) => {
            std::fs::write(path, plist)
                .with_context(|| format!("writing schedule to {}", path.display()))?;
            Ok(path.to_path_buf())
        }
        None => {
            print!("{plist}");
            Ok(PathBuf::from("<stdout>"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekly_plist_targets_monday() {
        let plist = launchd_plist(Path::new("/usr/local/bin/spn"), false, 9, 30);
        assert!(plist.contains("<key>Label</key>\n    <string>com.spine.upgrade</string>"));
        assert!(plist.contains("<key>Weekday</key>\n      <integer>1</integer>"));
        assert!(plist.contains("<integer>9</integer>"));
        assert!(plist.contains("<integer>30</integer>"));
        assert!(plist.starts_with("<?xml"));
        assert!(plist.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn daily_plist_has_no_weekday() {
        let plist = launchd_plist(Path::new("/spn"), true, 22, 5);
        assert!(!plist.contains("Weekday"));
        assert!(plist.contains("<string>/spn</string>"));
        assert!(plist.contains("<string>upgrade</string>"));
    }

    #[test]
    fn parses_and_rejects_times() {
        assert_eq!(parse_at("09:30").unwrap(), (9, 30));
        assert_eq!(parse_at("23:59").unwrap(), (23, 59));
        assert!(parse_at("24:00").is_err());
        assert!(parse_at("12:60").is_err());
        assert!(parse_at("noon").is_err());
    }
}
