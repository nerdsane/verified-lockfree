/// Trait specification tests for TCP Congestion Control.
///
/// The evolved code must provide:
///   pub enum NetworkEvent { Ack { timestamp, bytes, rtt }, Loss { timestamp } }
///   pub struct SimulationResult { throughput, total_losses, avg_cwnd, goodput_ratio }
///   pub fn congestion_control(events: &[NetworkEvent]) -> SimulationResult

#[cfg(test)]
mod tests {
    use super::*;

    fn ack(ts: f64, bytes: u64, rtt: f64) -> NetworkEvent {
        NetworkEvent::Ack {
            timestamp: ts,
            bytes,
            rtt,
        }
    }

    fn loss(ts: f64) -> NetworkEvent {
        NetworkEvent::Loss { timestamp: ts }
    }

    #[test]
    fn test_pure_acks_increase_throughput() {
        let events: Vec<NetworkEvent> = (0..100)
            .map(|i| ack(i as f64 * 0.01, 1, 0.05))
            .collect();
        let result = congestion_control(&events);
        assert!(result.throughput > 0.0);
        assert_eq!(result.total_losses, 0);
        assert!(result.goodput_ratio > 0.99);
    }

    #[test]
    fn test_loss_reduces_cwnd() {
        // Ramp up then lose
        let mut events: Vec<NetworkEvent> = (0..50)
            .map(|i| ack(i as f64 * 0.01, 1, 0.05))
            .collect();
        events.push(loss(0.50));
        events.extend((51..100).map(|i| ack(i as f64 * 0.01, 1, 0.05)));

        let result = congestion_control(&events);
        assert_eq!(result.total_losses, 1);
        // Avg cwnd should be less than pure-ack scenario
        let pure_events: Vec<NetworkEvent> = (0..100)
            .map(|i| ack(i as f64 * 0.01, 1, 0.05))
            .collect();
        let pure_result = congestion_control(&pure_events);
        assert!(result.avg_cwnd < pure_result.avg_cwnd);
    }

    #[test]
    fn test_multiple_losses_goodput_degrades() {
        let mut events: Vec<NetworkEvent> = Vec::new();
        let mut t: f64 = 0.0;
        for _ in 0..10 {
            // 10 acks then a loss, repeated 10 times
            for _ in 0..10 {
                events.push(ack(t, 1, 0.05));
                t += 0.01;
            }
            events.push(loss(t));
            t += 0.01;
        }
        let result = congestion_control(&events);
        assert_eq!(result.total_losses, 10);
        assert!(result.goodput_ratio < 1.0);
        assert!(result.goodput_ratio > 0.5); // Not completely degraded
    }

    #[test]
    fn test_throughput_positive() {
        let events = vec![ack(0.0, 100, 0.01), ack(0.01, 100, 0.01)];
        let result = congestion_control(&events);
        assert!(result.throughput > 0.0);
    }

    #[test]
    fn test_avg_cwnd_bounded() {
        let events: Vec<NetworkEvent> = (0..1000)
            .map(|i| ack(i as f64 * 0.001, 1, 0.05))
            .collect();
        let result = congestion_control(&events);
        assert!(result.avg_cwnd >= 1.0);
        assert!(result.avg_cwnd <= 1024.0);
    }

    #[test]
    fn test_competing_flows_fairness() {
        // Simulate two flows sharing capacity: losses should reduce cwnd
        let mut events = Vec::new();
        let mut t: f64 = 0.0;
        for i in 0..200 {
            if i % 20 == 19 {
                events.push(loss(t)); // Periodic congestion
            } else {
                events.push(ack(t, 1, 0.05));
            }
            t += 0.01;
        }
        let result = congestion_control(&events);
        assert!(result.total_losses >= 5);
        assert!(result.throughput > 0.0);
    }
}
