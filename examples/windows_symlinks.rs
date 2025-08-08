//! Example demonstrating Windows symlink support in RoboSync
//!
//! This example shows how to use RoboSync to synchronize directories
//! containing symbolic links on Windows.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use robosync::windows_symlinks;
    use std::fs;
    use std::path::Path;

    println!("Windows Symlink Example for RoboSync");
    println!("====================================");
    println!();

    // Create a test directory structure
    let test_dir = Path::new("test_symlinks");
    if test_dir.exists() {
        fs::remove_dir_all(test_dir)?;
    }

    // Create source directory structure
    let source_dir = test_dir.join("source");
    fs::create_dir_all(&source_dir)?;

    // Create some files
    let file1 = source_dir.join("file1.txt");
    fs::write(&file1, "This is file 1")?;

    let file2 = source_dir.join("file2.txt");
    fs::write(&file2, "This is file 2")?;

    // Create a subdirectory
    let subdir = source_dir.join("subdir");
    fs::create_dir(&subdir)?;

    let file3 = subdir.join("file3.txt");
    fs::write(&file3, "This is file 3 in subdir")?;

    // Create symbolic links
    println!("Creating symbolic links...");
    println!("NOTE: This requires either:");
    println!("  1) Running as Administrator");
    println!("  2) Developer Mode enabled (Windows 10+)");
    println!("  3) SeCreateSymbolicLinkPrivilege granted");
    println!();

    // Create file symlink
    let link1 = source_dir.join("link_to_file1.txt");
    match windows_symlinks::create_symlink(&link1, &file1) {
        Ok(_) => println!(
            "✓ Created file symlink: {} -> {}",
            link1.display(),
            file1.display()
        ),
        Err(e) => println!("✗ Failed to create file symlink: {}", e),
    }

    // Create directory symlink
    let link2 = source_dir.join("link_to_subdir");
    match windows_symlinks::create_symlink(&link2, &subdir) {
        Ok(_) => println!(
            "✓ Created directory symlink: {} -> {}",
            link2.display(),
            subdir.display()
        ),
        Err(e) => println!("✗ Failed to create directory symlink: {}", e),
    }

    // Create relative symlink
    let link3 = source_dir.join("relative_link.txt");
    match windows_symlinks::create_symlink(&link3, Path::new("file2.txt")) {
        Ok(_) => println!(
            "✓ Created relative symlink: {} -> file2.txt",
            link3.display()
        ),
        Err(e) => println!("✗ Failed to create relative symlink: {}", e),
    }

    println!();
    println!("Now you can use RoboSync to synchronize this directory:");
    println!("  robosync {} destination_dir", source_dir.display());
    println!();
    println!("RoboSync will preserve all symbolic links during synchronization.");

    Ok(())
}

#[cfg(not(windows))]
fn main() {
    println!("This example is for Windows only.");
    println!("On Unix systems, symlink support is already built-in.");
}
