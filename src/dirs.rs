use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;

/// Get the default configuration path following ActivityWatch conventions
pub fn get_default_config_path() -> PathBuf {
    let mut path = ::dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("activitywatch");
    path.push("aw-notify");
    path.push("config.toml");
    path
}

/// Get the log directory path following ActivityWatch conventions
pub fn get_log_dir() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let mut dir = ::dirs::cache_dir().ok_or_else(|| anyhow!("Failed to get cache dir"))?;
        dir.push("activitywatch");
        dir.push("aw-notify");
        dir.push("log");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[cfg(target_os = "windows")]
    {
        let mut dir =
            ::dirs::data_local_dir().ok_or_else(|| anyhow!("Failed to get local data dir"))?;
        dir.push("activitywatch");
        dir.push("Logs");
        dir.push("aw-notify");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        // macOS and other Unix-like systems
        let mut dir = ::dirs::home_dir().ok_or_else(|| anyhow!("Failed to get home dir"))?;
        dir.push("Library");
        dir.push("Logs");
        dir.push("activitywatch");
        dir.push("aw-notify");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}
