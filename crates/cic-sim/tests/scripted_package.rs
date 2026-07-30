//! The whole path a scripted map actually takes: a `.cicmap`'s bytes to handlers running in a tick.
//!
//! Each half of this is covered where it lives — `cic-assets` proves a package reads the scripts its
//! scenario names, and `cic_sim::scripts` proves compiled scripts dispatch deterministically. What
//! only this test covers is the seam between them, which is the clause
//! [M10](../../../docs/milestones/m10-scripting.md)'s exit condition is written in: behaviour written
//! in a file, *loaded from a package*, producing the same results every time.
//!
//! The zip writer below is a fixture rather than a dependency. `cic-assets` has one of its own for the
//! same purpose, and sharing it would mean making a test helper part of that crate's public surface to
//! save forty lines here.

use cic_assets::package::{MapPackage, PackageLimits, SCENARIO_PATH};
use cic_assets::scenario::{PlayerSlot, Position, Scenario, TerrainReference};
use cic_assets::terrain::{Terrain, TerrainLayer};
use cic_script::{Limits, RuntimeLimits};
use cic_sim::kernel::{Kernel, KernelConfig, first_divergence};
use cic_sim::scripts::{SCRIPTS, Scripts};

/// A stored-only zip, which is all a fixture needs and all the reader requires.
fn zip(members: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut directory = Vec::new();
    let mut offsets = Vec::new();
    for (name, payload) in members {
        offsets.push(u32::try_from(body.len()).expect("fixture is small"));
        let size = u32::try_from(payload.len()).expect("fixture is small");
        body.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        body.extend_from_slice(&20u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes()); // stored
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&size.to_le_bytes());
        body.extend_from_slice(&size.to_le_bytes());
        body.extend_from_slice(&u16::try_from(name.len()).expect("short name").to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(name.as_bytes());
        body.extend_from_slice(payload);
    }
    for ((name, payload), offset) in members.iter().zip(&offsets) {
        let size = u32::try_from(payload.len()).expect("fixture is small");
        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&u16::try_from(name.len()).expect("short name").to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }

    let directory_offset = u32::try_from(body.len()).expect("fixture is small");
    let directory_size = u32::try_from(directory.len()).expect("fixture is small");
    let count = u16::try_from(members.len()).expect("few members");
    let mut archive = body;
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&directory_size.to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive
}

/// Eleven samples ten units apart spans a hundred world units square.
fn terrain() -> Terrain {
    Terrain::new(
        11,
        11,
        10.0,
        0.25,
        vec![200; 121],
        vec![TerrainLayer {
            name: "grass".to_owned(),
            weights: vec![255; 121],
        }],
    )
    .expect("a valid fixture terrain")
}

/// A map whose scenario names two scripts, both carried in the archive.
fn scripted_map() -> Vec<u8> {
    let scenario = Scenario {
        format_version: 1,
        name: "Scripted".to_owned(),
        description: String::new(),
        terrain: TerrainReference {
            path: "terrain/alpine.cict".to_owned(),
        },
        players: vec![PlayerSlot {
            id: "north".to_owned(),
            name: "North".to_owned(),
            faction: "faction/vanguard".to_owned(),
            start: Position {
                x: 10.0,
                y: 90.0,
                z: 0.0,
            },
            team: 1,
        }],
        objects: Vec::new(),
        waypoints: Vec::new(),
        scripts: vec![
            "scripts/mission.cics".to_owned(),
            "scripts/observer.cics".to_owned(),
        ],
    };

    let mission = br#"
        on start() {
            sys.set_flag("briefed", true);
            sys.arm_timer("reinforce", 1.0);
        }
        on tick(elapsed) { sys.add_counter("ticks", 1); }
        on timer_elapsed(timer) {
            if timer == "reinforce" { sys.add_counter("waves", 1); }
        }
    "#;
    // Reads what the first script wrote, which only holds if authored order is dispatch order.
    let observer = br#"on tick(e) { if sys.flag("briefed") { sys.add_counter("seen", 1); } }"#;

    zip(&[
        (SCENARIO_PATH, scenario.to_json().expect("serialize")),
        ("terrain/alpine.cict", terrain().encode()),
        ("scripts/mission.cics", mission.to_vec()),
        ("scripts/observer.cics", observer.to_vec()),
    ])
}

