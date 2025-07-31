//! Formatted display output for RoboSync

use crate::color_output::ConditionalColor;
use crate::sync_stats::SyncStats;
use crossterm::style::Color;
use indicatif::{ProgressBar, ProgressStyle};

/// Display formatted header
pub fn print_header(
    version: &str,
    source: &str,
    dest: &str,
    include: &str,
    exclude: &[String],
    options: &str,
) {
    println!("  ───────────────────────────────────────────────────────────────────────────────");
    println!(
        "     RoboSync {version}: Fast parallel file synchronization"
    );
    println!("  ───────────────────────────────────────────────────────────────────────────────");

    // Calculate max width for proper alignment
    let max_len = source.len().max(dest.len()).max(50);

    println!("    ╭────────┬{}╮", "─".repeat(max_len + 2));
    println!("    │ Source │ {source:<max_len$} │");
    println!("    ├────────┼{}┤", "─".repeat(max_len + 2));
    println!("    │ Dest   │ {dest:<max_len$} │");

    if !include.is_empty() && include != "*.*" {
        println!("    ├────────┼{}┤", "─".repeat(max_len + 2));
        println!("    │ Incl.  │ {include:<max_len$} │");
    }

    if !exclude.is_empty() {
        let exclude_str = exclude.join(" ");
        println!("    ├────────┼{}┤", "─".repeat(max_len + 2));
        println!("    │ Excl.  │ {exclude_str:<max_len$} │");
    }

    if !options.is_empty() {
        println!("    ├────────┼{}┤", "─".repeat(max_len + 2));
        println!("    │ Options│ {options:<max_len$} │");
    }

    println!("    └────────┴{}┘", "─".repeat(max_len + 2));
    println!("  ───────────────────────────────────────────────────────────────────────────────");
}

/// Display file analysis results
pub fn print_file_analysis(
    total_files: u64,
    small_files: u64,
    medium_files: u64,
    large_files: u64,
    total_size: u64,
    small_size: u64,
    medium_size: u64,
    large_size: u64,
) {
    println!(
        "\n     {}",
        "File Analysis Complete:".color_bold_if(Color::Cyan)
    );
    println!();
    println!(
        "     {}  {:>8} {}   ({:>8} {}, {:>7} {}, {:>3} {})",
        "Files:".color_if(Color::White),
        format_number(total_files).color_bold_if(Color::White),
        "total".color_if(Color::White),
        format_number(small_files).color_if(Color::Green),
        "small".color_if(Color::Green),
        format_number(medium_files).color_if(Color::Yellow),
        "medium".color_if(Color::Yellow),
        format_number(large_files).color_if(Color::Red),
        "large".color_if(Color::Red)
    );
    println!(
        "     {}   {:>8} {}   ({:>8} {}, {:>7} {}, {:>3} {})",
        "Size:".color_if(Color::White),
        format_bytes(total_size).color_bold_if(Color::White),
        "total".color_if(Color::White),
        format_bytes(small_size).color_if(Color::Green),
        "small".color_if(Color::Green),
        format_bytes(medium_size).color_if(Color::Yellow),
        "medium".color_if(Color::Yellow),
        format_bytes(large_size).color_if(Color::Red),
        "large".color_if(Color::Red)
    );
}

/// Display pending operations
pub fn print_pending_operations(
    files_create: u64,
    files_update: u64,
    files_delete: u64,
    files_skip: u64,
    dirs_create: u64,
    dirs_update: u64,
    dirs_delete: u64,
    dirs_skip: u64,
    size_create: u64,
    size_update: u64,
    size_delete: u64,
    size_skip: u64,
) {
    let _files_total = files_create + files_update + files_delete + files_skip;
    let _dirs_total = dirs_create + dirs_update + dirs_delete + dirs_skip;
    let _size_total = size_create + size_update + size_delete + size_skip;

    println!(
        "\n     {}",
        "Pending Operations:".color_bold_if(Color::Cyan)
    );
    println!();

    // Simple list format - much cleaner
    if files_create > 0 {
        println!(
            "     Files to create: {}",
            format_number(files_create).color_if(Color::Green)
        );
    }
    if files_update > 0 {
        println!(
            "     Files to update: {}",
            format_number(files_update).color_if(Color::Yellow)
        );
    }
    if files_delete > 0 {
        println!(
            "     Files to delete: {}",
            format_number(files_delete).color_if(Color::Red)
        );
    }
    if dirs_create > 0 {
        println!(
            "     Directories to create: {}",
            format_number(dirs_create).color_if(Color::Green)
        );
    }
    if dirs_delete > 0 {
        println!(
            "     Directories to delete: {}",
            format_number(dirs_delete).color_if(Color::Red)
        );
    }

    let total_operations = files_create + files_update + files_delete + dirs_create + dirs_delete;
    println!(
        "\n     Total: {} operations, {} transfer size",
        format_number(total_operations).color_bold_if(Color::White),
        format_bytes_short(size_create + size_update).color_bold_if(Color::White)
    );
}

