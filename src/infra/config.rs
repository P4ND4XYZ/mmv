use std::{fs, path::PathBuf};

use crate::lua::LuaRuntime;
#[cfg(test)]
use std::path::Path;

pub struct ConfigLoad {
    pub runtime: LuaRuntime,
    pub error: Option<ConfigError>,
}

pub struct ConfigError {
    pub path: Option<PathBuf>,
    pub cause: String,
}

pub fn load_config() -> ConfigLoad {
    load_path(super::paths::UserPaths::current().map(|paths| paths.config))
}

#[cfg(test)]
fn load_from(local_app_data: Option<&Path>) -> ConfigLoad {
    load_path(
        local_app_data
            .filter(|base| base.is_absolute())
            .map(|base| base.join("mmv/init.lua"))
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "user base unavailable")
            }),
    )
}

fn load_path(path: std::io::Result<PathBuf>) -> ConfigLoad {
    let result = (|| {
        let path = path.map_err(|error| ConfigError {
            path: None,
            cause: error.to_string(),
        })?;
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => {
                return Err(ConfigError {
                    path: Some(path),
                    cause: error.to_string(),
                });
            }
        };
        Ok(Some((source, path)))
    })();
    // The user candidate is isolated and atomic; the baseline has no bindings.
    let (runtime, error) = match result {
        Ok(source) => {
            let input = source
                .as_ref()
                .map(|(source, path)| (source.as_str(), path.to_string_lossy()));
            let (runtime, cause) = LuaRuntime::initialize(
                "",
                input
                    .as_ref()
                    .map(|(source, name)| (*source, name.as_ref())),
            )
            .expect("create Lua runtime");
            let error = cause.map(|cause| ConfigError {
                path: source.map(|(_, path)| path),
                cause,
            });
            (runtime, error)
        }
        Err(error) => (
            LuaRuntime::initialize("", None)
                .expect("create Lua runtime")
                .0,
            Some(error),
        ),
    };
    ConfigLoad { runtime, error }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn missing_file_is_normal_and_missing_environment_is_error() {
        let path = std::env::temp_dir().join(format!("mmv-config-missing-{}", std::process::id()));
        let missing = load_from(Some(&path));
        assert_eq!(missing.runtime.config(), Config::default());
        assert!(missing.error.is_none());
        assert!(!missing.runtime.enqueue_key("f").unwrap());
        assert_eq!(missing.runtime.pump_one(), crate::lua::Pump::Idle);
        assert!(missing.runtime.context().ui.borrow().windows.is_empty());
        let unavailable = load_from(None);
        assert_eq!(unavailable.runtime.config(), Config::default());
        assert!(unavailable.error.unwrap().path.is_none());
        let relative = load_from(Some(Path::new("")));
        assert_eq!(relative.runtime.config(), Config::default());
        assert!(relative.error.unwrap().path.is_none());
    }

    #[test]
    fn read_failure_and_execution_failure_return_defaults_and_path() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "mmv-config-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let dir = root.join("mmv");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("init.lua");
        std::fs::create_dir(&path).unwrap();
        let read_error = load_from(Some(&root));
        assert_eq!(read_error.runtime.config(), Config::default());
        assert!(!read_error.runtime.enqueue_key("f").unwrap());
        assert_eq!(
            read_error.error.unwrap().path.as_deref(),
            Some(path.as_path())
        );
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(
            &path,
            "local m=require('mmv'); m.setup({view={position='left'}}); error('broken')",
        )
        .unwrap();
        let execute_error = load_from(Some(&root));
        assert_eq!(execute_error.runtime.config(), Config::default());
        assert!(!execute_error.runtime.enqueue_key("f").unwrap());
        let error = execute_error.error.unwrap();
        assert_eq!(error.path.as_deref(), Some(path.as_path()));
        assert!(error.cause.contains("broken"));
        std::fs::write(&path, "local m=require('mmv'); m.setup({view={position='left'}}); pcall(function() m.set_hl('Normal',{fg='bad'}) end)").unwrap();
        let invalid = load_from(Some(&root));
        assert_eq!(invalid.runtime.config(), Config::default());
        assert!(!invalid.runtime.enqueue_key("f").unwrap());
        assert!(invalid.error.unwrap().cause.contains("color"));
        std::fs::write(&path, "local m=require('mmv'); m.setup({view={position='left'}}); local n=5; m.keymap.set('x',function() n=n+1; result=n end)").unwrap();
        let valid = load_from(Some(&root));
        assert!(valid.error.is_none());
        assert_eq!(
            valid.runtime.config().position,
            crate::config::Position::Left
        );
        assert!(valid.runtime.enqueue_key("x").unwrap());
        assert_eq!(valid.runtime.pump_one(), crate::lua::Pump::Dispatched);
        assert_eq!(valid.runtime.pump_one(), crate::lua::Pump::Idle);
        assert_eq!(
            valid.runtime.lua().globals().get::<i64>("result").unwrap(),
            6
        );
        std::fs::write(&path, "local m=require('mmv'); m.keymap.set('f',function()end); assert(m.keymap.remove('f')); assert(not m.keymap.remove('f'))").unwrap();
        let removed = load_from(Some(&root));
        assert!(removed.error.is_none());
        assert!(!removed.runtime.enqueue_key("f").unwrap());
        std::fs::write(&path, "local m=require('mmv');m.keymap.remove('f');pcall(m.keymap.set,'f',function()end,{focus='body'})").unwrap();
        let failed = load_from(Some(&root));
        assert!(failed.error.is_some());
        assert!(!failed.runtime.enqueue_key("f").unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }
}
