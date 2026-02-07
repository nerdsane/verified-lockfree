/// TCP Congestion Control - Initial Seed (Basic AIMD)
/// ShinkaEvolve will evolve this toward optimal congestion control.
///
/// Additive Increase / Multiplicative Decrease (AIMD):
/// - On ACK: cwnd += 1/cwnd  (additive increase)
/// - On loss: cwnd *= 0.5    (multiplicative decrease)

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

/// Run AIMD congestion control over a sequence of network events.
///
/// Returns throughput/latency metrics. Higher throughput with lower losses
/// indicates a better congestion control algorithm.
pub fn congestion_control(events: &[NetworkEvent]) -> SimulationResult {
    debug_assert!(!events.is_empty(), "Must have at least one event");

    let mut state = CongestionState {
        cwnd: 1.0,
        ssthresh: 64.0,
        rtt_estimate: 0.1,
        in_slow_start: true,
        bytes_acked: 0,
        losses: 0,
    };

    let mut cwnd_sum: f64 = 0.0;
    let mut count: u64 = 0;

    for event in events {
        match event {
            NetworkEvent::Ack { bytes, rtt, .. } => {
                state.bytes_acked += bytes;

                if *rtt > 0.0 {
                    let alpha = 0.125;
                    state.rtt_estimate = (1.0 - alpha) * state.rtt_estimate + alpha * rtt;
                }

                if state.in_slow_start {
                    state.cwnd += 1.0;
                    if state.cwnd >= state.ssthresh {
                        state.in_slow_start = false;
                    }
                } else {
                    state.cwnd += 1.0 / state.cwnd;
                }

                state.cwnd = state.cwnd.min(CWND_MAX);
            }
            NetworkEvent::Loss { .. } => {
                state.losses += 1;
                state.ssthresh = (state.cwnd / 2.0).max(CWND_MIN);
                state.cwnd = state.ssthresh;
                state.in_slow_start = false;
            }
        }

        cwnd_sum += state.cwnd;
        count += 1;
    }

    let avg_cwnd = if count > 0 {
        cwnd_sum / count as f64
    } else {
        0.0
    };

    let first_ts = events.first().map(|e| e.timestamp()).unwrap_or(0.0);
    let last_ts = events.last().map(|e| e.timestamp()).unwrap_or(1.0);
    let duration = (last_ts - first_ts).max(1e-9);
    let throughput = state.bytes_acked as f64 / duration;

    let total_sent = state.bytes_acked + state.losses;
    let goodput_ratio = if total_sent > 0 {
        state.bytes_acked as f64 / total_sent as f64
    } else {
        0.0
    };

    SimulationResult {
        throughput,
        total_losses: state.losses,
        avg_cwnd,
        goodput_ratio,
    }
}
