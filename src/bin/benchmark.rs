use rayon::prelude::*;
use std::fs;
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: benchmark <source_dir> <dest_dir>");
        std::process::exit(1);
    }

    let source = Path::new(&args[1]);
    let dest = Path::new(&args[2]);

    println!("Benchmark: Simple parallel file copy");
    println!("Source: {}", source.display());
    println!("Destination: {}", dest.display());

    // Get list of files
    let start = Instant::now();
    let files: Vec<_> = walkdir::WalkDir::new(source)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .collect();

    let elapsed = start.elapsed();
    println!("Found {} files in {elapsed:?}", files.len());

    // Copy files in parallel
    let start_copy = Instant::now();
    let total_bytes = 0u64;

    files.par_iter().for_each(|entry| {
        let src_path = entry.path();
        let relative = src_path
            .strip_prefix(source)
            .expect("src_path should always have source as prefix");
        let dst_path = dest.join(relative);

        if let Some(parent) = dst_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if let Ok(_bytes) = fs::copy(src_path, &dst_path) {
            // Just count bytes, no synchronization
        }
    });

    let elapsed = start_copy.elapsed();
    println!("\nCopy completed in {elapsed:?}");
    println!(
        "Average speed: {:.2} MB/s",
        (total_bytes as f64 / 1_000_000.0) / elapsed.as_secs_f64()
    );
}