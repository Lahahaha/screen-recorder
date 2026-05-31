use crate::{app::AppResult, paths::AppPaths};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    process::Command,
};

#[derive(Clone, Copy)]
pub(crate) enum InstanceKind {
    Main,
    History,
    Workdirs,
}

pub(crate) struct InstanceGuard {
    path: PathBuf,
    _file: File,
}

impl InstanceKind {
    fn lock_file_name(self) -> &'static str {
        match self {
            Self::Main => "screen-recorder-main.lock",
            Self::History => "screen-recorder-history.lock",
            Self::Workdirs => "screen-recorder-workdirs.lock",
        }
    }
}

impl InstanceGuard {
    pub(crate) fn acquire(paths: &AppPaths, kind: InstanceKind) -> AppResult<Option<Self>> {
        fs::create_dir_all(&paths.control)?;
        let path = paths.control.join(kind.lock_file_name());
        let mut file = match create_lock_file(&path)? {
            Some(file) => file,
            None => {
                if !remove_stale_lock_file(&path) {
                    return Ok(None);
                }
                match create_lock_file(&path)? {
                    Some(file) => file,
                    None => return Ok(None),
                }
            }
        };

        writeln!(file, "pid={}", std::process::id())?;
        Ok(Some(Self { path, _file: file }))
    }
}

fn create_lock_file(path: &PathBuf) -> AppResult<Option<File>> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn remove_stale_lock_file(path: &PathBuf) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    let Some(pid) = parse_lock_pid(&contents) else {
        return false;
    };
    if process_is_running(pid) {
        return false;
    }
    fs::remove_file(path).is_ok()
}

fn parse_lock_pid(contents: &str) -> Option<u32> {
    contents
        .lines()
        .find_map(|line| line.strip_prefix("pid="))
        .and_then(|pid| pid.trim().parse().ok())
}

#[cfg(target_os = "macos")]
fn process_is_running(pid: u32) -> bool {
    use std::process::Stdio;

    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn process_is_running(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;

    let mut cmd = Command::new("tasklist");
    cmd.args(["/FI", &format!("PID eq {pid}"), "/NH"]);
    cmd.creation_flags(0x08000000);
    cmd.output()
        .map(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .split_whitespace()
                    .any(|part| part == pid.to_string())
        })
        .unwrap_or(false)
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn acquire_blocks_same_instance_until_guard_drops() {
        let paths = test_paths();
        fs::create_dir_all(&paths.root).expect("create root");

        let first = InstanceGuard::acquire(&paths, InstanceKind::Main)
            .expect("acquire first")
            .expect("first guard");
        assert!(InstanceGuard::acquire(&paths, InstanceKind::Main)
            .expect("acquire second")
            .is_none());

        drop(first);

        assert!(InstanceGuard::acquire(&paths, InstanceKind::Main)
            .expect("acquire after drop")
            .is_some());
        let _ = fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn acquire_replaces_stale_lock_file() {
        let paths = test_paths();
        fs::create_dir_all(&paths.root).expect("create root");
        fs::write(
            paths.root.join(InstanceKind::History.lock_file_name()),
            "pid=4294967295\n",
        )
        .expect("write stale lock");

        assert!(InstanceGuard::acquire(&paths, InstanceKind::History)
            .expect("acquire")
            .is_some());
        let _ = fs::remove_dir_all(&paths.root);
    }

    fn test_paths() -> AppPaths {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "screen-recorder-single-instance-test-{}-{suffix}",
            std::process::id()
        ));
        AppPaths {
            control: root.join("control"),
            config: root.join("config.json"),
            screenshots: root.join("screenshots"),
            videos: root.join("videos"),
            root,
        }
    }
}
