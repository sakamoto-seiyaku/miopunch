package shelltarget

import (
	"context"
	"errors"
	"strings"

	"github.com/miopunch/miopunch/internal/poc"
)

const (
	TargetStatusReady       = "ready"
	TargetStatusUnsupported = "unsupported"
	TargetStatusUnknown     = "unknown"
)

type TargetReadiness struct {
	Target     string
	Status     string
	ReasonCode string
	Message    string
}

func classifyTargetReadiness(target string, err error, out string) TargetReadiness {
	message := strings.TrimSpace(out)
	if message == "" && err != nil {
		message = strings.TrimSpace(err.Error())
	}
	if err == nil {
		return TargetReadiness{
			Target: target,
			Status: TargetStatusReady,
		}
	}
	if errors.Is(err, ErrTmuxMissing) || looksLikeTmuxMissing(message) {
		return TargetReadiness{
			Target:     target,
			Status:     TargetStatusUnsupported,
			ReasonCode: string(poc.ReasonCodeSHTmuxMissing),
			Message:    message,
		}
	}
	if errors.Is(err, context.DeadlineExceeded) || looksLikeTimeout(message) {
		return TargetReadiness{
			Target:     target,
			Status:     TargetStatusUnknown,
			ReasonCode: string(poc.ReasonCodeTimeout),
			Message:    message,
		}
	}
	return TargetReadiness{
		Target:     target,
		Status:     TargetStatusUnknown,
		ReasonCode: string(poc.ReasonCodeUnavailable),
		Message:    message,
	}
}
