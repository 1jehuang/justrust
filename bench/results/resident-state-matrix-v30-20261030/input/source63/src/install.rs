//! `justrust install`: put a `cargo` proxy in front of the real cargo so every
//! build, check, and test is recorded without changing how it is invoked.

use crate::paths;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

fn default_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("no home directory")?
        .join(".local/bin"))
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

pub fn install(dir: Option<PathBuf>) -> Result<()> {
    let dir = match dir {
        Some(d) => d,
        None => default_dir()?,
    };
    std::fs::create_dir_all(&dir)?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let link = dir.join("cargo");
    if link.exists() && std::fs::read_link(&link).ok().as_deref() != Some(exe.as_path()) {
        let is_ours = std::fs::read_link(&link)
            .ok()
            .and_then(|t| {
                t.file_name()
                    .map(|n| n.to_string_lossy().starts_with("justrust"))
            })
            .unwrap_or(false);
        if !is_ours {
            bail!(
                "{} already exists and is not a justrust proxy. Remove it or pass --dir.",
                link.display()
            );
        }
    }
    paths::replace_symlink(&exe, &link)?;
    paths::ensure_rustc_shim()?;

    let real = paths::real_cargo()?;
    println!("Installed {} -> {}", link.display(), exe.display());
    println!("Real cargo: {}", real.display());
    let dirs = path_dirs();
    let ours = dirs.iter().position(|d| d == &dir);
    let theirs = real.parent().and_then(|p| dirs.iter().position(|d| d == p));
    match (ours, theirs) {
        (None, _) => println!(
            "Warning: {} is not on PATH, so the proxy will not be used.",
            dir.display()
        ),
        (Some(a), Some(b)) if a > b => {
            println!(
                "Warning: {} comes after the real cargo on PATH, so the proxy will not be used.",
                dir.display()
            )
        }
        _ => println!(
            "Every cargo build/check/test/clippy/run is now recorded. Disable with JUSTRUST_DISABLE=1."
        ),
    }
    Ok(())
}

pub fn uninstall(dir: Option<PathBuf>) -> Result<()> {
    let dir = match dir {
        Some(d) => d,
        None => default_dir()?,
    };
    let link = dir.join("cargo");
    let exe = std::env::current_exe()?.canonicalize()?;
    match std::fs::read_link(&link) {
        Ok(t)
            if t == exe
                || t.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("justrust")) =>
        {
            std::fs::remove_file(&link)?;
            println!("Removed {}", link.display());
        }
        Ok(_) => bail!(
            "{} is not a justrust proxy, leaving it alone",
            link.display()
        ),
        Err(_) => println!("No proxy at {}", link.display()),
    }
    Ok(())
}
