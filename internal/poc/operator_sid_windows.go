//go:build windows

package poc

import (
	"errors"
	"strings"

	"golang.org/x/sys/windows"
)

// CurrentOperatorSID returns the Windows SID of the current user, formatted
// like "S-1-5-21-...".
func CurrentOperatorSID() (string, error) {
	token := windows.GetCurrentProcessToken()
	tu, err := token.GetTokenUser()
	if err != nil {
		return "", err
	}
	if tu == nil || tu.User.Sid == nil {
		return "", errors.New("missing token user sid")
	}
	sid := strings.TrimSpace(tu.User.Sid.String())
	if sid == "" {
		return "", errors.New("failed to stringify user sid")
	}
	return sid, nil
}
