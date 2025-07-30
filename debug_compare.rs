use std::fs;
use std::path::Path;
use std::time::SystemTime;

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() < 3 {
        println!("Usage: {} <source_dir> <dest_dir>", args[0]);
        return;
    }

    let source_dir = Path::new(&args[1]);
    let dest_dir = Path::new(&args[2]);

    println!("Debug File Comparison");
    println!("Source: {}", source_dir.display()); 
    println!("Dest: {}", dest_dir.display());
    println!();

    // Get first few files from each directory
    let source_files = get_sample_files(source_dir, 10);
    let dest_files = get_sample_files(dest_dir, 10);

    println!("=== SOURCE FILES ===");
    for (path, size, modified) in &source_files {
        println!("Path: {}", path.display());
        println!("  Size: {} bytes", size);
        println!("  Modified: {:?}", modified);
        println!();
    }

    println!("=== DESTINATION FILES ===");
    for (path, size, modified) in &dest_files {
        println!("Path: {}", path.display());
        println!("  Size: {} bytes", size);
        println!("  Modified: {:?}", modified);
        println!();
    }

    println!("=== COMPARISON ===");
    for (source_path, source_size, source_modified) in &source_files {
        if let Some(rel_path) = source_path.strip_prefix(source_dir).ok() {
            let dest_path = dest_dir.join(rel_path);
            
            if let Some((_, dest_size, dest_modified)) = dest_files.iter()
                .find(|(p, _, _)| p.strip_prefix(dest_dir).ok() == Some(rel_path)) {
                
                println!("Comparing: {}", rel_path.display());
                println!("  Source size: {}, Dest size: {}", source_size, dest_size);
                println!("  Source modified: {:?}", source_modified);
                println!("  Dest modified: {:?}", dest_modified);
                
                let size_match = source_size == dest_size;
                let time_match = source_modified <= dest_modified;
                let needs_update = !size_match || !time_match;
                
                println!("  Size match: {}", size_match);
                println!("  Time match: {} (source <= dest)", time_match);
                println!("  NEEDS UPDATE: {}", needs_update);
                println!();
            } else {
                println!("File not found in destination: {}", rel_path.display());
                println!("  NEEDS COPY: true");
                println!();
            }
        }
    }
}

fn get_sample_files(dir: &Path, limit: usize) -> Vec<(std::path::PathBuf, u64, SystemTime)> {
    let mut files = Vec::new();
    
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.take(limit) {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(metadata) = entry.metadata() {
                        let size = metadata.len();
                        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                        files.push((path, size, modified));
                    }
                }
            }
        }
    }
    
    files
}