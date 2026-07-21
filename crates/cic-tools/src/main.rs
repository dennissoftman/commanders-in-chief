use std::env;
use std::error::Error;
use std::fs;
use std::process::ExitCode;

use cic_tools::render_manifest;
use cic_vfs::{BigLimits, Vfs};

const USAGE: &str = "Usage: cic-inspect manifest <mount> [<mount> ...]\n\
Each mount is a directory or BIG archive. Mounts are applied from left to right; later mounts override earlier mounts.";

fn main() -> ExitCode {
    match run(env::args().skip(1)) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: impl IntoIterator<Item = String>) -> Result<String, Box<dyn Error>> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next().ok_or("missing command")?;
    if command != "manifest" {
        return Err(format!("unknown command {command:?}").into());
    }

    let mounts = arguments.collect::<Vec<_>>();
    if mounts.is_empty() {
        return Err("manifest requires at least one mount".into());
    }

    let mut vfs = Vfs::new();
    for (index, mount) in mounts.iter().enumerate() {
        let metadata = fs::metadata(mount)?;
        let provider_name = format!("mount-{index}");
        if metadata.is_dir() {
            vfs.mount_directory(provider_name, mount)?;
        } else if metadata.is_file() {
            vfs.mount_big_file(provider_name, mount, BigLimits::default())?;
        } else {
            return Err(format!("mount is neither a directory nor a regular file: {mount}").into());
        }
    }
    Ok(render_manifest(&vfs))
}
