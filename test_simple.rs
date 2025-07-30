use std::env;
use std::path::Path;

fn main() {
    println!("Simple RoboSync Test");
    
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 3 {
        println!("Usage: {} <source> <destination>", args[0]);
        return;
    }
    
    let source = Path::new(&args[1]);
    let destination = Path::new(&args[2]);
    
    println!("Source: {}", source.display());
    println!("Destination: {}", destination.display());
    
    // Check if directories exist
    println!("Checking source directory...");
    if source.exists() {
        println!("✓ Source exists");
        if source.is_dir() {
            println!("✓ Source is a directory");
            
            // Try to read the directory
            match std::fs::read_dir(source) {
                Ok(entries) => {
                    let mut count = 0;
                    for entry in entries {
                        match entry {
                            Ok(entry) => {
                                count += 1;
                                if count <= 5 {
                                    println!("  Found: {}", entry.path().display());
                                }
                            }
                            Err(e) => {
                                println!("  Error reading entry: {}", e);
                            }
                        }
                        if count >= 5 {
                            println!("  ... and more entries");
                            break;
                        }
                    }
                    println!("✓ Successfully read {} entries from source", count);
                }
                Err(e) => {
                    println!("✗ Error reading source directory: {}", e);
                }
            }
        } else {
            println!("Source is a file");
        }
    } else {
        println!("✗ Source does not exist!");
        return;
    }
    
    println!("Checking destination...");
    if destination.exists() {
        println!("✓ Destination exists");
    } else {
        println!("Destination does not exist - would be created");
    }
    
    println!("✓ Basic file system test completed successfully!");
}