/// Opens the package, compiles what its scenario names, and registers the dispatcher.
fn kernel_from(bytes: &[u8]) -> Kernel {
    let package = MapPackage::open(bytes, PackageLimits::default()).expect("the package opens");
    let sources = package
        .scripts(PackageLimits::default())
        .expect("its scripts read");
    let borrowed: Vec<(&str, &str)> = sources
        .iter()
        .map(|(path, source)| (path.as_str(), source.as_str()))
        .collect();
    let scripts = Scripts::compile(&borrowed, Limits::DEFAULT, RuntimeLimits::DEFAULT)
        .expect("its scripts compile against the kernel's interface");

    let mut kernel = Kernel::new(KernelConfig {
        seed: 11,
        ticks_per_second: 30,
    });
    kernel.add_subsystem(Box::new(scripts));
    kernel
}

fn scripts(kernel: &Kernel) -> &Scripts {
    kernel
        .subsystem(SCRIPTS)
        .and_then(|subsystem| subsystem.as_any().downcast_ref::<Scripts>())
        .expect("the dispatcher is registered")
}

#[test]
fn a_packages_scripts_load_and_run() {
    let bytes = scripted_map();
    let mut kernel = kernel_from(&bytes);

    // Ninety ticks is three seconds at thirty a second, so the one-second timer falls due on tick
    // thirty and is then consumed -- nothing re-arms it.
    for _ in 0..90 {
        kernel.advance(&[]).expect("advances");
    }

    let subsystem = scripts(&kernel);
    assert_eq!(
        subsystem.paths(),
        ["scripts/mission.cics", "scripts/observer.cics"],
        "the dispatcher holds the scenario's authored order"
    );
    assert_eq!(subsystem.faulted(), 0, "no handler faulted");

    let mission = subsystem.mission();
    assert!(mission.flag("briefed"), "`start` ran");
    assert_eq!(mission.counter("ticks"), 90, "`tick` ran once per tick");
    assert_eq!(
        mission.counter("waves"),
        1,
        "the timer fell due once and was not re-armed"
    );
    assert_eq!(
        mission.counter("seen"),
        90,
        "the second script saw the first script's flag on every tick, including the first"
    );
    assert!(
        subsystem.peak_fuel() > 0 && subsystem.peak_fuel() < RuntimeLimits::DEFAULT.fuel,
        "fuel is reported and these handlers are nowhere near the limit: {}",
        subsystem.peak_fuel()
    );
}

#[test]
fn the_same_package_replays_to_identical_hashes() {
    // The exit condition's other half: identical inputs, identical per-tick hashes. Scripted
    // behaviour is inside the determinism claim rather than beside it.
    let bytes = scripted_map();
    let run = || {
        let mut kernel = kernel_from(&bytes);
        (0..90)
            .map(|_| kernel.advance(&[]).expect("advances"))
            .collect::<Vec<_>>()
    };
    let ours = run();
    let theirs = run();
    assert_eq!(first_divergence(&ours, &theirs), None);
    assert_eq!(ours, theirs);
}

#[test]
fn a_script_naming_an_undeclared_verb_fails_the_load_not_the_trigger() {
    // The whole point of resolving the host surface at compile time: a mod reaching for something the
    // engine does not offer is caught when the map loads, naming the file, rather than when a player
    // happens to trigger the handler.
    let scenario = Scenario {
        format_version: 1,
        name: "Greedy".to_owned(),
        description: String::new(),
        terrain: TerrainReference {
            path: "terrain/alpine.cict".to_owned(),
        },
        players: vec![PlayerSlot {
            id: "north".to_owned(),
            name: "North".to_owned(),
            faction: "faction/vanguard".to_owned(),
            start: Position {
                x: 10.0,
                y: 90.0,
                z: 0.0,
            },
            team: 1,
        }],
        objects: Vec::new(),
        waypoints: Vec::new(),
        scripts: vec!["scripts/greedy.cics".to_owned()],
    };
    let bytes = zip(&[
        (SCENARIO_PATH, scenario.to_json().expect("serialize")),
        ("terrain/alpine.cict", terrain().encode()),
        (
            "scripts/greedy.cics",
            b"on start() { sys.grant_resources(99999); }".to_vec(),
        ),
    ]);

    let package = MapPackage::open(&bytes, PackageLimits::default()).expect("the package opens");
    let sources = package
        .scripts(PackageLimits::default())
        .expect("its scripts read");
    let borrowed: Vec<(&str, &str)> = sources
        .iter()
        .map(|(path, source)| (path.as_str(), source.as_str()))
        .collect();
    let error = Scripts::compile(&borrowed, Limits::DEFAULT, RuntimeLimits::DEFAULT)
        .expect_err("an undeclared verb must fail the load");

    assert_eq!(error.path, "scripts/greedy.cics");
    assert!(
        error.to_string().contains("grant_resources"),
        "the diagnostic names what was reached for: {error}"
    );
}
