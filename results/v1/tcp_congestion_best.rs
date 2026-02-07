/// TCP Congestion Control - Lock-Free Implementation with Advanced AIMD
/// Uses atomic operations for thread-safe congestion control with BBR-inspired enhancements.

use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use crossbeam_epoch::{self as epoch, Atomic, Owned};

/// A network event (ACK or loss).
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    Ack {
        timestamp: f64,
        bytes: u64,
        rtt: f64,
    },
    Loss {
        timestamp: f64,
    },
}

impl NetworkEvent {
    pub fn timestamp(&self) -> f64 {
        match self {
            NetworkEvent::Ack { timestamp, .. } => *timestamp,
            NetworkEvent::Loss { timestamp } => *timestamp,
        }
    }
}

/// State of the congestion controller.
#[derive(Debug, Clone)]
pub struct CongestionState {
    pub cwnd: f64,
    pub ssthresh: f64,
    pub rtt_estimate: f64,
    pub in_slow_start: bool,
    pub bytes_acked: u64,
    pub losses: u64,
}

/// Result of running a congestion simulation.
#[derive(Debug, Clone)]
pub struct SimulationResult {
    pub throughput: f64,
    pub total_losses: u64,
    pub avg_cwnd: f64,
    pub goodput_ratio: f64,
}

const CWND_MIN: f64 = 1.0;
const CWND_MAX: f64 = 1024.0;

#[repr(C)]
struct AtomicState {
    cwnd_bits: AtomicU64,
    ssthresh_bits: AtomicU64,
    rtt_estimate_bits: AtomicU64,
    in_slow_start: AtomicBool,
    bytes_acked: AtomicU64,
    losses: AtomicU64,
    bandwidth_sum_bits: AtomicU64,
    bandwidth_count: AtomicU64,
}

impl AtomicState {
    fn new() -> Self {
        Self {
            cwnd_bits: AtomicU64::new(1.0f64.to_bits()),
            ssthresh_bits: AtomicU64::new(64.0f64.to_bits()),
            rtt_estimate_bits: AtomicU64::new(0.1f64.to_bits()),
            in_slow_start: AtomicBool::new(true),
            bytes_acked: AtomicU64::new(0),
            losses: AtomicU64::new(0),
            bandwidth_sum_bits: AtomicU64::new(0.0f64.to_bits()),
            bandwidth_count: AtomicU64::new(0),
        }
    }

    fn load_cwnd(&self) -> f64 {
        f64::from_bits(self.cwnd_bits.load(Ordering::Acquire))
    }

    fn store_cwnd(&self, value: f64) {
        self.cwnd_bits.store(value.to_bits(), Ordering::Release);
    }

    fn load_ssthresh(&self) -> f64 {
        f64::from_bits(self.ssthresh_bits.load(Ordering::Acquire))
    }

    fn store_ssthresh(&self, value: f64) {
        self.ssthresh_bits.store(value.to_bits(), Ordering::Release);
    }

    fn load_rtt_estimate(&self) -> f64 {
        f64::from_bits(self.rtt_estimate_bits.load(Ordering::Acquire))
    }

    fn store_rtt_estimate(&self, value: f64) {
        self.rtt_estimate_bits.store(value.to_bits(), Ordering::Release);
    }

    fn cas_cwnd(&self, current: f64, new: f64) -> Result<f64, f64> {
        match self.cwnd_bits.compare_exchange_weak(
            current.to_bits(),
            new.to_bits(),
            Ordering::AcqRel,
            Ordering::Acquire
        ) {
            Ok(_) => Ok(new),
            Err(actual_bits) => Err(f64::from_bits(actual_bits)),
        }
    }

