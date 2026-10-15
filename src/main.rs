use justrust_engine::fixtures::{FixtureProvider, Shape};
use justrust_engine::{Engine, Error};
use std::io::Write;
use std::time::Instant;

const HELP: &str = "jr-prototype [--shape wide|chain|diamond|dynamic|fixed-point|multi-output|build-check-test]\n  [--workers N] [--size N] [--work N] [--seed N]\n  [--revision FULL_SHA] [--trace NEW_FILE] [--no-reference]\n\nSynthetic graph engine only, not a Rust compiler. Accounting always enabled.\nMeasurements are contended/uncontrolled unless external isolation is established.\n--work controls deterministic CPU rounds per operation. --trace retains events\nand observed edges as Debug records, never overwriting an existing file.\n";

fn main() {
    if let Err(error) = run() {
        eprintln!("jr-prototype: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut workers = 1usize;
    let mut size = 1000u64;
    let mut work = 1000u64;
    let mut seed = 1u64;
    let mut name = String::from("wide");
    let mut revision = None;
    let mut trace = None;
    let mut reference = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            print!("{HELP}");
            return Ok(());
        }
        if arg == "--no-reference" {
            reference = false;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| Error::new(format!("missing value for {arg}")))?;
        match arg.as_str() {
            "--workers" => workers = value.parse()?,
            "--size" => size = value.parse()?,
            "--work" => work = value.parse()?,
            "--seed" => seed = value.parse()?,
            "--shape" => name = value,
            "--revision" => {
                if ![40, 64].contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(Error::new("revision must be a full Git SHA").into());
                }
                revision = Some(value);
            }
            "--trace" => trace = Some(value),
            _ => return Err(Error::new(format!("unknown option {arg}")).into()),
        }
    }
    if workers == 0 || workers > 4096 || size > 1_000_000 || work > 1_000_000_000 {
        return Err(
            Error::new("workers must be 1..4096, size <=1000000, work <=1000000000").into(),
        );
    }
    let shape = Shape::parse(&name)?;
    // Opening first avoids running an experiment then losing its requested trace.
    let mut trace = trace
        .map(|p| {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(p)
        })
        .transpose()?;
    let expected = reference.then(|| shape.reference(size, work, seed));
    let start = Instant::now();
    let engine = Engine::new(FixtureProvider);
    let demands: Vec<_> = shape
        .roots(size, work, seed)
        .into_iter()
        .map(|key| engine.request(key))
        .collect();
    let report = engine.run(workers)?;
    let total_ns = start.elapsed().as_nanos();
    for demand in &demands {
        demand.completion()?;
    }
    let results: Vec<_> = demands
        .iter()
        .map(|d| d.result(0))
        .collect::<Result<_, _>>()?;
    if let Some(expected) = expected
        && expected != results
    {
        return Err(Error::new(format!(
            "reference mismatch: expected {expected:?}, got {results:?}"
        ))
        .into());
    }
    if !report.capture_complete {
        return Err(Error::new("incomplete engine capture").into());
    }
    if let Some(file) = trace.as_mut() {
        writeln!(
            file,
            "revision={revision:?} shape={name} workers={workers} size={size} work={work} seed={seed}"
        )?;
        writeln!(file, "{report:#?}")?;
    }
    let revision = revision
        .map(|s| format!("\"{s}\""))
        .unwrap_or_else(|| "null".into());
    println!(
        "{{\"schema\":1,\"fixture\":\"{name}\",\"revision\":{revision},\"workers\":{workers},\"size\":{size},\"work\":{work},\"seed\":{seed},\"reference_checked\":{reference},\"measurement\":\"contended/uncontrolled\",\"total_ns\":{total_ns},\"engine_ns\":{},\"nodes\":{},\"intern_hits\":{},\"tasks\":{},\"polls\":{},\"publications\":{},\"edges\":{},\"failures\":{},\"queue_ns\":{},\"execution_ns\":{},\"wait_ns\":{},\"retained_outputs\":{},\"events_dropped\":{},\"capture_complete\":{},\"results\":{:?}}}",
        report.elapsed_ns,
        report.nodes,
        report.intern_hits,
        report.tasks,
        report.polls,
        report.publications,
        report.edges,
        report.failures,
        report.queue_ns,
        report.execution_ns,
        report.wait_ns,
        report.retained_outputs,
        report.events_dropped,
        report.capture_complete,
        results
    );
    Ok(())
}
