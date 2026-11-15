//! Command line: route a problem file, write the fixed mask of one net, or answer a question about zones.

use pcb_routing::{mask::net_mask, Event, Grid, Problem, Router, Status};
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str =
    "usage: pcb-routing route PROBLEM.json -o SOLUTION.json [--threads N] [--trace] [--verify] [--only ATTEMPT]
       pcb-routing masks PROBLEM.json --net KEY -o MASK.bin
       pcb-routing pours [QUESTION.json] [-o ANSWER.json] [--threads N]

route   lay the nets of the problem; exit status 0 if every net is laid and none are too close, 1 otherwise
masks   one byte per cell, row by row: bit i set = routing layer i closed to the net's tracks, 0x80 = closed to its vias
pours   answer a question about a board's filled zones (what they join, where a net may be joined to one, how
        each pad is tied to its zone); the question is read from standard input and the answer written to
        standard output where no file is named

--trace   one line per round of negotiation and the time spent per kind of work (also: PCB_ROUTING_TRACE=1)
--verify  run every search a second time in one pass and report any whose cost differs (also: PCB_ROUTING_CHECK=1)
--only N  run attempt N of the problem's variants alone (also: PCB_ROUTING_ONLY=N)";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("pcb-routing: {message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let flag = |flag: &str, env: &str| args.iter().any(|a| a == flag) || std::env::var(env).is_ok();
    if args.get(1).map(String::as_str) == Some("pours") {
        // (the question is the argument that is neither a flag nor the value of one)
        let flagged = |k: usize| args[k].starts_with('-') || ["-o", "--threads"].contains(&args[k - 1].as_str());
        let threads =
            value("--threads").map(|n| n.parse().map_err(|_| "--threads takes a number".to_string())).transpose()?;
        return pours((2..args.len()).find(|&k| !flagged(k)).map(|k| &args[k]), value("-o"), threads.unwrap_or(0));
    }
    if args.len() < 3 || !["route", "masks"].contains(&args[1].as_str()) {
        return Err(USAGE.into());
    }
    let start = Instant::now();
    let text = std::fs::read_to_string(&args[2]).map_err(|e| format!("cannot read {}: {e}", args[2]))?;
    let mut problem: Problem = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", args[2]))?;
    if let Some(n) = value("--threads") {
        problem.options.threads = n.parse().map_err(|_| "--threads takes a number".to_string())?;
    }
    let out = value("-o").ok_or("-o FILE is required")?;
    if args[1] == "masks" {
        problem.validate().map_err(|e| e.to_string())?;
        let key = value("--net").ok_or("--net KEY is required")?;
        let net = problem.nets.iter().find(|n| n.key == key).ok_or(format!("no net with the key {key}"))?;
        let grid = Grid::new(problem.window, problem.step);
        std::fs::write(&out, net_mask(&problem, &grid, net)).map_err(|e| format!("cannot write {out}: {e}"))?;
        println!("{} x {} cells, {:.2} s", grid.w, grid.h, start.elapsed().as_secs_f64());
        return Ok(ExitCode::SUCCESS);
    }
    let read = start.elapsed().as_secs_f64();
    let trace = flag("--trace", "PCB_ROUTING_TRACE");
    let verify = flag("--verify", "PCB_ROUTING_CHECK");
    let mut router = Router::new(&problem).verify_search(verify);
    if let Some(v) = value("--only").or_else(|| std::env::var("PCB_ROUTING_ONLY").ok()) {
        router = router.only_attempt(v.parse().map_err(|_| "--only takes an attempt number".to_string())?);
    }
    if trace || verify {
        router = router.on_event(move |event| match event {
            Event::Round { attempt, round, relaid, in_conflict, present } if trace => {
                eprintln!("attempt {attempt} round {round}: laid {relaid} nets, {in_conflict} still in conflict, present {present:.2}")
            }
            Event::Mismatch { net, two_pass, one_pass } => eprintln!("CHECK FAILED {net}: two passes {two_pass:?}, one pass {one_pass:?} (cost in grid steps)"),
            _ => {}
        });
    }
    let solution = router.run().map_err(|e| e.to_string())?;
    std::fs::write(&out, serde_json::to_string(&solution).map_err(|e| e.to_string())?)
        .map_err(|e| format!("cannot write {out}: {e}"))?;
    let score = solution.score.map_or("none".to_string(), |s| format!("{s:.1}"));
    println!(
        "{} after {} rounds (attempt {}), score {score}, read {read:.2} s, total {:.2} s",
        solution.status,
        solution.iterations,
        solution.variant,
        start.elapsed().as_secs_f64()
    );
    if trace {
        for (name, seconds) in &solution.stats.seconds {
            eprintln!("time, all attempts together: {name} {seconds:.2} s");
        }
        eprintln!("states taken out: first pass {}, second pass {}", solution.stats.swept, solution.stats.expanded);
    }
    if verify {
        eprintln!("searches whose two-pass cost differed from the one-pass cost: {}", solution.stats.mismatches);
    }
    for n in &solution.nets {
        let mut layers: Vec<&str> = n.tracks.iter().map(|t| t.layer.as_str()).collect();
        layers.sort();
        layers.dedup();
        println!("  {:<22} {:<18} {:7.2} mm  vias {:2}  {}", n.key, n.net, n.length_mm, n.vias.len(), layers.join(" "));
    }
    for c in solution.conflicts.iter().take(30) {
        println!(
            "  CONFLICT {:?} {} with {:?} at {:?} {}",
            c.kind,
            c.key,
            c.with,
            c.xy,
            c.layer.clone().unwrap_or_default()
        );
    }
    if !solution.message.is_empty() {
        println!("  {}", solution.message);
    }
    Ok(if solution.status == Status::Routed { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

/// Answer a question about zones: from a file or standard input, to a file or standard output.
fn pours(question: Option<&String>, out: Option<String>, threads: usize) -> Result<ExitCode, String> {
    use std::io::Read;
    let (name, text) = match question {
        Some(path) => (path.as_str(), std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?),
        None => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text).map_err(|e| format!("cannot read the question: {e}"))?;
            ("the question", text)
        }
    };
    let ask: pcb_routing::pours::Ask = serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|e| format!("cannot start threads: {e}"))?;
    let answer = pool.install(|| pcb_routing::pours::answer(&ask)).map_err(|e| match e {
        pcb_routing::Error::Invalid { field, reason } => format!("{name}: {field}: {reason}"),
        other => other.to_string(),
    })?;
    let json = serde_json::to_string(&answer).map_err(|e| e.to_string())?;
    match out {
        Some(path) => std::fs::write(&path, json).map_err(|e| format!("cannot write {path}: {e}"))?,
        None => println!("{json}"),
    }
    Ok(ExitCode::SUCCESS)
}