    fn update_bandwidth(&self, bandwidth: f64) {
        // Lock-free bandwidth tracking
        loop {
            let current_sum_bits = self.bandwidth_sum_bits.load(Ordering::Acquire);
            let current_sum = f64::from_bits(current_sum_bits);
            let new_sum = current_sum + bandwidth;
            
            match self.bandwidth_sum_bits.compare_exchange_weak(
                current_sum_bits,
                new_sum.to_bits(),
                Ordering::AcqRel,
                Ordering::Acquire
            ) {
                Ok(_) => {
                    self.bandwidth_count.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                Err(_) => continue,
            }
        }
    }

    fn get_avg_bandwidth(&self) -> f64 {
        let count = self.bandwidth_count.load(Ordering::Acquire);
        if count == 0 {
            return 0.0;
        }
        let sum = f64::from_bits(self.bandwidth_sum_bits.load(Ordering::Acquire));
        sum / count as f64
    }
}

/// Run enhanced AIMD congestion control with lock-free atomic operations.
pub fn congestion_control(events: &[NetworkEvent]) -> SimulationResult {
    debug_assert!(!events.is_empty(), "Must have at least one event");

    let state = AtomicState::new();
    let cwnd_sum = AtomicU64::new(0);
    let count = AtomicU64::new(0);
    
    for event in events {
        match event {
            NetworkEvent::Ack { bytes, rtt, timestamp: _ } => {
                state.bytes_acked.fetch_add(*bytes, Ordering::Relaxed);

                // Update RTT estimate
                if *rtt > 0.0 {
                    let current_rtt = state.load_rtt_estimate();
                    let alpha = 0.125;
                    let new_rtt = (1.0 - alpha) * current_rtt + alpha * rtt;
                    state.store_rtt_estimate(new_rtt);

                    // Track bandwidth samples
                    let bandwidth = *bytes as f64 / rtt;
                    state.update_bandwidth(bandwidth);
                }

                // Lock-free congestion window update
                loop {
                    let current_cwnd = state.load_cwnd();
                    let is_slow_start = state.in_slow_start.load(Ordering::Acquire);
                    let ssthresh = state.load_ssthresh();

                    let new_cwnd = if is_slow_start {
                        let increased = current_cwnd + 1.0;
                        if increased >= ssthresh {
                            state.in_slow_start.store(false, Ordering::Release);
                        }
                        increased
                    } else {
                        // Enhanced congestion avoidance with bandwidth estimation
                        let base_increase = 1.0 / current_cwnd;
                        let rtt_factor = (0.1 / state.load_rtt_estimate()).min(2.0).max(0.5);
                        let bandwidth_factor = {
                            let avg_bw = state.get_avg_bandwidth();
                            if avg_bw > 0.0 {
                                let current_bw = *bytes as f64 / state.load_rtt_estimate();
                                (current_bw / avg_bw).min(2.0).max(0.5)
                            } else {
                                1.0
                            }
                        };
                        current_cwnd + base_increase * rtt_factor * bandwidth_factor
                    };

                    let clamped_cwnd = new_cwnd.min(CWND_MAX).max(CWND_MIN);
                    
                    match state.cas_cwnd(current_cwnd, clamped_cwnd) {
                        Ok(_) => break,
                        Err(_) => continue, // Retry on contention
                    }
                }
            }
            NetworkEvent::Loss { timestamp: _ } => {
                state.losses.fetch_add(1, Ordering::Relaxed);
                
                // Lock-free loss handling
                loop {
                    let current_cwnd = state.load_cwnd();
                    let new_ssthresh = (current_cwnd * 0.7).max(CWND_MIN); // Less aggressive than 0.5
                    let new_cwnd = new_ssthresh;
                    
                    state.store_ssthresh(new_ssthresh);
                    state.in_slow_start.store(false, Ordering::Release);
                    
                    match state.cas_cwnd(current_cwnd, new_cwnd) {
                        Ok(_) => break,
                        Err(_) => continue,
                    }
                }
            }
        }

        // Update running averages atomically
        let current_cwnd = state.load_cwnd();
        loop {
            let current_sum_bits = cwnd_sum.load(Ordering::Acquire);
            let current_sum = f64::from_bits(current_sum_bits);
            let new_sum = current_sum + current_cwnd;
            
            match cwnd_sum.compare_exchange_weak(
                current_sum_bits,
                new_sum.to_bits(),
                Ordering::AcqRel,
                Ordering::Acquire
            ) {
                Ok(_) => {
                    count.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                Err(_) => continue,
            }
        }
    }

    let final_count = count.load(Ordering::Relaxed);
    let avg_cwnd = if final_count > 0 {
        f64::from_bits(cwnd_sum.load(Ordering::Relaxed)) / final_count as f64
    } else {
        0.0
    };

    let first_ts = events.first().map(|e| e.timestamp()).unwrap_or(0.0);
    let last_ts = events.last().map(|e| e.timestamp()).unwrap_or(1.0);
    let duration = (last_ts - first_ts).max(1e-9);
    let bytes_acked = state.bytes_acked.load(Ordering::Relaxed);
    let throughput = bytes_acked as f64 / duration;

    let losses = state.losses.load(Ordering::Relaxed);
    let total_sent = bytes_acked + losses;
    let goodput_ratio = if total_sent > 0 {
        bytes_acked as f64 / total_sent as f64
    } else {
        0.0
    };

    SimulationResult {
        throughput,
        total_losses: losses,
        avg_cwnd,
        goodput_ratio,
    }
}