//go:build !windows

package poc

// CurrentOperatorSID returns an OS-level identifier for the current operator.
//
// On non-Windows platforms this is currently unused by POC-05 and returns an
// empty string.
func CurrentOperatorSID() (string, error) {
	return "", nil
}
