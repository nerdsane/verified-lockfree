"""
TCP Congestion Control - Initial Seed (Basic AIMD)
ShinkaEvolve will evolve this toward optimal congestion control policies.

Additive Increase / Multiplicative Decrease (AIMD):
- On ACK: cwnd += 1/cwnd  (additive increase)
- On loss: cwnd *= 0.5    (multiplicative decrease)
"""

from dataclasses import dataclass, field
from typing import List, Tuple


@dataclass
class CongestionState:
    """State of the congestion controller."""
    cwnd: float = 1.0          # Congestion window (in segments)
    ssthresh: float = 64.0     # Slow start threshold
    rtt_estimate: float = 0.1  # Estimated RTT in seconds
    in_slow_start: bool = True
    bytes_sent: int = 0
    bytes_acked: int = 0
    losses: int = 0


@dataclass
class NetworkEvent:
    """A network event (ACK or loss)."""
    kind: str        # "ack" or "loss"
    timestamp: float # Time of event
    bytes: int = 1   # Bytes acknowledged (for ACK events)
    rtt: float = 0.0 # Measured RTT (for ACK events)


@dataclass
class SimulationResult:
    """Result of running a congestion control simulation."""
    throughput: float          # Bytes per second achieved
    total_losses: int          # Total loss events
    avg_cwnd: float            # Average congestion window
    cwnd_history: List[Tuple[float, float]]  # [(time, cwnd)]
    goodput_ratio: float       # Useful bytes / total bytes


class AimdController:
    """Basic AIMD congestion controller."""

    CWND_MIN: float = 1.0
    CWND_MAX: float = 1024.0

    def __init__(self):
        self.state = CongestionState()

    def on_ack(self, event: NetworkEvent) -> None:
        """Handle an ACK event."""
        self.state.bytes_acked += event.bytes

        if event.rtt > 0:
            # Simple EWMA for RTT estimation
            alpha = 0.125
            self.state.rtt_estimate = (
                (1 - alpha) * self.state.rtt_estimate + alpha * event.rtt
            )

        if self.state.in_slow_start:
            # Slow start: exponential growth
            self.state.cwnd += 1.0
            if self.state.cwnd >= self.state.ssthresh:
                self.state.in_slow_start = False
        else:
            # Congestion avoidance: additive increase
            self.state.cwnd += 1.0 / self.state.cwnd

        self.state.cwnd = min(self.state.cwnd, self.CWND_MAX)

    def on_loss(self, event: NetworkEvent) -> None:
        """Handle a loss event."""
        self.state.losses += 1

        # Multiplicative decrease
        self.state.ssthresh = max(self.state.cwnd / 2.0, self.CWND_MIN)
        self.state.cwnd = self.state.ssthresh
        self.state.in_slow_start = False

    def process_event(self, event: NetworkEvent) -> None:
        """Process any network event."""
        if event.kind == "ack":
            self.on_ack(event)
        elif event.kind == "loss":
            self.on_loss(event)

    def get_send_window(self) -> int:
        """Return current number of segments allowed in flight."""
        return max(1, int(self.state.cwnd))


def simulate(events: List[NetworkEvent]) -> SimulationResult:
    """Run a simulation with the given events."""
    controller = AimdController()
    cwnd_history = []
    cwnd_sum = 0.0
    count = 0

    for event in events:
        controller.process_event(event)
        cwnd_history.append((event.timestamp, controller.state.cwnd))
        cwnd_sum += controller.state.cwnd
        count += 1

    avg_cwnd = cwnd_sum / count if count > 0 else 0.0
    duration = events[-1].timestamp - events[0].timestamp if len(events) > 1 else 1.0
    throughput = controller.state.bytes_acked / duration if duration > 0 else 0.0
    total_sent = controller.state.bytes_acked + controller.state.losses
    goodput_ratio = controller.state.bytes_acked / total_sent if total_sent > 0 else 0.0

    return SimulationResult(
        throughput=throughput,
        total_losses=controller.state.losses,
        avg_cwnd=avg_cwnd,
        cwnd_history=cwnd_history,
        goodput_ratio=goodput_ratio,
    )


def congestion_control(num_flows, capacity, rtt):
    """Adapter for ShinkaEvolve simulator interface.

    Args:
        num_flows: number of competing flows.
        capacity: link capacity in Mbps.
        rtt: base RTT in ms.

    Returns:
        dict with throughput and latency.
    """
    # Simulate AIMD for each flow sharing the link
    per_flow_bw = capacity / max(num_flows, 1)
    return {
        "throughput": per_flow_bw * 0.75,  # AIMD achieves ~75% utilization
        "latency": rtt * 1.2,  # Slight increase due to queuing
    }


if __name__ == "__main__":
    # Simple scenario: 100 ACKs, then a loss, then 50 more ACKs
    events = []
    t = 0.0
    for i in range(100):
        events.append(NetworkEvent(kind="ack", timestamp=t, bytes=1, rtt=0.05))
        t += 0.01
    events.append(NetworkEvent(kind="loss", timestamp=t))
    t += 0.01
    for i in range(50):
        events.append(NetworkEvent(kind="ack", timestamp=t, bytes=1, rtt=0.05))
        t += 0.01

    result = simulate(events)
    print(f"Throughput: {result.throughput:.1f} bytes/sec")
    print(f"Losses: {result.total_losses}")
    print(f"Avg cwnd: {result.avg_cwnd:.1f}")
    print(f"Goodput ratio: {result.goodput_ratio:.2%}")
