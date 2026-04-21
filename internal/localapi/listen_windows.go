//go:build windows

package localapi

import (
	"errors"
	"fmt"
	"net"
	"strings"

	"github.com/Microsoft/go-winio"
)

func windowsPipePath(operatorSID string) (string, error) {
	trimmed := strings.TrimSpace(operatorSID)
	if trimmed == "" {
		return "", errors.New("empty operator_sid")
	}
	return `\\.\pipe\miopunch\localapi-` + trimmed, nil
}

func listenWindowsPipe(pipePath string, operatorSID string) (net.Listener, error) {
	if strings.TrimSpace(pipePath) == "" {
		return nil, errors.New("empty pipe path")
	}
	if strings.TrimSpace(operatorSID) == "" {
		return nil, errors.New("empty operator_sid")
	}

	// Allow only LocalSystem + operator user.
	sddl := fmt.Sprintf("D:P(A;;GA;;;SY)(A;;GA;;;%s)", strings.TrimSpace(operatorSID))
	cfg := &winio.PipeConfig{
		SecurityDescriptor: sddl,
		MessageMode:        false,
		InputBufferSize:    64 * 1024,
		OutputBufferSize:   64 * 1024,
	}
	ln, err := winio.ListenPipe(pipePath, cfg)
	if err != nil {
		return nil, err
	}
	return ln, nil
}
