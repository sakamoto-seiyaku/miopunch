//go:build !windows

package shelltarget

import (
	"context"
	"time"
)

// RunConPTYSmoke reports that ConPTY diagnostics are only available on Windows.
func RunConPTYSmoke(ctx context.Context, req ConPTYSmokeRequest) ConPTYSmokeResult {
	timeout := conPTYSmokeTimeout(req.Timeout)
	return ConPTYSmokeResult{
		Application: req.Application,
		Args:        append([]string(nil), req.Args...),
		StartErr:    "ConPTY smoke is only available on Windows",
		TimeoutMS:   timeout.Milliseconds(),
		DurationMS:  time.Duration(0).Milliseconds(),
	}
}
