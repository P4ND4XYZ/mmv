//! User-scoped locations. Never fall back to the working directory or XDG.
use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub(super) struct UserPaths {
    pub config: PathBuf,
    pub logs: PathBuf,
    pub instance: PathBuf,
}
impl UserPaths {
    pub fn current() -> io::Result<Self> {
        #[cfg(windows)]
        let base = std::env::var_os("LOCALAPPDATA");
        #[cfg(not(windows))]
        let base = std::env::var_os("HOME");
        Self::from_base(base.as_deref().map(Path::new), cfg!(windows))
    }
    fn from_base(base: Option<&Path>, windows: bool) -> io::Result<Self> {
        let base = base.filter(|p| p.is_absolute()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "user base directory is unavailable or not absolute",
            )
        })?;
        Ok(if windows {
            Self {
                config: base.join("mmv/init.lua"),
                logs: base.join("my-markdown-viewer/logs"),
                instance: base.join("mmv/instance"),
            }
        } else {
            Self {
                config: base.join(".config/mmv/init.lua"),
                logs: base.join(".local/state/mmv"),
                instance: base.join(".local/state/mmv/instance"),
            }
        })
    }
}

/// Lifetime identity is the directory, never a TCP port or a login session.
#[derive(Clone, Debug)]
pub struct Namespace {
    pub(super) directory: PathBuf,
    pub(super) port: u16,
}
impl Namespace {
    pub fn current_user(port: std::num::NonZeroU16) -> io::Result<Self> {
        Ok(Self {
            directory: UserPaths::current()?.instance,
            port: port.get(),
        })
    }
    pub(super) fn address(&self) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port)
    }
    #[cfg(test)]
    pub(super) fn isolated() -> Self {
        use std::{
            net::TcpListener,
            sync::atomic::{AtomicU64, Ordering},
        };
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let directory = std::env::temp_dir().join(format!(
            "mmv-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        Self { directory, port }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Environment is changed only in fresh helper processes, never in a test thread.
    #[test]
    fn location_environment_child() {
        let Some(base) = std::env::var_os("MMV_TEST_PATH_BASE") else {
            return;
        };
        if std::env::var_os("MMV_TEST_PATH_MISSING").is_some() {
            assert!(UserPaths::current().is_err());
            return;
        }
        let base = PathBuf::from(base);
        let expected = UserPaths::from_base(Some(&base), cfg!(windows)).unwrap();
        let actual = UserPaths::current().unwrap();
        assert_eq!(actual.config, expected.config);
        assert_eq!(actual.logs, expected.logs);
        assert_eq!(actual.instance, expected.instance);
    }

    #[test]
    fn current_paths_ignore_xdg_and_reject_missing_home_in_fresh_processes() {
        use std::{
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let base = std::env::temp_dir().join(format!("mmv-test-paths-{}", std::process::id()));
        for missing in [false, true] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "infra::paths::tests::location_environment_child"])
                .env("MMV_TEST_PATH_BASE", &base)
                .env("LOCALAPPDATA", &base)
                .env("HOME", &base)
                .env("XDG_CONFIG_HOME", base.join("not-config"))
                .env("XDG_STATE_HOME", base.join("not-state"))
                .env("XDG_RUNTIME_DIR", base.join("not-runtime"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if missing {
                command
                    .env_remove(if cfg!(windows) {
                        "LOCALAPPDATA"
                    } else {
                        "HOME"
                    })
                    .env("MMV_TEST_PATH_MISSING", "1");
            }
            let mut child = Child(command.spawn().unwrap());
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    assert!(status.success(), "path helper failed: {status}");
                    break;
                }
                assert!(Instant::now() < deadline, "path helper timeout");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    #[test]
    fn user_locations_are_fixed_and_missing_base_never_uses_cwd() {
        let base = std::env::temp_dir();
        let linux = UserPaths::from_base(Some(&base), false).unwrap();
        assert_eq!(linux.config, base.join(".config/mmv/init.lua"));
        assert_eq!(linux.logs, base.join(".local/state/mmv"));
        assert_eq!(linux.instance, base.join(".local/state/mmv/instance"));
        let windows = UserPaths::from_base(Some(&base), true).unwrap();
        assert_eq!(windows.config, base.join("mmv/init.lua"));
        assert_eq!(windows.logs, base.join("my-markdown-viewer/logs"));
        assert_eq!(windows.instance, base.join("mmv/instance"));
        for base in [None, Some(Path::new("")), Some(Path::new("relative"))] {
            assert!(UserPaths::from_base(base, false).is_err());
            assert!(UserPaths::from_base(base, true).is_err());
        }
    }
}
