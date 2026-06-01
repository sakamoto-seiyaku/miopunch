package controlplane

import "strings"

// IsRPCRequest reports whether kind represents an RPC request message.
//
// POC v0 rule: RPC requests use kind suffix "_request".
func IsRPCRequest(kind string) bool {
	return strings.HasSuffix(kind, "_request")
}
