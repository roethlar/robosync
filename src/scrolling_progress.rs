//! Scrolling progress display with fixed header

use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::io::{self, Write};
use std::collections::VecDeque;
use crossterm::{cursor, terminal, QueueableCommand};

const MAX_LOG_LINES: usize = 8;  // Number of scrolling lines to show

pub struct ScrollingProgress {
    start_time: Instant,
    source: String,
    dest: String,
    options: String,
    log_lines: Arc<Mutex<VecDeque<String>>>,
    stats: Arc<Mutex<ProgressStats>>,
    last_refresh: Arc<Mutex<Instant>>,
}

#[derive(Default)]
pub struct ProgressStats {
    pub dirs_total: u64,
    pub dirs_created: u64,
    pub dirs_skipped: u64,
    pub files_total: u64,
    pub files_copied: u64,
    pub files_skipped: u64,
    pub files_failed: u64,
    pub files_deleted: u64,
    pub bytes_total: u64,
    pub bytes_copied: u64,
    pub bytes_skipped: u64,
}

impl ScrollingProgress {
    pub fn new(source: String, dest: String, options: String) -> Self {
        Self {
            start_time: Instant::now(),
            source,
            dest,
            options,
            log_lines: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES))),
            stats: Arc::new(Mutex::new(ProgressStats::default())),
            last_refresh: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Add a log line to the scrolling display
    pub fn log(&self, message: String) {
        let mut lines = self.log_lines.lock().unwrap();
        lines.push_back(message);
        if lines.len() > MAX_LOG_LINES {
            lines.pop_front();
        }
        self.refresh();
    }

    /// Update statistics
    pub fn update_stats<F>(&self, updater: F) 
    where 
        F: FnOnce(&mut ProgressStats)
    {
        let mut stats = self.stats.lock().unwrap();
        updater(&mut *stats);
        drop(stats);
        self.refresh();
    }

    /// Check if we should refresh the display
    fn refresh(&self) {
        let now = Instant::now();
        let should_refresh = {
            let last = self.last_refresh.lock().unwrap();
            now.duration_since(*last) >= std::time::Duration::from_millis(100)
        };
        
        if should_refresh {
            *self.last_refresh.lock().unwrap() = now;
            self.refresh_display();
        }
    }

    /// Refresh the entire display
    pub fn refresh_display(&self) {
        let mut stdout = io::stdout();
        
        // Move to top and clear screen
        let _ = stdout.queue(cursor::MoveTo(0, 0));
        let _ = stdout.queue(terminal::Clear(terminal::ClearType::FromCursorDown));
        
        // Print header
        println!("-------------------------------------------------------------------------------");
        println!("   ROBOSYNC     ::     High-Performance File Synchronization");
        println!("-------------------------------------------------------------------------------");
        println!();
        println!("  Started : {}", chrono::Local::now().format("%A, %B %d, %Y %I:%M:%S %p"));
        println!("   Source : {}", self.source);
        println!("     Dest : {}", self.dest);
        println!();
        println!("    Files : *.*");
        println!();
        println!("  Options : {}", self.options);
        println!();
        println!("------------------------------------------------------------------------------");
        println!();
        
        // Print scrolling log area
        let lines = self.log_lines.lock().unwrap();
        for line in lines.iter() {
            println!("  {}", line);
        }
        // Pad empty lines
        for _ in lines.len()..MAX_LOG_LINES {
            println!();
        }
        
        println!();
        println!("------------------------------------------------------------------------------");
        println!();
        
        // Print statistics
        let stats = self.stats.lock().unwrap();
        let elapsed = self.start_time.elapsed();
        
        println!("               Total    Copied   Skipped  Mismatch    FAILED    Extras");
        println!("    Dirs : {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
            stats.dirs_total,
            stats.dirs_created,
            stats.dirs_skipped,
            0,  // mismatch not tracked
            0,  // dir failures not tracked separately
            0   // extras handled as deletes
        );
        println!("   Files : {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
            stats.files_total,
            stats.files_copied,
            stats.files_skipped,
            0,  // mismatch not tracked
            stats.files_failed,
            stats.files_deleted
        );
        println!("   Bytes : {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
            format_size(stats.bytes_total),
            format_size(stats.bytes_copied),
            format_size(stats.bytes_skipped),
            "0",  // mismatch
            "0",  // failed bytes not tracked
            "0"   // extra bytes not tracked
        );
        println!("   Times : {:>9}   {:>6}                       0:00:00   0:00:00",
            format_duration(elapsed),
            format_duration(elapsed)
        );
        
        let _ = stdout.flush();
    }

    /// Final display
    pub fn finish(&self) {
        self.refresh_display();
        println!("   Ended : {}", chrono::Local::now().format("%A, %B %d, %Y %I:%M:%S %p"));
        println!();
    }
}

/// Format bytes to human-readable size
fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0".to_string();
    }
    
    const UNITS: &[&str] = &["", "k", "m", "g", "t"];
    let exponent = (bytes as f64).log(1024.0).floor() as usize;
    let exponent = exponent.min(UNITS.len() - 1);
    
    if exponent == 0 {
        format!("{}", bytes)
    } else {
        let value = bytes as f64 / 1024_f64.powi(exponent as i32);
        format!("{:.1}{}", value, UNITS[exponent])
    }
}

/// Format duration as HH:MM:SS
fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;
    
    format!("{:>2}:{:02}:{:02}", hours, mins, secs)
}