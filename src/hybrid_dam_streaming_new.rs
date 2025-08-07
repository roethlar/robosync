    /// Execute synchronization with TRUE streaming file discovery to eliminate startup latency
    /// This version processes files ONE AT A TIME as they're discovered (per Gemini's mandate)
    pub fn execute_streaming(
        &self,
        source_root: &Path,
        dest_root: &Path,
        options: &SyncOptions,
    ) -> Result<SyncStats> {
        use crate::streaming_walker::StreamingWalker;
        use std::sync::mpsc;
        use std::thread;
        
        // Create channel for TRUE streaming operations
        let (tx, rx) = mpsc::channel();
        
        // Clone values for the walker thread
        let source_path = source_root.to_path_buf();
        let dest_path = dest_root.to_path_buf();
        let walker_options = options.clone();
        
        // Spawn walker thread that sends operations AS THEY'RE DISCOVERED (not batched!)
        let walker_thread = thread::spawn(move || {
            StreamingWalker::discover_files_to_channel(
                source_path,
                dest_path,
                walker_options,
                tx
            )
        });
        
        // Main application loop - process operations ONE AT A TIME as they arrive
        let mut total_stats = SyncStats::default();
        let mut dam_batch = Vec::new(); // Small batch for tar streaming efficiency only
        let dam_flush_threshold = self.config.dam_flush_threshold as usize;
        
        // Process each operation as it arrives from the walker thread
        for operation in rx {
            // Convert FileOperation to FileEntry if needed
            let file_entry = match self.operation_to_file_entry(&operation, source_root, dest_root) {
                Ok(entry) => entry,
                Err(e) => {
                    if options.verbose >= 1 {
                        eprintln!("⚠️  Warning: Skipping file due to error: {}", e);
                    }
                    total_stats.increment_errors();
                    continue;
                }
            };
            
            // Log file operation if verbose >= 2
            if options.verbose >= 2 {
                let operation_type = match &operation {
                    FileOperation::Create { .. } => "Copying",
                    FileOperation::Update { .. } => "Updating",
                    FileOperation::Delete { .. } => "Deleting",
                    FileOperation::CreateDirectory { .. } => "Creating directory",
                    FileOperation::CreateSymlink { .. } => "Creating symlink",
                    FileOperation::UpdateSymlink { .. } => "Updating symlink",
                };
                
                // Format file size
                let size_str = if file_entry.size == 0 {
                    String::new()
                } else {
                    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
                    let mut unit_index = 0;
                    let mut size = file_entry.size as f64;
                    
                    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
                        size /= 1024.0;
                        unit_index += 1;
                    }
                    
                    if unit_index == 0 {
                        format!(" ({} B)", file_entry.size)
                    } else {
                        format!(" ({:.2} {})", size, UNITS[unit_index])
                    }
                };
                
                println!("[{}] {}: {}{}", 
                    Self::timestamp(), 
                    operation_type,
                    file_entry.dst_path.display(),
                    size_str
                );
            }
            
            // Determine which tier this file belongs to and process immediately
            let strategy = self.determine_strategy(&file_entry);
            
            match strategy {
                TransferStrategy::Dam => {
                    // For small files, batch for efficiency (but still stream!)
                    dam_batch.push(file_entry);
                    
                    // Flush batch if threshold reached
                    if dam_batch.len() >= dam_flush_threshold {
                        let batch_to_process = std::mem::take(&mut dam_batch);
                        let total_size: u64 = batch_to_process.iter().map(|f| f.size).sum();
                        
                        match self.process_dam_batch(DamBatchJob {
                            files: batch_to_process,
                            total_size,
                            compression_enabled: false,
                        }) {
                            Ok(result) => {
                                total_stats.add_bytes_transferred(result.bytes_transferred);
                                for _ in 0..result.files_copied {
                                    total_stats.increment_files_copied();
                                }
                            }
                            Err(e) => {
                                if options.verbose >= 1 {
                                    eprintln!("Dam batch error: {}", e);
                                }
                                total_stats.increment_errors();
                            }
                        }
                    }
                }
                TransferStrategy::Pool => {
                    // Process medium files immediately
                    match self.pool.process_file(file_entry) {
                        Ok(result) => {
                            total_stats.add_bytes_transferred(result.bytes_transferred);
                            for _ in 0..result.files_copied {
                                total_stats.increment_files_copied();
                            }
                        }
                        Err(e) => {
                            if options.verbose >= 1 {
                                eprintln!("Pool processing error: {}", e);
                            }
                            total_stats.increment_errors();
                        }
                    }
                }
                TransferStrategy::Slicer => {
                    // Process large files immediately
                    match self.slicer.process_file(file_entry) {
                        Ok(result) => {
                            total_stats.add_bytes_transferred(result.bytes_transferred);
                            for _ in 0..result.files_copied {
                                total_stats.increment_files_copied();
                            }
                        }
                        Err(e) => {
                            if options.verbose >= 1 {
                                eprintln!("Slicer processing error: {}", e);
                            }
                            total_stats.increment_errors();
                        }
                    }
                }
            }
        }
        
        // Flush any remaining dam batch
        if !dam_batch.is_empty() {
            let total_size: u64 = dam_batch.iter().map(|f| f.size).sum();
            match self.process_dam_batch(DamBatchJob {
                files: dam_batch,
                total_size,
                compression_enabled: false,
            }) {
                Ok(result) => {
                    total_stats.add_bytes_transferred(result.bytes_transferred);
                    for _ in 0..result.files_copied {
                        total_stats.increment_files_copied();
                    }
                }
                Err(e) => {
                    if options.verbose >= 1 {
                        eprintln!("Final dam batch error: {}", e);
                    }
                    total_stats.increment_errors();
                }
            }
        }
        
        // Wait for walker thread to complete
        match walker_thread.join() {
            Ok(Ok(())) => {
                // Walker completed successfully
            }
            Ok(Err(e)) => {
                if options.verbose >= 1 {
                    eprintln!("⚠️  Warning: Walker encountered errors: {}", e);
                }
                total_stats.increment_errors();
            }
            Err(_) => {
                if options.verbose >= 1 {
                    eprintln!("⚠️  Warning: Walker thread panicked, but synchronization continued");
                }
                total_stats.increment_errors();
            }
        }
        
        Ok(total_stats)
    }