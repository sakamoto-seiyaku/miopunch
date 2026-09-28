//go:build !windows

package shelltarget

import "context"

func ListTargets(ctx context.Context) ([]string, error) {
	_ = ctx
	return []string{"local"}, nil
}
