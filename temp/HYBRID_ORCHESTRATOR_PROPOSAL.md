# RoboSync 3.0: Hybrid Orchestrator Architecture Proposal

## Executive Summary

**Current State**: RoboSync 2.0 attempts to replace rsync/robocopy but is 1.75x-4.6x slower on its primary use case (small files).

**Proposed Pivot**: Transform RoboSync into an intelligent orchestrator that automatically selects the optimal transfer method for each scenario, combining native tools where they excel with custom implementations where we've proven superiority.

## Problem Statement

### Original Goal
"Solve rsync's lack of parallel transfers for small files"

### Discovered Reality
- Parallel coordination overhead (50-90ms) exceeds small file transfer time (0.1-1ms)
- This is a physics problem, not an implementation problem
- Native tools have 40+ years of optimization for their specific domains

### Key Insight
**No single tool is optimal for all scenarios.** The winner depends on:
- File count and sizes
- Source/destination filesystems  
- Network vs local transfer
- Platform-specific features availability

## Proposed Solution: Hybrid Orchestrator

### Core Concept
RoboSync becomes a **smart dispatcher** that:
1. Profiles the workload (file count, sizes, filesystem types)
2. Selects the optimal transfer strategy
3. Delegates to native tools OR uses built-in optimized paths
4. Provides consistent cross-platform interface

### Architecture

```rust
enum TransferStrategy {
    NativeRsync(RsyncArgs),        // Small files, network delta
    NativeRobocopy(RobocopyArgs),  // Windows-specific features
    NativeCp(CpArgs),              // Same-filesystem reflinks
    RoboSyncMmap,                  // Large files (proven 4-12x faster)
    RoboSyncParallel,              // Medium files, mixed workloads
    RoboSyncSparse,                // Sparse files (proven 2.5x faster)
    FpsyncParallel,                // Directory-level parallelization
}

impl RoboSync {
    fn analyze_workload(&self) -> WorkloadProfile {
        // Quick sampling of source directory
        // Filesystem detection
        // Network vs local determination
    }
    
    fn select_strategy(&self, profile: WorkloadProfile) -> TransferStrategy {
        match profile {
            // Small files: delegate to rsync
            Profile { file_count: n, avg_size: s, .. } 
                if n > 1000 && s < 100_000 => {
                TransferStrategy::NativeRsync(optimized_args)
            },
            
            // Large files: use our proven mmap implementation
            Profile { total_size: size, .. } 
                if size > 100_000_000 => {
                TransferStrategy::RoboSyncMmap
            },
            
            // Same filesystem with reflink: use native cp
            Profile { source_fs, dest_fs, .. }
                if source_fs == dest_fs && supports_reflink => {
                TransferStrategy::NativeCp(reflink_args)
            },
            
            // Directory tree: parallelize at directory level
            Profile { dir_count: d, .. }
                if d > 10 => {
                TransferStrategy::FpsyncParallel
            },
            
            _ => TransferStrategy::RoboSyncParallel
        }
    }
}
```

## Performance Projections

### Based on Current Test Data

| Scenario | Current RoboSync | Native Tool | Hybrid Approach | Expected Speedup |
|----------|-----------------|-------------|-----------------|------------------|
| Small files (5000×1KB) | 0.640s | rsync: 0.365s | rsync: 0.365s | Match best tool |
| Large files (50×30MB) | 1s | rsync: 4s | RoboSync: 1s | 4x faster |
| Mixed workload | 15s | rsync: 2s | rsync: 2s | Match best tool |
| Sparse files | 0.116s | cp: 0.389s | RoboSync: 0.116s | 3x faster |
| Windows small | 646ms | robocopy: 141ms | robocopy: 141ms | Match best tool |
| Same-FS copy | Variable | cp --reflink: instant | cp: instant | Optimal |

**Result**: Always optimal performance, never worse than native tools.

## Implementation Strategy

### Phase 1: Workload Profiler (2-3 days)
- Implement fast sampling algorithm (stat first 100 files)
- Filesystem type detection (already have this)
- Network vs local detection (already have this)
- Decision matrix based on test data

### Phase 2: Native Tool Integration (3-4 days)
- Shell out to rsync with optimized arguments
- Shell out to robocopy on Windows
- Shell out to cp for same-filesystem copies
- Argument translation layer (RoboSync args → native args)

### Phase 3: Optimization (2-3 days)
- Cache profiling results for repeated transfers
- Fine-tune decision thresholds based on real-world usage
- Add manual override options (`--force-strategy=rsync`)

### Phase 4: Advanced Features (optional, 5-7 days)
- Directory-level parallelization (like fpsync)
- Automatic chunking for large transfers
- Progress aggregation from multiple native processes
- Unified error handling across all strategies

## Benefits

### For Users
1. **Optimal performance always** - Never slower than native tools
2. **Consistent interface** - Same commands on all platforms
3. **Zero configuration** - Automatic strategy selection
4. **Backwards compatible** - Existing RoboSync commands still work

### For Development
1. **Reduced complexity** - Stop fighting physics problems
2. **Focused optimization** - Only optimize where we can win
3. **Maintainable** - Clear separation of concerns
4. **Extensible** - Easy to add new strategies

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Dependency on external tools | Graceful fallback to built-in implementation |
| Profiling overhead | Cache results, sample-based profiling |
| Complex decision matrix | Start simple, refine based on telemetry |
| User confusion | Clear logging of strategy selection with `--verbose` |

## Success Metrics

1. **Performance**: Never >10% slower than optimal native tool
2. **Adoption**: Users prefer this over manually choosing tools
3. **Simplicity**: Decision logic under 500 lines of code
4. **Reliability**: <0.1% wrong strategy selections

## Alternative Approaches Considered

### Pure Rust Rewrite of rsync
- **Rejected**: Would be 10-20% slower due to bounds checking
- **Effort**: 6+ months to reach parity

### Fix Current Architecture
- **Rejected**: Parallel overhead for small files is unavoidable
- **Physics**: Thread coordination will always exceed transfer time

### Abandon Project
- **Rejected**: We've proven value in specific scenarios
- **Opportunity**: Hybrid approach leverages our wins

## Conclusion

RoboSync's attempt to universally beat native tools has failed. However, we've discovered scenarios where our approach is superior (large files, sparse files, reflinks). 

By pivoting to a hybrid orchestrator model, we can:
- Always deliver optimal performance
- Provide value through intelligent automation
- Maintain the consistent cross-platform interface
- Stop fighting unwinnable battles

**Recommendation**: Proceed with hybrid orchestrator architecture.

## Questions for Review

1. Should we prioritize Windows or Linux for initial implementation?
2. What level of native tool dependency is acceptable?
3. Should we support manual strategy override from day one?
4. How transparent should strategy selection be to users?
5. Should we maintain pure-Rust fallback for all scenarios?

---

*Prepared by: roboclaude*  
*Date: 2025-08-06*  
*Version: 1.0*  
*Status: PROPOSAL - Awaiting Review*