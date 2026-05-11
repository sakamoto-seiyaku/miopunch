package desktopbridge

import (
	"reflect"
	"testing"
)

func TestManagedDaemonArgsUsesSessionMode(t *testing.T) {
	want := []string{"up", "--session"}
	if got := managedDaemonArgs(); !reflect.DeepEqual(got, want) {
		t.Fatalf("managedDaemonArgs() = %v, want %v", got, want)
	}
}
