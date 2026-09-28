package buildinfo

import (
	"runtime/debug"
	"testing"
)

func TestVersionFromBuildInfo_ReleaseVersion(t *testing.T) {
	old := releaseVersion
	releaseVersion = "v0.1.0-rc.1"
	t.Cleanup(func() {
		releaseVersion = old
	})

	got := versionFromBuildInfo(nil, false)
	if got != "v0.1.0-rc.1" {
		t.Errorf("versionFromBuildInfo(nil, false) = %q, want %q", got, "v0.1.0-rc.1")
	}
}

func TestVersionFromBuildInfo_ModuleVersion(t *testing.T) {
	info := &debug.BuildInfo{
		Main: debug.Module{
			Version: "v0.2.0",
		},
	}

	got := versionFromBuildInfo(info, true)
	if got != "v0.2.0" {
		t.Errorf("versionFromBuildInfo(module version) = %q, want %q", got, "v0.2.0")
	}
}

func TestVersionFromBuildInfo_Revision(t *testing.T) {
	info := &debug.BuildInfo{
		Main: debug.Module{
			Version: "(devel)",
		},
		Settings: []debug.BuildSetting{
			{Key: "vcs.revision", Value: "0123456789abcdef"},
		},
	}

	got := versionFromBuildInfo(info, true)
	if got != "0123456789ab" {
		t.Errorf("versionFromBuildInfo(revision) = %q, want %q", got, "0123456789ab")
	}
}

func TestVersionFromBuildInfo_DirtyRevision(t *testing.T) {
	info := &debug.BuildInfo{
		Main: debug.Module{
			Version: "(devel)",
		},
		Settings: []debug.BuildSetting{
			{Key: "vcs.revision", Value: "0123456789abcdef"},
			{Key: "vcs.modified", Value: "true"},
		},
	}

	got := versionFromBuildInfo(info, true)
	if got != "0123456789ab+dirty" {
		t.Errorf("versionFromBuildInfo(dirty revision) = %q, want %q", got, "0123456789ab+dirty")
	}
}

func TestVersionFromBuildInfo_NoBuildInfo(t *testing.T) {
	got := versionFromBuildInfo(nil, false)
	if got != "dev" {
		t.Errorf("versionFromBuildInfo(nil, false) = %q, want %q", got, "dev")
	}
}
