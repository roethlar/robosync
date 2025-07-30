//! Example showing how to use RoboSync's error reporting functionality

use robosync::error_report::{ErrorReporter, ErrorReportHandle};
use robosync::metadata::{copy_file_with_metadata_and_reporter, CopyFlags};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = Path::new("/source/path");
    let dest = Path::new("/dest/path");
    
    // Create an error reporter
    let error_reporter = ErrorReporter::new(source, dest);
    let error_handle = error_reporter.get_handle();
    
    // Example: Copy files with error reporting
    let copy_flags = CopyFlags::from_string("DATSOU");
    
    // This would normally be in a loop copying many files
    let test_source = Path::new("test_file.txt");
    let test_dest = Path::new("/tmp/test_file.txt");
    
    match copy_file_with_metadata_and_reporter(test_source, test_dest, &copy_flags, Some(&error_handle)) {
        Ok(bytes) => println!("Copied {} bytes", bytes),
        Err(e) => {
            error_handle.add_error(test_source, &e.to_string());
            eprintln!("Error copying file");
        }
    }
    
    // Write the error report
    match error_reporter.write_report() {
        Ok(Some(report_path)) => {
            println!("Error report saved to: {}", report_path.display());
        }
        Ok(None) => {
            println!("No errors or warnings to report");
        }
        Err(e) => {
            eprintln!("Failed to write error report: {}", e);
        }
    }
    
    Ok(())
}