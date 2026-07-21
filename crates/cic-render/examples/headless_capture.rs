use std::error::Error;
use std::fs;
use std::path::PathBuf;

use cic_render::{HeadlessRenderer, Pose};

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("synthetic-capture.ppm"), PathBuf::from);
    let renderer = pollster::block_on(HeadlessRenderer::new())?;
    let capture = renderer.capture_triangle(64, 64, Pose::translation(0.25, 0.0)?)?;
    fs::write(&output, capture.ppm())?;
    println!("adapter\t{}", renderer.adapter_info().name);
    println!("rgba_sha256\t{}", capture.sha256());
    println!("wrote\t{}", output.display());
    Ok(())
}
