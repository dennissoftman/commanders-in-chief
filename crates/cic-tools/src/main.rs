use std::env;
use std::error::Error;
use std::fs;
use std::process::ExitCode;

use cic_formats::{CsfLimits, parse_csf};
use cic_tools::{render_csf, render_manifest};
use cic_vfs::{BigLimits, Vfs, VirtualPath};

const USAGE: &str = "Usage:\n\
  cic-inspect manifest <mount> [<mount> ...]\n\
  cic-inspect csf <virtual-path> <mount> [<mount> ...]\n\
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
    match command.as_str() {
        "manifest" => {
            let mounts = arguments.collect::<Vec<_>>();
            let vfs = mount_all("manifest", &mounts)?;
            Ok(render_manifest(&vfs))
        }
        "csf" => {
            let resource_name = arguments.next().ok_or("csf requires a virtual path")?;
            let mounts = arguments.collect::<Vec<_>>();
            let vfs = mount_all("csf", &mounts)?;
            let resource_path = VirtualPath::new(&resource_name)?;
            let entry = vfs
                .resolve(&resource_path)
                .ok_or_else(|| format!("resource not found: {resource_path}"))?;
            let csf = parse_csf(entry.bytes(), resource_path.as_str(), CsfLimits::default())?;
            Ok(render_csf(&csf))
        }
        _ => Err(format!("unknown command {command:?}").into()),
    }
}

fn mount_all(command: &str, mounts: &[String]) -> Result<Vfs, Box<dyn Error>> {
    if mounts.is_empty() {
        return Err(format!("{command} requires at least one mount").into());
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
    Ok(vfs)
}
