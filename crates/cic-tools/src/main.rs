use std::env;
use std::error::Error;
use std::process::ExitCode;

use cic_tools::render_manifest;
use cic_vfs::Vfs;

const USAGE: &str = "Usage: cic-inspect manifest <directory> [<directory> ...]\n\
Directories are mounted from left to right; later mounts override earlier mounts.";

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

    let directories = arguments.collect::<Vec<_>>();
    if directories.is_empty() {
        return Err("manifest requires at least one directory".into());
    }

    let mut vfs = Vfs::new();
    for (index, directory) in directories.iter().enumerate() {
        vfs.mount_directory(format!("mount-{index}"), directory)?;
    }
    Ok(render_manifest(&vfs))
}
