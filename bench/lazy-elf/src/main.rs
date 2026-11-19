use anyhow::{Result, bail, ensure};
use std::{
    env,
    fs::{File, OpenOptions},
    io::{BufReader, Read},
    path::PathBuf,
};
#[allow(dead_code)]
#[path = "../../../src/remote_artifact_prediction.rs"]
mod production;
mod rows;
#[allow(dead_code)]
mod prediction {
    include!(concat!(env!("OUT_DIR"), "/prediction.rs"));
}
fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    ensure!(
        args.len() == 4,
        "usage: lazy-elf-bench control|candidate OLD NEW FRAME, or expand:SIZE OLD FRAME OUT (see README)"
    );
    let old = PathBuf::from(&args[1]);
    let input = PathBuf::from(&args[2]);
    let output = PathBuf::from(&args[3]);
    let size = input.metadata()?.len();
    match args[0].to_str() {
        Some(mode @ ("control" | "candidate")) => {
            ensure!(size <= prediction::MAX, "oversized input");
            drop(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&output)?,
            );
            if mode == "control" {
                production::compress(&old, &input, size, &output)
            } else {
                prediction::compress(&old, &input, size, &output, true)
            }
        }
        Some(mode) if mode.starts_with("expand:") => {
            let n = mode[7..].parse()?;
            production::expand(
                &old,
                n,
                BufReader::new(File::open(&input)?).take(size),
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(output)?,
            )
        }
        _ => bail!("unknown mode"),
    }
}
