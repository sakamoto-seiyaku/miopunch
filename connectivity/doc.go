// Package connectivity implements the P2 "connectivity orchestration" layer.
//
// It sits on top of the P1 punching kernel (xtcp/nathole) and provides:
//   - Prepare/Gather: UDP sockets + IPv6 candidates + IPv4 port mapping helpers + (optional) STUN.
//   - Exchange: a single snapshot of direct candidates (no trickle) via control plane messages.
//   - Attempt: fixed-order connectivity attempts (IPv6 direct → IPv4 direct → IPv4 punching).
//
// This package intentionally avoids relay, trickle candidates, and full port mapping lifecycle.
package connectivity
