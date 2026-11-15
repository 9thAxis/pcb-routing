//! pcb-routing route PROBLEM.json -o SOLUTION.json
//! pcb-routing masks PROBLEM.json --net KEY -o MASK.bin   (one byte per cell, row by row: bit i = layer i closed, 0x80 = closed to vias)

use pcb_routing::model::Problem;
use pcb_routing::raster::{net_mask, Grid};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    if args.len() < 3 || !["route", "masks"].contains(&args[1].as_str()) {
        eprintln!("usage: pcb-routing route PROBLEM.json -o SOLUTION.json [--threads N]\n       pcb-routing masks PROBLEM.json --net KEY -o MASK.bin");
        std::process::exit(2);
    }
    let start = Instant::now();
    let text = std::fs::read_to_string(&args[2]).unwrap_or_else(|e| panic!("cannot read {}: {e}", args[2]));
    let problem: Problem = serde_json::from_str(&text).unwrap_or_else(|e| panic!("bad problem file: {e}"));
    let threads = value("--threads").map(|v| v.parse().expect("--threads N")).unwrap_or(problem.options.threads);
    if threads > 0 {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().unwrap();
    }
    let out = value("-o").expect("-o FILE");
    if args[1] == "masks" {
        let key = value("--net").expect("--net KEY");
        let net = problem.nets.iter().find(|n| n.key == key).expect("no net with that key");
        let grid = Grid::new(problem.window, problem.step);
        std::fs::write(&out, net_mask(&problem, &grid, net)).unwrap();
        println!("{} x {} cells, {:.2} s", grid.w, grid.h, start.elapsed().as_secs_f64());
        return;
    }
    let read = start.elapsed().as_secs_f64();
    let solution = pcb_routing::router::solve(&problem);
    std::fs::write(&out, serde_json::to_string(&solution).unwrap()).unwrap();
    println!("{} after {} rounds (attempt {}), score {:.1}, read {:.2} s, total {:.2} s", solution.status, solution.iterations, solution.variant, solution.score, read, start.elapsed().as_secs_f64());
    if std::env::var("PCB_ROUTING_TRACE").is_ok() {
        for (name, clock) in pcb_routing::router::CLOCK_NAMES.iter().zip(&pcb_routing::router::CLOCK) {
            eprintln!("time, all attempts together: {name} {:.2} s", clock.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e9);
        }
    }
    for n in &solution.nets {
        let mut layers: Vec<&str> = n.tracks.iter().map(|t| t.layer.as_str()).collect();
        layers.sort();
        layers.dedup();
        println!("  {:<22} {:<18} {:7.2} mm  vias {:2}  {}", n.key, n.net, n.length_mm, n.vias.len(), layers.join(" "));
    }
    for c in solution.conflicts.iter().take(30) {
        println!("  CONFLICT {} {} with {:?} at {:?} {}", c.kind, c.key, c.with, c.xy, c.layer.clone().unwrap_or_default());
    }
    if !solution.message.is_empty() {
        println!("  {}", solution.message);
    }
    if solution.status != "routed" {
        std::process::exit(1);
    }
}