/// Display sync summary
pub fn print_sync_summary(
    stats: &SyncStats,
    skipped_files: u64,
    skipped_dirs: u64,
    skipped_size: u64,
) {
    println!("\n     {}", "Sync Summary:".color_bold_if(Color::Cyan));
    println!();
    println!(
        "     {:>6} {:>8} {:>8} {:>8} {:>8} {:>9}",
        "",
        "Copied".color_if(Color::White),
        "Updated".color_if(Color::White),
        "Deleted".color_if(Color::White),
        "Failed".color_if(Color::White),
        "Skipped".color_if(Color::White)
    );
    println!(
        "     {:>6} {:>8} {:>8} {:>8} {:>8} {:>9}",
        "──────", "────────", "────────", "────────", "────────", "─────────"
    );
    println!(
        "     Files  {:>8} {:>8} {:>8} {:>8} {:>9}",
        format_number(stats.files_copied()),
        "0", // Updated tracked separately in our case
        format_number(stats.files_deleted()),
        format_number(stats.errors()),
        format_number(skipped_files)
    );
    println!(
        "     Dirs   {:>8} {:>8} {:>8} {:>8} {:>9}",
        "0", // Dir stats not tracked separately
        "0",
        "0",
        "0",
        format_number(skipped_dirs)
    );
    println!(
        "     Size   {:>8} {:>8} {:>8} {:>8} {:>9}",
        format_bytes_short(stats.bytes_transferred()),
        "0 B",
        "0 B",
        "0 B",
        format_bytes_short(skipped_size)
    );
}

/// Display worker performance
pub fn print_worker_performance(workers: Vec<WorkerStats>) {
    if workers.is_empty() {
        return;
    }

    println!(
        "\n     {}",
        "Worker Performance:".color_bold_if(Color::Cyan)
    );
    println!();

    for worker in workers.iter() {
        // Skip the "Delete operations" worker if it exists
        if worker.name.contains("Delete") {
            continue;
        }

        let worker_color = match worker.name.as_str() {
            "Large" => Color::Red,
            "Medium" => Color::Yellow,
            "Small" => Color::Green,
            _ => Color::White,
        };

        println!(
            "     {}: {} files, {} in {:.1}s ({}/s)",
            worker.name.as_str().color_if(worker_color),
            format_number(worker.files).color_if(Color::White),
            format_bytes_short(worker.bytes).color_if(Color::White),
            worker.duration_secs,
            format_bytes_short(worker.throughput)
                .as_str()
                .color_if(Color::Cyan)
        );
    }
}

/// Worker statistics
pub struct WorkerStats {
    pub name: String,
    pub files: u64,
    pub bytes: u64,
    pub duration_secs: f32,
    pub throughput: u64,
}

/// Create progress bar with custom style
pub fn create_progress_bar(total: u64) -> ProgressBar {
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("\n  [{bar:40}] {pos}/{len} | {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("█▓░"),
    );
    pb
}

/// Format number with thousands separator
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();

    for (i, &ch) in chars.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }

    result.chars().rev().collect()
}

/// Format bytes to human readable string
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let exponent = (bytes as f64).log(1024.0).floor() as usize;
    let exponent = exponent.min(UNITS.len() - 1);
    let value = bytes as f64 / 1024_f64.powi(exponent as i32);
    if exponent == 0 {
        format!("{} {}", bytes, UNITS[exponent])
    } else {
        format!("{:.1} {}", value, UNITS[exponent])
    }
}

/// Format bytes to short form (for tables)
fn format_bytes_short(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let exponent = (bytes as f64).log(1024.0).floor() as usize;
    let exponent = exponent.min(UNITS.len() - 1);
    let value = bytes as f64 / 1024_f64.powi(exponent as i32);

    if exponent == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{:.0} {}", value, UNITS[exponent])
    } else if value >= 10.0 {
        format!("{:.1} {}", value, UNITS[exponent])
    } else {
        format!("{:.2} {}", value, UNITS[exponent])
    }
}
