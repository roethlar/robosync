use anyhow::Result;
use clap::{Arg, Command};
use std::path::PathBuf;

fn main() -> Result<()> {
    println!("RoboSync Debug Test - Starting...");
    
    let matches = Command::new("RoboSync Test")
        .version("debug")
        .about("Debug version to test basic functionality")
        .arg(
            Arg::new("source")
                .help("Source directory or file")
                .required(true)
                .value_parser(clap::value_parser!(PathBuf))
        )
        .arg(
            Arg::new("destination")
                .help("Destination directory or file")
                .required(true)
                .value_parser(clap::value_parser!(PathBuf))
        )
        .arg(
            Arg::new("mirror")
                .long("mir")
                .help("Mirror a directory tree")
                .action(clap::ArgAction::SetTrue)
        )
        .arg(
            Arg::new("exclude-dirs")
                .long("xd")
                .value_name("PATTERN")
                .help("Exclude directories matching given patterns")
                .action(clap::ArgAction::Append)
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Produce verbose output")
                .action(clap::ArgAction::Count)
        )
        .get_matches();

    let source: PathBuf = matches.get_one::<PathBuf>("source").unwrap().clone();
    let destination: PathBuf = matches.get_one::<PathBuf>("destination").unwrap().clone();
    
    println!("Source: {}", source.display());
    println!("Destination: {}", destination.display());
    
    if matches.get_flag("mirror") {
        println!("Mirror mode: enabled");
    }
    
    if let Some(patterns) = matches.get_many::<String>("exclude-dirs") {
        println!("Exclude directories: {:?}", patterns.collect::<Vec<_>>());
    }
    
    let verbose_level = matches.get_count("verbose");
    println!("Verbose level: {}", verbose_level);
    
    // Check if directories exist
    println!("Checking source directory...");
    if source.exists() {
        println!("Source exists: {}", source.display());
        if source.is_dir() {
            println!("Source is a directory");
        } else {
            println!("Source is a file");
        }
    } else {
        println!("ERROR: Source does not exist!");
        return Err(anyhow::anyhow!("Source directory does not exist"));
    }
    
    println!("Checking destination directory...");
    if destination.exists() {
        println!("Destination exists: {}", destination.display());
    } else {
        println!("Destination does not exist - would be created");
    }
    
    println!("Basic test completed successfully!");
    println!("This confirms that:");
    println!("- Command line parsing works");
    println!("- File system access works");
    println!("- Basic logic flow works");
    println!("- The issue is likely in the actual sync implementation");
    
    Ok(())
}