# TCP Congestion Control: Problem Statement

## What It Does

A TCP congestion control algorithm governs how fast a sender transmits data into a network whose capacity, delay, and loss characteristics are unknown and changing. The algorithm observes feedback signals -- acknowledgments that confirm data delivery and loss events that indicate congestion -- and adjusts the sending rate (expressed as a congestion window, the number of unacknowledged segments allowed in flight) to balance two competing goals: utilizing as much of the available bandwidth as possible, and avoiding overwhelming the network to the point where queues overflow and throughput collapses.

Multiple flows sharing the same bottleneck link must converge to a fair allocation of bandwidth. A single flow on an uncongested link should ramp up to the available capacity quickly. When congestion is detected (through packet loss or delay signals), the algorithm must reduce its sending rate promptly enough to relieve the congestion, but not so aggressively that it underutilizes the link. The result is a feedback control loop that must perform well across a wide range of network conditions: from low-latency datacenter links to high-delay intercontinental paths, from lightly loaded links to heavily congested ones.

## Safety Properties

1. **Buffer bounds**: The congestion window never exceeds the configured maximum. The number of segments in flight never exceeds the congestion window.
2. **Fairness**: When multiple flows share a bottleneck, the Jain's fairness index across their steady-state throughputs is at least 0.9. No single flow monopolizes the link.
3. **No congestion collapse**: Under sustained load, total throughput across all flows does not drop below 50% of the link capacity. The algorithm does not enter a state where the network carries mostly retransmissions rather than useful data.
4. **Non-negative window**: The congestion window is always at least one segment. The algorithm never stops sending entirely unless the connection is explicitly closed.
5. **Monotonic response to loss**: A loss event results in a reduction of the congestion window. The algorithm does not increase the window in direct response to a loss signal.

## Liveness Properties

1. **Convergence**: Starting from any initial window size, the algorithm reaches a steady-state throughput within 100 round-trip times.
2. **Bandwidth discovery**: On a link with no competing traffic and no loss, the algorithm increases its sending rate until it approaches the link capacity.
3. **Recovery from loss**: After a loss event reduces the window, the algorithm eventually restores throughput to its pre-loss level (assuming the congestion clears).
4. **Fair convergence**: When a new flow joins a bottleneck link with existing flows, all flows converge to approximately equal shares within a bounded number of round-trip times.

## Performance Dimensions

- Throughput: bytes per second achieved by a single flow on links of 10 Mbps, 100 Mbps, and 1 Gbps.
- Throughput-to-delay ratio: throughput divided by average RTT, capturing efficiency per unit of delay.
- Latency under load: average and p99 RTT when the link is 50% and 90% utilized.
- Loss recovery time: number of RTTs from a loss event to restoration of pre-loss throughput.
- Fairness convergence time: RTTs until Jain's fairness index exceeds 0.95 after a new flow starts.
- Goodput ratio: fraction of transmitted bytes that are useful (not retransmissions).
- Sensitivity to base RTT: throughput variation when base RTT ranges from 10 ms to 200 ms.

## What Is NOT Specified

- The specific algorithm family (AIMD, CUBIC, BBR, Vegas, Reno, compound, learning-based).
- Whether the algorithm uses loss-based signals, delay-based signals, or both.
- The slow-start mechanism and threshold.
- The multiplicative decrease factor on loss (1/2 is classic but not required).
- Whether the algorithm uses explicit congestion notification (ECN) in addition to loss.
- The RTT estimation method (EWMA, min-RTT, windowed min).
- The mechanism for detecting loss (triple duplicate ACKs, timeout, both).
- Whether pacing is used to smooth burst transmissions.
