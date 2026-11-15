//! Route a problem file and print what was laid.
//!
//!     cargo run --release --example route_file -- examples/two_nets.json

use pcb_routing::{Event, Problem, Router, Status};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "examples/two_nets.json".into());
    let problem: Problem = serde_json::from_str(&std::fs::read_to_string(&path)?)?;

    let solution = Router::new(&problem)
        .on_event(|event| {
            if let Event::Round { attempt, round, in_conflict, .. } = event {
                eprintln!("attempt {attempt}, round {round}: {in_conflict} nets still too close");
            }
        })
        .run()?;

    println!("{} (attempt {} of {})", solution.status, solution.variant, solution.attempts.len());
    for net in &solution.nets {
        println!("{:<8} {:6.2} mm, {} vias", net.key, net.length_mm, net.vias.len());
        for track in &net.tracks {
            println!("    {} {:?}", track.layer, track.points);
        }
    }
    if solution.status != Status::Routed {
        println!("{} {:?}", solution.message, solution.conflicts);
    }
    Ok(())
}
